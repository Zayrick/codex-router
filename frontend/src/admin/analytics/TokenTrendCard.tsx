import { useState } from "react";
import { Bar, CartesianGrid, ComposedChart, Line, ReferenceArea, XAxis, YAxis } from "recharts";

import type { UsageAnalytics } from "@/admin-api";
import { Legend, LegendItem } from "@/components/app/bars";
import { SectionCard } from "@/components/app/section-card";
import { Segmented } from "@/components/app/segmented";
import { STATUS_META, TOKEN_PARTS, VIZ, tokenSplit } from "@/components/app/viz";
import { ChartContainer, ChartTooltip, ChartTooltipContent, type ChartConfig } from "@/components/ui/chart";
import { DAY_MS, HOUR_MS, formatCost, formatCount, formatPercent, formatTokens, isWeekend } from "@/lib/format";

import { bucketLabel, bucketTick, cacheRate, successCount } from "./metrics";

type Metric = "tokens" | "cost" | "requests";

const CONFIG = {
	input: { label: TOKEN_PARTS.input.label, color: TOKEN_PARTS.input.color },
	cached: { label: TOKEN_PARTS.cached.label, color: TOKEN_PARTS.cached.color },
	output: { label: TOKEN_PARTS.output.label, color: TOKEN_PARTS.output.color },
	reasoning: { label: TOKEN_PARTS.reasoning.label, color: TOKEN_PARTS.reasoning.color },
	cacheRate: { label: TOKEN_PARTS.cacheRate.label, color: TOKEN_PARTS.cacheRate.color },
	cost: { label: "成本", color: VIZ.teal },
	success: { label: "成功", color: VIZ.blue },
	failed: { label: "未成功", color: STATUS_META.failed.color },
} satisfies ChartConfig;

function bucketDescription(bucketMs: number): string {
	if (bucketMs >= DAY_MS) return `每 ${bucketMs / DAY_MS} 天`;
	if (bucketMs >= HOUR_MS) return `每 ${bucketMs / HOUR_MS} 小时`;
	return `每 ${bucketMs / 60_000} 分钟`;
}

