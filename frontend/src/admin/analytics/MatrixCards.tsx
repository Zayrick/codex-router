import { useState } from "react";

import type { UsageAnalytics } from "@/admin-api";
import { Meter } from "@/components/app/bars";
import { SectionCard } from "@/components/app/section-card";
import { Segmented } from "@/components/app/segmented";
import { VIZ } from "@/components/app/viz";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { formatCost, formatCount, formatPercent, formatTokens } from "@/lib/format";
import { cn } from "@/lib/utils";

import { NoData } from "./common";

const TOP_ROWS = 5;
const TOP_COLUMNS = 5;

/** Cell fill on a blue (or violet) ramp; text turns white on dark cells. */
function HeatCell({ value, max, label, title, color = VIZ.blue }: { value: number; max: number; label: string; title: string; color?: string }) {
	const intensity = max > 0 ? value / max : 0;
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<div
					className={cn(
						"flex h-8 min-w-0 items-center justify-center rounded-md text-xs font-medium tabular-nums",
						value === 0 && "bg-surface-subtle text-faint",
						intensity > 0.55 ? "text-white" : "text-foreground",
					)}
					style={value > 0 ? { background: `color-mix(in srgb, ${color} ${Math.round(14 + intensity * 76)}%, transparent)` } : undefined}
				>
					{value > 0 ? label : "—"}
				</div>
			</TooltipTrigger>
			<TooltipContent>{title}</TooltipContent>
		</Tooltip>
	);
}

type Metric = "tokens" | "cost" | "requests";

/** Figma "card · 用户 × 模型". */
export function IdentityModelCard({ analytics, className = "" }: { analytics: UsageAnalytics; className?: string }) {
	const [metric, setMetric] = useState<Metric>("tokens");
	const pick = (cell: { totalTokens: number; costUsd: number; requests: number }) =>
		metric === "tokens" ? cell.totalTokens : metric === "cost" ? cell.costUsd : cell.requests;
	const format = metric === "tokens" ? formatTokens : metric === "cost" ? formatCost : formatCount;
	const columns = analytics.models.slice(0, TOP_COLUMNS).map((model) => model.model);
	const identities = [...analytics.identities].sort((left, right) => pick(right) - pick(left));
	const rows = identities.slice(0, TOP_ROWS);
	const rest = identities.slice(TOP_ROWS);
	const lookup = new Map(analytics.identityModels.map((cell) => [`${cell.userId}:${cell.model}`, cell]));
	const valueOf = (keys: string[], model: string) =>
		keys.reduce((sum, key) => {
			const cell = lookup.get(`${key}:${model}`);
			return sum + (cell ? pick(cell) : 0);
		}, 0);
	const matrix = [
		...rows.map((row) => ({ label: row.userName || row.userId, keys: [row.userId] })),
		...(rest.length ? [{ label: `其他 ${rest.length} 个`, keys: rest.map((row) => row.userId) }] : []),
	].map((row) => ({ ...row, values: columns.map((model) => valueOf(row.keys, model)) }));
	const max = Math.max(0, ...matrix.flatMap((row) => row.values));
	const unit = metric === "tokens" ? "Token 用量" : metric === "cost" ? "成本" : "请求数";
	return (
		<SectionCard
			actions={
				<Segmented
					label="指标"
					onChange={setMetric}
					options={[
						{ value: "tokens", label: "Token" },
						{ value: "cost", label: "成本" },
						{ value: "requests", label: "请求" },
					]}
					value={metric}
				/>
			}
			className={className}
			description={`${unit}，颜色越深越多`}
			title="用户 × 模型"
		>
			{matrix.length === 0 || columns.length === 0 ? (
				<NoData />
			) : (
				<div className="overflow-x-auto">
					<div className="grid min-w-[560px] gap-1" style={{ gridTemplateColumns: `minmax(96px, 1.1fr) repeat(${columns.length}, minmax(72px, 1fr))` }}>
						<span />
						{columns.map((model) => (
							<span className="truncate text-center font-mono text-[11px] text-faint" key={model} title={model}>
								{model.replace(/^gpt-/, "")}
							</span>
						))}
						{matrix.map((row) => (
							<div className="contents" key={row.label}>
								<span className="flex items-center truncate pr-2 text-[12.5px] font-medium">{row.label}</span>
								{row.values.map((value, index) => (
									<HeatCell key={columns[index]} label={format(value)} max={max} title={`${row.label} · ${columns[index]}：${format(value)}`} value={value} />
								))}
							</div>
						))}
					</div>
				</div>
			)}
		</SectionCard>
	);
}

