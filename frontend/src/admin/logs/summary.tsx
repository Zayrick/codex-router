import { useState } from "react";
import { Bar, BarChart, CartesianGrid, ReferenceArea, ReferenceLine, XAxis, YAxis } from "recharts";

import type { AnalyticsBucket, AnalyticsTotals } from "@/admin-api";
import { Legend, LegendItem } from "@/components/app/bars";
import { VIZ } from "@/components/app/viz";
import { Card } from "@/components/ui/card";
import { ChartContainer, ChartTooltip, ChartTooltipContent, type ChartConfig } from "@/components/ui/chart";
import { Skeleton } from "@/components/ui/skeleton";
import { formatClock, formatCost, formatCount, formatDay, formatPercent, formatSeconds, formatTokens, ratio } from "@/lib/format";
import { cn } from "@/lib/utils";

import { bucketLabel } from "./event-utils";

const CHART_CONFIG = {
	completed: { label: "已完成", color: VIZ.blue },
	failed: { label: "失败", color: VIZ.danger },
	cancelled: { label: "取消", color: VIZ.muted },
	other: { label: "中断 / 未完成", color: VIZ.warning },
} satisfies ChartConfig;

/** Figma "log-summary": five stats and the request distribution. */
export function LogSummary({
	summary,
	series,
	bucketMs,
	rangeTotal,
	rangeLabel,
	now,
	onSelectRange,
	loading,
}: {
	summary: AnalyticsTotals | null;
	series: AnalyticsBucket[];
	bucketMs: number;
	rangeTotal: number;
	rangeLabel: string;
	now: number;
	onSelectRange: (range: { startAt: number; endAt: number }) => void;
	loading: boolean;
}) {
	if (!summary) {
		return (
			<Card className="gap-4 px-6 pt-5 pb-4">
				<div className="grid grid-cols-2 gap-4 md:grid-cols-5">
					{Array.from({ length: 5 }, (_, index) => (
						<Skeleton className="h-16" key={index} />
					))}
				</div>
				<Skeleton className="h-24" />
			</Card>
		);
	}
	const success = summary.completed + summary.incomplete;
	const settled = summary.requests - summary.cancelled;
	const successRate = ratio(success, settled);
	const cacheRate = ratio(summary.cachedInputTokens, summary.inputTokens);
	return (
		<Card className={cn("gap-4 px-6 pt-5 pb-4 transition-opacity", loading && "opacity-80")}>
			<div className="grid grid-cols-2 gap-y-4 md:grid-cols-5">
				<Stat
					detail={rangeTotal > 0 ? `${rangeLabel} ${formatCount(rangeTotal)} 条中的 ${formatPercent(summary.requests / rangeTotal)}` : null}
					label="匹配请求"
					value={formatCount(summary.requests)}
				/>
				<Stat
					detail={`失败 ${summary.failed} · 取消 ${summary.cancelled} · 中断 ${summary.interrupted} · 未完成 ${summary.incomplete}`}
					label="成功率"
					value={formatPercent(successRate)}
					valueClassName={successRate === null ? undefined : successRate >= 0.95 ? "text-success" : successRate >= 0.8 ? "text-warning" : "text-danger"}
				/>
				<Stat
					detail={`缓存命中 ${formatPercent(cacheRate)} · 输出 ${formatTokens(summary.outputTokens)}`}
					label="Token"
					value={formatTokens(summary.totalTokens)}
				/>
				<Stat
					detail={`每请求 ${formatCost(summary.requests ? summary.costUsd / summary.requests : null)}`}
					label="成本"
					value={formatCost(summary.costUsd)}
				/>
				<Stat
					detail={`P50 ${formatSeconds(summary.latency.p50, 1)} · 首字 P95 ${formatSeconds(summary.ttft.p95, 1)}`}
					label="P95 延迟"
					value={formatSeconds(summary.latency.p95, 1)}
				/>
			</div>
			<div className="flex flex-col gap-2 border-t pt-4">
				<div className="flex flex-wrap items-center gap-x-4 gap-y-1">
					<span className="text-[12.5px] leading-5 font-bold">请求分布</span>
					<Legend className="flex-1">
						<LegendItem color={VIZ.blue} label="已完成" value={formatCount(summary.completed)} />
						<LegendItem color={VIZ.danger} label="失败" value={formatCount(summary.failed)} />
						<LegendItem color={VIZ.muted} label="取消" value={formatCount(summary.cancelled)} />
						<LegendItem color={VIZ.warning} label="中断 / 未完成" value={formatCount(summary.interrupted + summary.incomplete)} />
					</Legend>
					<span className="text-[11.5px] text-faint">每 {bucketLabel(bucketMs)} · 拖选可缩小范围</span>
				</div>
				<Distribution bucketMs={bucketMs} now={now} onSelectRange={onSelectRange} series={series} />
			</div>
		</Card>
	);
}