/** Figma "card · Token 构成趋势": stacked token parts with the cache-hit line. */
export function TokenTrendCard({ analytics, now, className = "" }: { analytics: UsageAnalytics; now: number; className?: string }) {
	const [metric, setMetric] = useState<Metric>("tokens");
	const { series, bucketMs, totals } = analytics;
	const span = analytics.endAt - analytics.startAt;
	const data = series.map((bucket) => {
		const split = tokenSplit(bucket);
		const future = bucket.startAt > now;
		return {
			at: bucket.startAt,
			...split,
			cacheRate: !future && bucket.inputTokens > 0 ? bucket.cachedInputTokens / bucket.inputTokens : null,
			cost: bucket.costUsd,
			success: successCount(bucket),
			failed: bucket.requests - successCount(bucket),
			weekend: isWeekend(bucket.startAt),
		};
	});
	const weekendSpans: { from: number; to: number }[] = [];
	data.forEach((point, index) => {
		if (!point.weekend) return;
		const last = weekendSpans.at(-1);
		if (last && data[index - 1]?.weekend) last.to = point.at;
		else weekendSpans.push({ from: point.at, to: point.at });
	});
	const split = tokenSplit(totals);
	const tickFormatter = (value: number) => bucketTick(value, bucketMs, span);
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
			description={`${bucketDescription(bucketMs)}${metric === "tokens" ? " · 折线为缓存命中率" : ""}`}
			title="Token 构成趋势"
		>
			<Legend>
				{metric === "tokens" ? (
					<>
						<LegendItem color={TOKEN_PARTS.input.color} label={TOKEN_PARTS.input.label} value={formatTokens(split.input)} />
						<LegendItem color={TOKEN_PARTS.cached.color} label={TOKEN_PARTS.cached.label} value={formatTokens(split.cached)} />
						<LegendItem color={TOKEN_PARTS.output.color} label={TOKEN_PARTS.output.label} value={formatTokens(split.output)} />
						<LegendItem color={TOKEN_PARTS.reasoning.color} label={TOKEN_PARTS.reasoning.label} value={formatTokens(split.reasoning)} />
						<LegendItem color={TOKEN_PARTS.cacheRate.color} label={TOKEN_PARTS.cacheRate.label} shape="line" value={formatPercent(cacheRate(totals))} />
					</>
				) : metric === "cost" ? (
					<LegendItem color={VIZ.teal} label="成本" value={formatCost(totals.costUsd)} />
				) : (
					<>
						<LegendItem color={VIZ.blue} label="成功" value={formatCount(successCount(totals))} />
						<LegendItem color={STATUS_META.failed.color} label="未成功" value={formatCount(totals.requests - successCount(totals))} />
					</>
				)}
			</Legend>
			<ChartContainer className="aspect-auto h-[258px] w-full" config={CONFIG}>
				<ComposedChart data={data} margin={{ top: 8, right: 4, bottom: 0, left: 0 }}>
					<CartesianGrid vertical={false} />
					{weekendSpans.map((spanRange) => (
						<ReferenceArea fill="var(--surface-subtle)" fillOpacity={1} ifOverflow="extendDomain" key={spanRange.from} x1={spanRange.from} x2={spanRange.to} yAxisId="left" />
					))}
					<XAxis axisLine={false} dataKey="at" minTickGap={24} tickFormatter={tickFormatter} tickLine={false} tickMargin={8} />
					<YAxis
						axisLine={false}
						tickFormatter={(value: number) => (metric === "cost" ? formatCost(value) : metric === "tokens" ? formatTokens(value) : formatCount(value))}
						tickLine={false}
						width={48}
						yAxisId="left"
					/>
					{metric === "tokens" ? (
						<YAxis
							axisLine={false}
							domain={[0, 1]}
							orientation="right"
							tick={{ fill: VIZ.warning }}
							tickFormatter={(value: number) => `${Math.round(value * 100)}%`}
							tickLine={false}
							width={40}
							yAxisId="right"
						/>
					) : null}
					<ChartTooltip
						content={
							<ChartTooltipContent
								formatter={(value, name) => {
									const number = Number(value);
									const text = name === "cacheRate" ? formatPercent(number) : name === "cost" ? formatCost(number) : metric === "requests" ? formatCount(number) : formatTokens(number);
									return (
										<span className="flex w-full justify-between gap-3">
											<span className="text-muted-foreground">{CONFIG[name as keyof typeof CONFIG]?.label ?? name}</span>
											<span className="font-medium tabular-nums">{text}</span>
										</span>
									);
								}}
								labelFormatter={(_, payload) => bucketLabel(Number(payload[0]?.payload?.at ?? 0), bucketMs)}
							/>
						}
						cursor={{ fill: "var(--surface-hover)" }}
					/>
					{metric === "tokens" ? (
						<>
							<Bar dataKey="input" fill="var(--color-input)" stackId="tokens" yAxisId="left" />
							<Bar dataKey="cached" fill="var(--color-cached)" stackId="tokens" yAxisId="left" />
							<Bar dataKey="output" fill="var(--color-output)" stackId="tokens" yAxisId="left" />
							<Bar dataKey="reasoning" fill="var(--color-reasoning)" radius={[3, 3, 0, 0]} stackId="tokens" yAxisId="left" />
							<Line connectNulls dataKey="cacheRate" dot={false} stroke="var(--color-cacheRate)" strokeWidth={2} type="monotone" yAxisId="right" />
						</>
					) : metric === "cost" ? (
						<Bar dataKey="cost" fill="var(--color-cost)" radius={[3, 3, 0, 0]} yAxisId="left" />
					) : (
						<>
							<Bar dataKey="success" fill="var(--color-success)" stackId="requests" yAxisId="left" />
							<Bar dataKey="failed" fill="var(--color-failed)" radius={[3, 3, 0, 0]} stackId="requests" yAxisId="left" />
						</>
					)}
				</ComposedChart>
			</ChartContainer>
		</SectionCard>
	);
}
