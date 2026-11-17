import type { AnalyticsBucket, ErrorStats, UsageAnalytics } from "@/admin-api";
import { Legend, LegendItem, Meter, StackedBar } from "@/components/app/bars";
import { CodeChip, Tag } from "@/components/app/badges";
import { Heatmap, HeatmapLegend } from "@/components/app/heatmap";
import { HEAT_BLUE, HEAT_TEAL, rampColor } from "@/components/app/heatmap-scale";
import { SectionCard } from "@/components/app/section-card";
import { STATUS_META, STATUS_ORDER, VIZ } from "@/components/app/viz";
import { cn } from "@/lib/utils";
import { DAY_MS, HOUR_MS, formatCost, formatCount, formatDateTime, formatDay, formatPercent, formatSeconds, formatTokens, formatWeekday, startOfLocalDay } from "@/lib/format";

import { NoData, SectionLabel } from "./common";
import { modelColors, successRate } from "./metrics";

/** Figma "card · 模型 Token 占比". */
export function ModelShareCard({ analytics, className = "" }: { analytics: UsageAnalytics; className?: string }) {
	const colors = modelColors(analytics);
	const total = analytics.totals.totalTokens;
	const models = analytics.models.slice(0, 6);
	return (
		<SectionCard className={className} description={`共 ${formatTokens(total)} Token · ${analytics.models.length} 个模型`} title="模型 Token 占比">
			{models.length === 0 ? <NoData /> : null}
			<div className="flex flex-col gap-4">
				{models.map((model) => {
					const share = total ? model.totalTokens / total : 0;
					return (
						<div className="flex flex-col gap-1.5" key={model.model}>
							<div className="flex items-baseline justify-between gap-3">
								<span className="truncate font-mono text-xs">{model.model}</span>
								<span className="flex shrink-0 items-baseline gap-2">
									<span className="text-[12.5px] font-semibold tabular-nums">{formatTokens(model.totalTokens)}</span>
									<span className="w-10 text-right text-[11px] text-faint tabular-nums">{formatPercent(share)}</span>
								</span>
							</div>
							<Meter color={colors.get(model.model) ?? VIZ.gray} height={4} ratio={share} />
						</div>
					);
				})}
			</div>
		</SectionCard>
	);
}

/** Day rows × 24 hours from an hourly series that starts at local midnight. */
function dayRows(series: AnalyticsBucket[], now: number, value: (bucket: AnalyticsBucket) => number) {
	const first = series[0]?.startAt ?? startOfLocalDay(now);
	const today = startOfLocalDay(now);
	return Array.from({ length: 7 }, (_, day) => {
		const dayStart = first + day * DAY_MS;
		return {
			key: String(dayStart),
			current: dayStart === today,
			label: (
				<>
					{dayStart === today ? "今天" : formatWeekday(dayStart)} {formatDay(dayStart)}
				</>
			),
			cells: Array.from({ length: 24 }, (_, hour) => {
				const at = dayStart + hour * HOUR_MS;
				if (at > now) return null;
				const bucket = series.find((entry) => entry.startAt === at);
				return bucket ? value(bucket) : 0;
			}),
		};
	});
}

/** Figma "card · Token 活动" / "card · 成本活动": 7 × 24 hourly heatmap. */
export function ActivityHeatmapCard({ series, now, metric }: { series: AnalyticsBucket[] | null; now: number; metric: "tokens" | "cost" }) {
	const value = (bucket: AnalyticsBucket) => (metric === "tokens" ? bucket.totalTokens : bucket.costUsd);
	const format = metric === "tokens" ? formatTokens : formatCost;
	const ramp = metric === "tokens" ? HEAT_BLUE : HEAT_TEAL;
	const rows = series ? dayRows(series, now, value) : [];
	const max = Math.max(0, ...(series ?? []).map(value));
	const total = (series ?? []).reduce((sum, bucket) => sum + value(bucket), 0);
	const peak = (series ?? []).reduce<AnalyticsBucket | null>((best, bucket) => (value(bucket) > (best ? value(best) : 0) ? bucket : best), null);
	return (
		<SectionCard description={metric === "cost" ? "不含未定价模型" : undefined} title={metric === "tokens" ? "Token 活动" : "成本活动"}>
			<div className="flex items-baseline gap-3">
				<span className={cn("text-2xl leading-8 font-bold tracking-[-0.03em] tabular-nums", metric === "tokens" ? "text-[#3b63d9] dark:text-[#9fb5ff]" : "text-success")}>{format(total)}</span>
				{peak ? <span className="text-xs text-muted-foreground">峰值 {formatDateTime(peak.startAt)} · {format(value(peak))}</span> : null}
			</div>
			{series ? (
				<Heatmap
					color={(cell) => rampColor(cell, max, ramp)}
					rows={rows}
					title={(cell, row, column) => `${formatDay(Number(rows[row]?.key ?? 0))} ${String(column).padStart(2, "0")}:00 · ${cell === null ? "未到" : format(cell)}`}
				/>
			) : (
				<NoData />
			)}
			<div className="flex items-center gap-3">
				<HeatmapLegend
					future
					items={[
						{ color: "var(--surface-subtle)", label: "无请求" },
						{ color: ramp[0], label: "少" },
						{ color: ramp[ramp.length - 1] ?? ramp[0], label: "多" },
					]}
				/>
				<span className="text-[11px] text-faint">每格 = 1 小时</span>
			</div>
		</SectionCard>
	);
}