function Stat({ label, value, detail, valueClassName }: { label: string; value: string; detail: string | null; valueClassName?: string | undefined }) {
	return (
		<div className="flex min-w-0 flex-col gap-1 pr-4 md:border-l md:pl-6 md:first:border-l-0 md:first:pl-0">
			<span className="text-xs leading-4 text-muted-foreground">{label}</span>
			<span className={cn("text-[22px] leading-7 font-bold tracking-[-0.03em] tabular-nums", valueClassName)}>{value}</span>
			{detail ? <span className="truncate text-[11.5px] leading-4 text-faint">{detail}</span> : null}
		</div>
	);
}


export function Distribution({
	series,
	bucketMs,
	now,
	onSelectRange,
}: {
	series: AnalyticsBucket[];
	bucketMs: number;
	now: number;
	onSelectRange: (range: { startAt: number; endAt: number }) => void;
}) {
	const [drag, setDrag] = useState<{ from: number; to: number } | null>(null);
	const daily = bucketMs >= 86_400_000;
	const data = series.map((bucket) => ({
		at: bucket.startAt,
		completed: bucket.completed,
		failed: bucket.failed,
		cancelled: bucket.cancelled,
		other: bucket.interrupted + bucket.incomplete,
	}));
	const first = data[0]?.at ?? 0;
	const last = data[data.length - 1]?.at ?? 0;
	const nowBucket = now >= first && now < last + bucketMs ? first + Math.floor((now - first) / bucketMs) * bucketMs : null;
	const label = (value: number) => (daily || last - first > 2 * 86_400_000 ? formatDay(value) : formatClock(value, false));

	function finish() {
		if (drag && drag.from !== drag.to) {
			const startAt = Math.min(drag.from, drag.to);
			const endAt = Math.max(drag.from, drag.to) + bucketMs;
			onSelectRange({ startAt, endAt });
		}
		setDrag(null);
	}

	return (
		<ChartContainer className="aspect-auto h-[86px] w-full select-none" config={CHART_CONFIG}>
			<BarChart
				barCategoryGap={1}
				data={data}
				margin={{ top: 4, right: 0, bottom: 0, left: 0 }}
				onMouseDown={(state) => {
					const at = Number(state?.activeLabel);
					if (Number.isFinite(at)) setDrag({ from: at, to: at });
				}}
				onMouseLeave={() => setDrag(null)}
				onMouseMove={(state) => {
					const at = Number(state?.activeLabel);
					if (drag && Number.isFinite(at)) setDrag({ ...drag, to: at });
				}}
				onMouseUp={finish}
			>
				<CartesianGrid strokeOpacity={0.6} vertical={false} />
				<XAxis axisLine={{ stroke: "var(--border-strong)" }} dataKey="at" fontSize={11} interval="preserveStartEnd" minTickGap={48} tickFormatter={label} tickLine={false} />
				<YAxis allowDecimals={false} hide />
				<ChartTooltip
					content={<ChartTooltipContent labelFormatter={(_, payload) => formatClockLabel(Number(payload?.[0]?.payload?.at), daily)} />}
					cursor={{ fill: "var(--surface-hover)" }}
				/>
				<Bar dataKey="completed" fill="var(--color-completed)" fillOpacity={0.85} isAnimationActive={false} stackId="status" />
				<Bar dataKey="other" fill="var(--color-other)" isAnimationActive={false} stackId="status" />
				<Bar dataKey="cancelled" fill="var(--color-cancelled)" isAnimationActive={false} stackId="status" />
				<Bar dataKey="failed" fill="var(--color-failed)" isAnimationActive={false} radius={[2, 2, 0, 0]} stackId="status" />
				{nowBucket !== null ? <ReferenceLine stroke="var(--text-tertiary)" strokeDasharray="3 3" x={nowBucket} /> : null}
				{drag && drag.from !== drag.to ? (
					<ReferenceArea fill="var(--primary)" fillOpacity={0.08} x1={Math.min(drag.from, drag.to)} x2={Math.max(drag.from, drag.to)} />
				) : null}
			</BarChart>
		</ChartContainer>
	);
}

function formatClockLabel(at: number, daily: boolean): string {
	if (!Number.isFinite(at)) return "";
	return daily ? formatDay(at) : `${formatDay(at)} ${formatClock(at, false)}`;
}
