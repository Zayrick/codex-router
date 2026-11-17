import type { UsageAnalytics } from "@/admin-api";
import { Legend, LegendItem, StackedBar } from "@/components/app/bars";
import { Sparkline } from "@/components/app/sparkline";
import { StatCard } from "@/components/app/stat-card";
import { TOKEN_PARTS, VIZ, tokenSplit } from "@/components/app/viz";
import { formatCost, formatCount, formatPercent, formatTokens, percentDelta, pointDelta } from "@/lib/format";

import { cacheRate, elapsedDays, successCount, successRate } from "./metrics";

/** Figma "kpi-primary": 请求 / Token / 成本 / 缓存命中率 with period deltas. */
export function UsageKpis({ analytics, now }: { analytics: UsageAnalytics; now: number }) {
	const { totals, previous, series } = analytics;
	const days = elapsedDays(analytics, now);
	const split = tokenSplit(totals);
	const pastSeries = series.filter((bucket) => bucket.startAt <= now);
	const rate = cacheRate(totals);
	const unsuccessful = totals.requests - successCount(totals);
	return (
		<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
			<StatCard
				delta={percentDelta(totals.requests, previous?.requests)}
				detail={`成功率 ${formatPercent(successRate(totals))} · 未成功 ${formatCount(unsuccessful)}`}
				label="请求"
				value={formatCount(totals.requests)}
			>
				<Sparkline color={VIZ.blue} points={pastSeries.map((bucket) => ({ value: bucket.requests }))} />
			</StatCard>
			<StatCard
				delta={percentDelta(totals.totalTokens, previous?.totalTokens)}
				detail={`日均 ${formatTokens(totals.totalTokens / days)} · 每请求 ${formatTokens(totals.requests ? totals.totalTokens / totals.requests : 0)}`}
				label="Token"
				value={formatTokens(totals.totalTokens)}
			>
				<div className="flex flex-col gap-2">
					<StackedBar
						segments={[
							{ key: "input", value: split.input, color: TOKEN_PARTS.input.color, label: TOKEN_PARTS.input.label },
							{ key: "cached", value: split.cached, color: TOKEN_PARTS.cached.color, label: TOKEN_PARTS.cached.label },
							{ key: "output", value: split.output, color: TOKEN_PARTS.output.color, label: TOKEN_PARTS.output.label },
							{ key: "reasoning", value: split.reasoning, color: TOKEN_PARTS.reasoning.color, label: TOKEN_PARTS.reasoning.label },
						]}
					/>
					<Legend className="gap-x-3 [&>span]:text-[11px]">
						<LegendItem color={TOKEN_PARTS.input.color} label="输入" value={formatTokens(split.input)} />
						<LegendItem color={TOKEN_PARTS.cached.color} label="缓存" value={formatTokens(split.cached)} />
						<LegendItem color={TOKEN_PARTS.output.color} label="输出" value={formatTokens(split.output)} />
						<LegendItem color={TOKEN_PARTS.reasoning.color} label="推理" value={formatTokens(split.reasoning)} />
					</Legend>
				</div>
			</StatCard>
			<StatCard
				delta={percentDelta(totals.costUsd, previous?.costUsd, false)}
				detail={`日均 ${formatCost(totals.costUsd / days)} · 每请求 ${formatCost(totals.requests ? totals.costUsd / totals.requests : 0)}`}
				label="成本"
				value={formatCost(totals.costUsd)}
			>
				<Sparkline color={VIZ.teal} points={pastSeries.map((bucket) => ({ value: bucket.costUsd }))} />
			</StatCard>
			<StatCard
				delta={pointDelta(rate, previous ? cacheRate(previous) : null)}
				detail={`缓存读取 ${formatTokens(totals.cachedInputTokens)} / 输入 ${formatTokens(totals.inputTokens)}`}
				label="缓存命中率"
				value={formatPercent(rate)}
			>
				<Sparkline color={VIZ.violet} points={pastSeries.map((bucket) => ({ value: bucket.inputTokens > 0 ? bucket.cachedInputTokens / bucket.inputTokens : null }))} />
			</StatCard>
		</div>
	);
}
