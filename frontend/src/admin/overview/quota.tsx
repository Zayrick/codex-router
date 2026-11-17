import { InfoIcon, RefreshCwIcon } from "lucide-react";

import type { WindowForecast } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { Meter, StackedBar, Legend, LegendItem } from "@/components/app/bars";
import { SectionCard } from "@/components/app/section-card";
import { VIZ, planLabel, seriesColor } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { DAY_MS, HOUR_MS, formatCost, formatCount, formatDateTime, formatPercent, formatPercentValue, formatSpan, formatTokens, formatUntil, formatClock } from "@/lib/format";
import { cn } from "@/lib/utils";

import { windowLabel, type OverviewData } from "./data";

interface QuotaRow {
	key: string;
	accountName: string;
	meta: string;
	window: WindowForecast;
}

function usedTone(used: number): { color: string; text: string } {
	if (used >= 75) return { color: VIZ.danger, text: "text-danger" };
	if (used >= 50) return { color: VIZ.amber, text: "text-warning" };
	return { color: VIZ.success, text: "text-foreground" };
}

/** Rows at risk first (by projected exhaustion), then by share used. */
function riskOrder(left: WindowForecast, right: WindowForecast): number {
	const leftRisk = left.exhaustsBeforeReset && left.exhaustAt !== null ? left.exhaustAt : Number.POSITIVE_INFINITY;
	const rightRisk = right.exhaustsBeforeReset && right.exhaustAt !== null ? right.exhaustAt : Number.POSITIVE_INFINITY;
	if (leftRisk !== rightRisk) return leftRisk - rightRisk;
	return (right.usedPercent ?? 0) - (left.usedPercent ?? 0);
}

/** Figma "card · 额度风险": every enabled account window ranked by exhaustion risk. */
export function QuotaRiskCard() {
	const { quota, state, now, navigate, refreshQuota, quotaRefreshing } = useAdmin();
	const accounts = quota?.accounts ?? [];
	const rows: QuotaRow[] = accounts
		.filter((account) => account.enabled)
		.flatMap((account) => {
			const pools = state.accountGroups.filter((group) => group.accountIds.includes(account.accountId)).map((group) => group.name);
			const meta = [planLabel(account.planType), pools.join(" · ") || "直连"].filter(Boolean).join(" · ");
			return account.windows
				.filter((window) => window.category === "codex" && window.usedPercent !== null)
				.map((window) => ({ key: `${account.accountId}:${window.id}`, accountName: account.accountName, meta, window }));
		})
		.sort((left, right) => riskOrder(left.window, right.window));
	const disabled = state.codexAccounts.filter((account) => !account.enabled);

	return (
		<SectionCard
			actions={
				<>
					<Button disabled={quotaRefreshing} onClick={() => void refreshQuota(true)} size="sm" type="button" variant="outline">
						{quotaRefreshing ? <Spinner data-icon="inline-start" /> : <RefreshCwIcon data-icon="inline-start" />}
						同步额度
					</Button>
					<Button onClick={() => navigate("codex")} size="sm" type="button" variant="ghost">
						Codex 账户 →
					</Button>
				</>
			}
			description="按耗尽风险排序 · 预测基于近 3 小时消耗"
			title="额度风险"
		>
			{!quota ? (
				<div className="flex flex-col gap-2">
					{[0, 1, 2, 3, 4].map((index) => (
						<Skeleton className="h-10" key={index} />
					))}
				</div>
			) : rows.length === 0 ? (
				<Empty className="py-8">
					<EmptyHeader>
						<EmptyTitle>暂无额度快照</EmptyTitle>
						<EmptyDescription>启用的账户完成额度同步后显示在这里。</EmptyDescription>
					</EmptyHeader>
				</Empty>
			) : (
				<Table className="text-[12.5px]">
					<TableHeader>
						<TableRow className="hover:bg-transparent">
							<TableHead className="h-8 text-[11.5px] font-medium text-faint">账户</TableHead>
							<TableHead className="h-8 text-[11.5px] font-medium text-faint">窗口</TableHead>
							<TableHead className="h-8 text-[11.5px] font-medium text-faint">已用</TableHead>
							<TableHead className="h-8 text-[11.5px] font-medium text-faint">重置</TableHead>
							<TableHead className="h-8 text-[11.5px] font-medium text-faint">预计耗尽</TableHead>
						</TableRow>
					</TableHeader>
					<TableBody>
						{rows.map((row) => {
							const used = row.window.usedPercent ?? 0;
							const tone = usedTone(used);
							return (
								<TableRow className="hover:bg-transparent" key={row.key}>
									<TableCell className="py-2">
										<div className="flex flex-col">
											<span className="font-medium">{row.accountName}</span>
											<span className="text-[11px] leading-4 text-faint">{row.meta}</span>
										</div>
									</TableCell>
									<TableCell className="py-2">{windowLabel(row.window)}</TableCell>
									<TableCell className="py-2">
										<div className="flex items-center gap-2">
											<Meter className="w-24" color={tone.color} label={`${row.accountName} ${windowLabel(row.window)}已用`} ratio={used / 100} />
											<span className={cn("font-mono font-bold", tone.text)}>{formatPercentValue(used)}</span>
										</div>
									</TableCell>
									<TableCell className="py-2">
										<div className="flex flex-col">
											<span className="font-mono text-xs">{formatDateTime(row.window.resetAt)}</span>
											<span className="text-[11px] leading-4 text-faint">{formatUntil(row.window.resetAt, now)}</span>
										</div>
									</TableCell>
									<TableCell className="py-2">
										<Exhaustion now={now} window={row.window} />
									</TableCell>
								</TableRow>
							);
						})}
					</TableBody>
				</Table>
			)}
			<p className="flex items-center gap-2 text-[11.5px] leading-4 text-muted-foreground">
				<InfoIcon className="size-3.5 shrink-0" />
				{disabled.length ? `已停用的 ${disabled.map((account) => account.name).join("、")} 未列出 · ` : ""}额度每 5 分钟同步
			</p>
		</SectionCard>
	);
}

