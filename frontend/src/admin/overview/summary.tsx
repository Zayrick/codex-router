import { ActivityIcon, CheckCircle2Icon, CircleDollarSignIcon, CoinsIcon, RefreshCwIcon } from "lucide-react";
import type { ReactNode } from "react";

import type { UsageAnalytics } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { LegendItem } from "@/components/app/bars";
import { Sparkline, type SparkPoint } from "@/components/app/sparkline";
import { StatCard, Metric } from "@/components/app/stat-card";
import { VIZ } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import {
	formatAgo,
	formatClock,
	formatCost,
	formatCount,
	formatPercent,
	formatTokens,
	percentDelta,
	pointDelta,
} from "@/lib/format";
import { cn } from "@/lib/utils";

import { cacheRate, currentBucket, failedCount, successRate, type OverviewData } from "./data";

/** Figma "status-strip": service health, last hour, RPM, active sessions, quota sync. */
export function StatusStrip({ data }: { data: OverviewData }) {
	const { quota, state, now, refreshQuota } = useAdmin();
	const enabled = state.codexAccounts.filter((account) => account.enabled);
	const quotaById = new Map((quota?.accounts ?? []).map((account) => [account.accountId, account]));
	const available = enabled.filter((account) => {
		const entry = quotaById.get(account.id);
		return !entry || (!entry.quarantined && !entry.windows.some((window) => window.limitReached));
	}).length;
	const hour = data.lastHour;
	const hourFailed = hour ? failedCount(hour.totals) : 0;
	const hourRate = hour ? successRate(hour.totals) : null;
	const tone = available === 0 ? "danger" : available < enabled.length || (hourRate !== null && hourRate < 0.95) ? "warning" : "success";
	const label = tone === "danger" ? "不可用" : tone === "warning" ? "部分降级" : "运行正常";
	const lastBucket = hour?.series.at(-1);
	const rpm = lastBucket ? lastBucket.requests / 5 : null;
	const sampled = (quota?.accounts ?? []).filter((account) => account.sampledAt !== null).length;

	return (
		<Card className="flex-row flex-wrap items-center gap-x-6 gap-y-4 px-6 py-4">
			<div className="flex flex-col gap-1">
				<span className="text-[11.5px] leading-4 font-medium text-faint">服务状态</span>
				<div className="flex items-center gap-2">
					<span
						className={cn(
							"inline-flex items-center gap-2 rounded-full px-2 py-1 text-[13px] leading-5 font-bold",
							tone === "success" && "bg-success-soft text-success",
							tone === "warning" && "bg-warning-soft text-warning",
							tone === "danger" && "bg-danger-soft text-danger",
						)}
					>
						<span aria-hidden="true" className="size-2 rounded-full bg-current" />
						{label}
					</span>
					<span className="text-xs text-muted-foreground tabular-nums">
						{available} / {enabled.length} 账户可用
					</span>
				</div>
			</div>
			<StripDivider />
			<Metric
				detail={
					<>
						请求 <span className={cn("ml-1 text-xl leading-7 font-bold tracking-[-0.02em] tabular-nums", hourFailed > 0 ? "text-danger" : "text-foreground")}>{formatCount(hourFailed)}</span> 失败
					</>
				}
				label="最近 1 小时"
				value={hour ? formatCount(hour.totals.requests) : "—"}
			/>
			<StripDivider />
			<Metric detail="近 5 分钟" label="当前 RPM" value={rpm === null ? "—" : rpm.toFixed(1)} />
			<StripDivider />
			<Metric detail="近 15 分钟" label="活跃会话" value={data.lastQuarter ? formatCount(data.lastQuarter.totals.sessions) : "—"} />
			<StripDivider />
			<Metric
				detail={quota?.sampledAt ? `${formatClock(quota.sampledAt, false)} · ${sampled} 个账户` : undefined}
				label="额度同步"
				value={quota?.sampledAt ? formatAgo(quota.sampledAt, now) : "—"}
			/>
			<div className="ml-auto flex items-center gap-2">
				<span className="text-xs text-faint">自动刷新 · 30s</span>
				<Button
					aria-label="刷新"
					disabled={data.loading}
					onClick={() => {
						data.reload();
						void refreshQuota();
					}}
					size="icon-sm"
					type="button"
					variant="outline"
				>
					{data.loading ? <Spinner /> : <RefreshCwIcon />}
				</Button>
			</div>
		</Card>
	);
}

