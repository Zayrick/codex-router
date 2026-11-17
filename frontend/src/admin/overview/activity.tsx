import { useState } from "react";
import { Bar, CartesianGrid, ComposedChart, Line, ReferenceArea, ReferenceLine, XAxis, YAxis } from "recharts";

import { useAdmin } from "@/admin/context";
import { Legend, LegendItem } from "@/components/app/bars";
import { SectionCard } from "@/components/app/section-card";
import { Segmented } from "@/components/app/segmented";
import { VIZ } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { ChartContainer, ChartTooltip, ChartTooltipContent, type ChartConfig } from "@/components/ui/chart";
import { Skeleton } from "@/components/ui/skeleton";
import { formatClock, formatCost, formatCount, formatSeconds, formatTokens, HOUR_MS } from "@/lib/format";

import { currentBucket, failedCount, type OverviewData } from "./data";

type Metric = "requests" | "tokens" | "cost";

const METRIC_FORMAT: Record<Metric, (value: number) => string> = {
	requests: formatCount,
	tokens: formatTokens,
	cost: formatCost,
};

/** Figma "card · 今日活动": hourly bars for today, yesterday's same hours dashed. */
export function ActivityCard({ data }: { data: OverviewData }) {
	const { now, navigate } = useAdmin();
	const [metric, setMetric] = useState<Metric>("requests");
	const { today, yesterday } = data;

	const actions = (
		<Segmented
			label="指标"
			onChange={setMetric}
			options={[
				{ value: "requests", label: "请求" },
				{ value: "tokens", label: "Token" },
				{ value: "cost", label: "成本" },
			]}
			value={metric}
		/>
	);

	if (!today) {
		return (
			<SectionCard actions={actions} description="按小时 · 失败含 429 与中断" title="今日活动">
				<Skeleton className="h-64 w-full" />
			</SectionCard>
		);
	}

	const current = currentBucket(today, now);
	const format = METRIC_FORMAT[metric];
	const rows = today.series.map((bucket, index) => {
		const previous = yesterday?.series[index];
		const past = index <= current;
		const failed = failedCount(bucket);
		const value = metric === "requests" ? bucket.requests - failed : metric === "tokens" ? bucket.totalTokens : bucket.costUsd;
		const compare = previous ? (metric === "requests" ? previous.requests : metric === "tokens" ? previous.totalTokens : previous.costUsd) : null;
		return {
			hour: index,
			label: `${String(index).padStart(2, "0")}:00`,
			success: past ? value : null,
			failed: past && metric === "requests" ? failed : null,
			compare,
		};
	});
	let peak = rows[0];
	for (const row of rows) if ((row.success ?? 0) + (row.failed ?? 0) > (peak?.success ?? 0) + (peak?.failed ?? 0)) peak = row;

	const totals = today.totals;
	const elapsedHours = Math.max(1 / 60, (now - today.startAt) / HOUR_MS);
	const yesterdaySameTime = today.previous;
	const projected = totals.requests > 0 ? Math.round((totals.requests / elapsedHours) * 24) : 0;
	const config = {
		success: { label: metric === "requests" ? "成功" : metric === "tokens" ? "Token" : "成本", color: VIZ.blue },
		failed: { label: "失败", color: VIZ.danger },
		compare: { label: "昨日", color: VIZ.muted },
	} satisfies ChartConfig;
	const metricTotal = metric === "requests" ? totals.requests - failedCount(totals) : metric === "tokens" ? totals.totalTokens : totals.costUsd;
	const previousTotal = yesterdaySameTime ? (metric === "requests" ? yesterdaySameTime.requests : metric === "tokens" ? yesterdaySameTime.totalTokens : yesterdaySameTime.costUsd) : null;

	return (
		<SectionCard actions={actions} description="按小时 · 失败含 429 与中断" title="今日活动">
			<Legend>
				<LegendItem color={VIZ.blue} label={config.success.label} value={format(metricTotal)} />
				{metric === "requests" ? <LegendItem color={VIZ.danger} label="失败" value={formatCount(failedCount(totals))} /> : null}
				<LegendItem color={VIZ.muted} label="昨日同期" shape="dashed" value={previousTotal === null ? "—" : format(previousTotal)} />
			</Legend>
			<ChartContainer className="aspect-auto h-56 w-full" config={config}>
				<ComposedChart data={rows} margin={{ top: 16, right: 8, bottom: 0, left: 0 }}>
					<CartesianGrid stroke="var(--border)" strokeOpacity={0.7} vertical={false} />
					<XAxis allowDecimals={false} axisLine={false} dataKey="hour" domain={[-0.5, rows.length - 0.5]} tickFormatter={(hour: number) => `${String(hour).padStart(2, "0")}:00`} tickLine={false} tickMargin={6} ticks={[0, 3, 6, 9, 12, 15, 18, 21]} type="number" />
					<YAxis axisLine={false} tickFormatter={(value: number) => format(value)} tickLine={false} width={44} />
					{current + 1 < rows.length ? (
						<ReferenceArea fill="var(--surface-subtle)" fillOpacity={1} x1={Math.max(0, current) + 0.5} x2={rows.length - 0.5} />
					) : null}
					<ChartTooltip content={<ChartTooltipContent formatter={(value, name) => `${config[name as keyof typeof config]?.label ?? name} ${format(Number(value))}`} labelFormatter={(_, payload) => payload[0]?.payload?.label} />} cursor={{ fill: "var(--surface-hover)" }} />
					<Bar dataKey="success" fill="var(--color-success)" isAnimationActive={false} maxBarSize={18} stackId="hour" />
					<Bar dataKey="failed" fill="var(--color-failed)" isAnimationActive={false} maxBarSize={18} radius={[3, 3, 0, 0]} stackId="hour" />
					<Line dataKey="compare" dot={false} isAnimationActive={false} stroke="var(--color-compare)" strokeDasharray="4 3" strokeWidth={1.25} type="monotone" />
					{current >= 0 && current < rows.length ? (
						<ReferenceLine label={{ value: `现在 ${formatClock(now, false)}`, position: "insideTopRight", fontSize: 10.5, fill: "var(--foreground)" }} stroke="var(--foreground)" strokeDasharray="3 3" x={current + (now - (today.startAt + current * today.bucketMs)) / today.bucketMs - 0.5} />
					) : null}
				</ComposedChart>
			</ChartContainer>
			<div className="flex flex-wrap items-center gap-2">
				<FootChip label="预计全天" value={`≈ ${formatCount(projected)} 请求`} />
				<FootChip label="失败" tone="danger" value={formatCount(failedCount(totals))} />
				<FootChip label="429" tone="danger" value={formatCount(totals.rateLimited)} />
				<FootChip label="TTFT P50" value={formatSeconds(totals.ttft.p50)} />
				<FootChip label="延迟 P95" value={formatSeconds(totals.latency.p95, 1)} />
				{peak && (peak.success ?? 0) > 0 ? <FootChip label="峰值" value={`${peak.label} · ${format((peak.success ?? 0) + (peak.failed ?? 0))}`} /> : null}
				<Button className="ml-auto" onClick={() => navigate("performance")} size="sm" type="button" variant="link">
					性能与可靠性 →
				</Button>
			</div>
		</SectionCard>
	);
}

/** Figma "chip · …" in card footers: gray pill with label and bold value. */
export function FootChip({ label, value, tone }: { label: string; value: string; tone?: "danger" | "warning" }) {
	return (
		<span className="inline-flex items-center gap-1.5 rounded-full bg-surface-subtle px-2 py-0.75 text-[11.5px] leading-4 whitespace-nowrap">
			<span className="text-muted-foreground">{label}</span>
			<span className={tone === "danger" ? "font-semibold text-danger" : tone === "warning" ? "font-semibold text-warning" : "font-semibold text-foreground"}>{value}</span>
		</span>
	);
}