function errorDescription(row: ErrorStats): string {
	if (row.statusCode === 429) return "上游额度限流";
	if (row.statusCode && row.statusCode >= 500) return "上游服务错误";
	if (row.status === "cancelled") return "客户端取消";
	if (row.status === "interrupted") return "流式连接中断";
	if (row.status === "incomplete") return "未完成：达到输出上限";
	return STATUS_META[row.status].label;
}

/** Figma "card · 结果与延迟" of the user page. */
export function HealthLatencyCard({ analytics }: { analytics: UsageAnalytics }) {
	const { totals, previous } = analytics;
	const errors = analytics.errors.filter((row) => row.status !== "completed");
	const errorTotal = errors.reduce((sum, row) => sum + row.requests, 0);
	const tiles: { label: string; value: number | null; before: number | null | undefined; format: (value: number | null) => string; unit?: string; higherIsBetter?: boolean }[] = [
		{ label: "首字 P50", value: totals.ttft.p50, before: previous?.ttft.p50, format: (value) => formatSeconds(value) },
		{ label: "首字 P95", value: totals.ttft.p95, before: previous?.ttft.p95, format: (value) => formatSeconds(value, 1) },
		{ label: "总延迟 P50", value: totals.latency.p50, before: previous?.latency.p50, format: (value) => formatSeconds(value, 1) },
		{ label: "总延迟 P95", value: totals.latency.p95, before: previous?.latency.p95, format: (value) => formatSeconds(value, 1) },
		{ label: "输出速度", value: totals.outputSpeed, before: previous?.outputSpeed, format: (value) => (value === null ? "—" : value.toFixed(1)), unit: "t/s 中位", higherIsBetter: true },
	];
	return (
		<SectionCard actions={<Tag tone="success">成功率 {formatPercent(successRate(totals))}</Tag>} title="结果与延迟">
			<div className="flex flex-col gap-2">
				<SectionLabel aside={`${formatCount(totals.requests)} 次请求`}>请求结果</SectionLabel>
				<StackedBar gap={2} height={10} segments={STATUS_ORDER.map((status) => ({ key: status, value: totals[status], color: STATUS_META[status].color, label: STATUS_META[status].label }))} />
				<Legend className="gap-x-3">
					{STATUS_ORDER.map((status) => (
						<LegendItem color={STATUS_META[status].color} key={status} label={STATUS_META[status].label} value={formatCount(totals[status])} />
					))}
				</Legend>
			</div>
			<div className="flex flex-col gap-2">
				<SectionLabel aside="对比上一周期">延迟</SectionLabel>
				<div className="grid grid-cols-2 gap-2 sm:grid-cols-5">
					{tiles.map((tile) => {
						const change = tile.value !== null && tile.before != null ? tile.value - tile.before : null;
						const good = change === null || change === 0 ? null : change > 0 === Boolean(tile.higherIsBetter);
						return (
							<div className="flex flex-col gap-0.5 rounded-lg bg-surface-subtle px-3 py-2.5" key={tile.label}>
								<span className="text-[11px] text-faint">{tile.label}</span>
								<span className="text-lg leading-6 font-bold tabular-nums">{tile.format(tile.value)}</span>
								{tile.unit ? (
									<span className="text-[11px] text-faint">{tile.unit}</span>
								) : change !== null ? (
									<span className={cn("text-[11px] font-medium tabular-nums", good ? "text-success" : "text-danger")}>
										{change > 0 ? "↑" : "↓"} {(Math.abs(change) / 1000).toFixed(change && Math.abs(change) < 1000 ? 2 : 1)}s
									</span>
								) : (
									<span className="text-[11px] text-faint">—</span>
								)}
							</div>
						);
					})}
				</div>
			</div>
			<div className="flex flex-col gap-2">
				<SectionLabel aside={`${formatCount(errorTotal)} 次 · ${formatPercent(totals.requests ? errorTotal / totals.requests : null)}`}>错误与异常</SectionLabel>
				{errors.length === 0 ? <span className="text-xs text-faint">没有未成功的请求</span> : null}
				{errors.slice(0, 6).map((row) => (
					<div className="flex items-center gap-3 border-b border-border/60 py-1.5 last:border-b-0" key={`${row.status}:${row.statusCode}:${row.errorCode}`}>
						<span className="w-12 shrink-0">
							{row.statusCode ? <CodeChip tone={row.statusCode === 499 ? "neutral" : "danger"}>{row.statusCode}</CodeChip> : <CodeChip tone="warning">—</CodeChip>}
						</span>
						<span className="flex min-w-0 flex-1 flex-col">
							<span className="truncate font-mono text-xs">{row.errorCode || row.status}</span>
							<span className="truncate text-[11px] text-faint">
								{errorDescription(row)}
								{row.accounts.length ? ` · ${row.accounts.slice(0, 2).map((account) => `${account.name} ${account.requests}`).join(" / ")}` : ""}
							</span>
						</span>
						<span className="shrink-0 text-[11px] text-faint tabular-nums">最近 {formatDateTime(row.lastAt)}</span>
						<span className="w-8 shrink-0 text-right text-[12.5px] font-semibold tabular-nums">{formatCount(row.requests)}</span>
					</div>
				))}
			</div>
		</SectionCard>
	);
}