function StripDivider() {
	return <Separator className="hidden h-10! sm:block" orientation="vertical" />;
}

/** Hourly values for today (null after the current hour) with yesterday's curve. */
function hourlyPoints(today: UsageAnalytics, yesterday: UsageAnalytics | null, now: number, pick: (bucket: UsageAnalytics["series"][number]) => number | null): SparkPoint[] {
	const current = currentBucket(today, now);
	return today.series.map((bucket, index) => {
		const previous = yesterday?.series[index];
		return { value: index <= current ? pick(bucket) : null, compare: previous ? pick(previous) : null };
	});
}

function KpiChart({ points, color }: { points: SparkPoint[]; color: string }) {
	return (
		<div className="flex flex-col gap-1">
			<Sparkline color={color} height={40} markLast points={points} />
			<div className="flex items-center justify-between font-mono text-[10.5px] leading-4 text-faint">
				<span>00:00</span>
				<span className="flex items-center gap-2 font-sans">
					<LegendItem className="text-[10.5px]" color={color} label="今日" shape="line" />
					<LegendItem className="text-[10.5px]" color={VIZ.muted} label="昨日" shape="dashed" />
				</span>
				<span>24:00</span>
			</div>
		</div>
	);
}

/** Figma "kpi-row": today vs the same time yesterday. */
export function KpiRow({ data }: { data: OverviewData }) {
	const { now } = useAdmin();
	const { today, yesterday, week } = data;
	if (!today) {
		return (
			<div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
				{[0, 1, 2, 3].map((index) => (
					<Skeleton className="h-49 rounded-xl" key={index} />
				))}
			</div>
		);
	}
	const totals = today.totals;
	const previous = today.previous;
	const rate = successRate(totals);
	const previousRate = previous ? successRate(previous) : null;
	const cards: { key: string; node: ReactNode }[] = [
		{
			key: "requests",
			node: (
				<StatCard
					delta={percentDelta(totals.requests, previous?.requests)}
					detail={`昨日同期 ${formatCount(previous?.requests)} · 昨日全天 ${formatCount(yesterday?.totals.requests)}`}
					icon={<ActivityIcon />}
					label="今日请求"
					value={formatCount(totals.requests)}
				>
					<KpiChart color={VIZ.blue} points={hourlyPoints(today, yesterday, now, (bucket) => bucket.requests)} />
				</StatCard>
			),
		},
		{
			key: "tokens",
			node: (
				<StatCard
					delta={percentDelta(totals.totalTokens, previous?.totalTokens)}
					detail={`昨日同期 ${formatTokens(previous?.totalTokens)} · 缓存命中 ${formatPercent(cacheRate(totals))}`}
					icon={<CoinsIcon />}
					label="今日 Token"
					value={formatTokens(totals.totalTokens)}
				>
					<KpiChart color={VIZ.blue} points={hourlyPoints(today, yesterday, now, (bucket) => bucket.totalTokens)} />
				</StatCard>
			),
		},
		{
			key: "cost",
			node: (
				<StatCard
					delta={percentDelta(totals.costUsd, previous?.costUsd, false)}
					detail={`昨日同期 ${formatCost(previous?.costUsd)} · 7 天 ${formatCost(week?.totals.costUsd)}`}
					icon={<CircleDollarSignIcon />}
					label="今日成本"
					value={formatCost(totals.costUsd)}
				>
					<KpiChart color={VIZ.teal} points={hourlyPoints(today, yesterday, now, (bucket) => bucket.costUsd)} />
				</StatCard>
			),
		},
		{
			key: "success",
			node: (
				<StatCard
					delta={pointDelta(rate, previousRate)}
					detail={`昨日同期 ${formatPercent(previousRate)} · 失败 ${formatCount(failedCount(totals))} · 429 × ${formatCount(totals.rateLimited)}`}
					icon={<CheckCircle2Icon />}
					label="成功率"
					value={formatPercent(rate)}
				>
					<KpiChart color={VIZ.success} points={hourlyPoints(today, yesterday, now, (bucket) => successRate(bucket))} />
				</StatCard>
			),
		},
	];
	return (
		<div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
			{cards.map((card) => (
				<div className="min-w-0 [&>[data-slot=card]]:h-full [&>[data-slot=card]]:px-6 [&>[data-slot=card]]:py-4" key={card.key}>
					{card.node}
				</div>
			))}
		</div>
	);
}