const EFFORT_ORDER = ["minimal", "low", "medium", "high", "xhigh"];
const TIER_COLUMNS: { tier: string; label: string }[] = [
	{ tier: "default", label: "标准" },
	{ tier: "priority", label: "Fast" },
	{ tier: "auto", label: "自动" },
];

/** Figma "card · 推理强度 × 速度档位" with Fast fulfilment. */
export function EffortTierCard({ analytics, className = "" }: { analytics: UsageAnalytics; className?: string }) {
	const cells = analytics.effortTiers;
	const total = cells.reduce((sum, cell) => sum + cell.requests, 0);
	const efforts = [...new Set(cells.map((cell) => cell.effort))].sort((left, right) => {
		const a = EFFORT_ORDER.indexOf(left);
		const b = EFFORT_ORDER.indexOf(right);
		return (a < 0 ? 99 : a) - (b < 0 ? 99 : b) || left.localeCompare(right);
	});
	const share = (effort: string, tier: string) =>
		total > 0 ? cells.filter((cell) => cell.effort === effort && (tier === "default" ? cell.tier === "default" || cell.tier === "" || cell.tier === "flex" : cell.tier === tier)).reduce((sum, cell) => sum + cell.requests, 0) / total : 0;
	const max = Math.max(0, ...efforts.flatMap((effort) => TIER_COLUMNS.map((column) => share(effort, column.tier))));
	const fast = analytics.fast;
	const fulfilment = fast.requested > 0 ? fast.fulfilled / fast.requested : null;
	return (
		<SectionCard className={className} description="请求占比与 Fast 兑现" title="推理强度 × 速度档位">
			{efforts.length === 0 ? (
				<NoData />
			) : (
				<div className="grid gap-1" style={{ gridTemplateColumns: "minmax(56px, 0.8fr) repeat(3, minmax(56px, 1fr))" }}>
					<span />
					{TIER_COLUMNS.map((column) => (
						<span className="text-center text-[11px] text-faint" key={column.tier}>
							{column.label}
						</span>
					))}
					{efforts.map((effort) => (
						<div className="contents" key={effort}>
							<span className="flex items-center font-mono text-[12.5px]">{effort}</span>
							{TIER_COLUMNS.map((column) => {
								const value = share(effort, column.tier);
								return <HeatCell color={VIZ.violet} key={column.tier} label={formatPercent(value)} max={max} title={`${effort} · ${column.label}：${formatPercent(value)}`} value={value} />;
							})}
						</div>
					))}
				</div>
			)}
			<div className="mt-auto flex flex-col gap-2 border-t border-border/60 pt-4">
				<div className="flex items-baseline justify-between">
					<span className="text-[13px] font-semibold">Fast 兑现率</span>
					<span className="text-base font-bold tabular-nums">{formatPercent(fulfilment)}</span>
				</div>
				<Meter color={VIZ.success} height={6} ratio={fulfilment} />
				<span className="text-xs text-muted-foreground tabular-nums">
					请求 Fast {formatCount(fast.requested)} 次 · 实际 Fast {formatCount(fast.fulfilled)} 次 · {formatCount(fast.downgraded)} 次降为标准
				</span>
			</div>
		</SectionCard>
	);
}