function Exhaustion({ window, now }: { window: WindowForecast; now: number }) {
	if (window.limitReached || (window.usedPercent ?? 0) >= 100) {
		return <ExhaustChip tone="danger">已耗尽</ExhaustChip>;
	}
	if (!window.exhaustsBeforeReset || window.exhaustAt === null) {
		const idle = window.burnPercentPerHour === null || window.burnPercentPerHour < 0.01;
		return <span className="text-xs text-muted-foreground">{idle ? "空闲" : "重置前不会耗尽"}</span>;
	}
	const until = window.exhaustAt - now;
	const soon = until < DAY_MS;
	const early = window.resetAt ? window.resetAt - window.exhaustAt : null;
	return (
		<div className="flex flex-col items-start gap-0.5">
			<ExhaustChip tone={until < 2 * HOUR_MS ? "danger" : "warning"}>{soon ? `约 ${formatSpan(Math.max(0, until))}后` : formatDateTime(window.exhaustAt)}</ExhaustChip>
			<span className="text-[11px] leading-4 text-faint">
				{soon ? `${formatClock(window.exhaustAt, false)} · ` : ""}
				{early !== null ? `早于重置 ${formatSpan(early)}` : ""}
			</span>
		</div>
	);
}

function ExhaustChip({ tone, children }: { tone: "danger" | "warning"; children: string }) {
	return (
		<span className={cn("inline-flex items-center gap-1.5 rounded-full px-2 py-0.75 text-[11.5px] leading-4 font-semibold", tone === "danger" ? "bg-danger-soft text-danger" : "bg-warning-soft text-warning")}>
			<span aria-hidden="true" className="size-1.5 rounded-full bg-current" />
			{children}
		</span>
	);
}

const TOP_IDENTITIES = 5;
const OTHER_COLOR = "#d5d1c8";

/** Figma "card · 今日用户": identities by cost plus today's model token mix. */
export function IdentitiesCard({ data }: { data: OverviewData }) {
	const { navigate } = useAdmin();
	const today = data.today;
	const actions = (
		<Button onClick={() => navigate("usage")} size="sm" type="button" variant="ghost">
			用量分析 →
		</Button>
	);
	if (!today) {
		return (
			<SectionCard actions={actions} description="按成本排序 · 今日 00:00 起" title="今日用户">
				<Skeleton className="h-72" />
			</SectionCard>
		);
	}
	const identities = [...today.identities].sort((left, right) => right.costUsd - left.costUsd || right.totalTokens - left.totalTokens);
	const top = identities.slice(0, TOP_IDENTITIES);
	const rest = identities.slice(TOP_IDENTITIES);
	const totalCost = today.totals.costUsd;
	const rows = [
		...top.map((identity, index) => ({
			key: identity.userId,
			color: seriesColor(index),
			name: identity.userName,
			tokens: identity.totalTokens,
			requests: identity.requests,
			cost: identity.costUsd,
		})),
		...(rest.length
			? [
					{
						key: "rest",
						color: OTHER_COLOR,
						name: `其他 ${rest.length} 个用户`,
						tokens: rest.reduce((sum, item) => sum + item.totalTokens, 0),
						requests: rest.reduce((sum, item) => sum + item.requests, 0),
						cost: rest.reduce((sum, item) => sum + item.costUsd, 0),
					},
				]
			: []),
	];
	const modelTotal = today.models.reduce((sum, model) => sum + model.totalTokens, 0);
	const models = today.models.slice(0, 6);

	return (
		<SectionCard actions={actions} description="按成本排序 · 今日 00:00 起" title="今日用户">
			{rows.length === 0 ? (
				<Empty className="py-8">
					<EmptyHeader>
						<EmptyTitle>今日暂无请求</EmptyTitle>
					</EmptyHeader>
				</Empty>
			) : (
				<>
					<StackedBar segments={rows.map((row) => ({ key: row.key, value: totalCost > 0 ? row.cost : row.tokens, color: row.color, label: row.name }))} />
					<ul className="flex flex-col divide-y divide-border/70">
						{rows.map((row) => (
							<li className="flex items-center gap-2 py-2" key={row.key}>
								<span aria-hidden="true" className="size-2 shrink-0 rounded-[2px]" style={{ background: row.color }} />
								<div className="flex min-w-0 flex-1 flex-col">
									<span className="flex items-center gap-1.5">
										<span className="truncate text-[12.5px] leading-5 font-medium">{row.name}</span>
									</span>
									<span className="text-[11px] leading-4 text-faint tabular-nums">
										{formatTokens(row.tokens)} Token · {formatCount(row.requests)} 请求
									</span>
								</div>
								<div className="flex flex-col items-end">
									<span className="font-mono text-[12.5px] leading-5 font-bold">{formatCost(row.cost)}</span>
									<span className="text-[11px] leading-4 text-faint tabular-nums">{totalCost > 0 ? formatPercent(row.cost / totalCost) : "—"}</span>
								</div>
							</li>
						))}
					</ul>
					<div className="flex items-center justify-between text-[11.5px] text-muted-foreground">
						<span>
							合计 {identities.length} 个用户 · {formatCount(today.totals.requests)} 请求
						</span>
						<span className="font-mono text-[12.5px] font-bold text-foreground">{formatCost(totalCost)}</span>
					</div>
					<div className="flex flex-col gap-2 border-t border-border/70 pt-4">
						<div className="flex items-center justify-between">
							<span className="text-xs font-semibold">今日模型 · Token 占比</span>
							<span className="text-[11.5px] text-faint">{formatTokens(modelTotal)}</span>
						</div>
						<StackedBar segments={models.map((model, index) => ({ key: model.model, value: model.totalTokens, color: seriesColor(index), label: model.model }))} />
						<Legend className="gap-x-2">
							{models.map((model, index) => (
								<LegendItem
									className="text-[11px]"
									color={seriesColor(index)}
									key={model.model}
									label={model.model}
									value={modelTotal > 0 ? `${Math.round((model.totalTokens / modelTotal) * 100)}%` : "—"}
								/>
							))}
						</Legend>
					</div>
				</>
			)}
		</SectionCard>
	);
}
