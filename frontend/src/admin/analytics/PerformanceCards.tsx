import { SlidersHorizontalIcon } from "lucide-react";
import { useState } from "react";
import { Area, Bar, BarChart, CartesianGrid, ComposedChart, Line, ReferenceLine, XAxis, YAxis } from "recharts";

import type { UsageAnalytics } from "@/admin-api";
import { Legend, LegendItem, Meter, RangeBar, StackedBar } from "@/components/app/bars";
import { CodeChip, Tag } from "@/components/app/badges";
import { SectionCard } from "@/components/app/section-card";
import { Segmented } from "@/components/app/segmented";
import { StatCard } from "@/components/app/stat-card";
import { STATUS_META, VIZ } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { ChartContainer, ChartTooltip, ChartTooltipContent, type ChartConfig } from "@/components/ui/chart";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverDescription, PopoverHeader, PopoverTitle, PopoverTrigger } from "@/components/ui/popover";
import { MINUTE_MS, formatCount, formatDateTime, formatDuration, formatPercent, formatSeconds, formatTokens } from "@/lib/format";
import { cn } from "@/lib/utils";

import { NoData, SectionLabel, SignedDelta } from "./common";
import { bucketLabel, bucketTick, elapsedMs, modelColors, successRate } from "./metrics";

export interface LatencyTargets {
	ttftP95Ms: number;
	latencyP95Ms: number;
}

function PercentileScale({ p50, p95, max, color }: { p50: number | null; p95: number | null; max: number; color: string }) {
	return (
		<div className="flex flex-col gap-0.5">
			<RangeBar color={color} from={p50} marker={p50} max={max} to={p95} />
			<div className="relative h-3 text-[10px] leading-3 text-faint">
				{p50 !== null ? (
					<span className="absolute -translate-x-1/2" style={{ left: `${Math.min(1, p50 / max) * 100}%` }}>
						P50
					</span>
				) : null}
				{p95 !== null ? (
					<span className="absolute -translate-x-1/2" style={{ left: `${Math.min(0.97, p95 / max) * 100}%` }}>
						P95
					</span>
				) : null}
			</div>
		</div>
	);
}

/** Figma "kpi-performance": 吞吐 / 首字延迟 / 总延迟 / 输出速度. */
export function PerformanceKpis({ analytics, now }: { analytics: UsageAnalytics; now: number }) {
	const { totals, previous, series, bucketMs } = analytics;
	const minutes = elapsedMs(analytics, now) / MINUTE_MS;
	const bucketMinutes = bucketMs / MINUTE_MS;
	const peak = series.reduce<(typeof series)[number] | null>((best, bucket) => (best === null || bucket.requests > best.requests ? bucket : best), null);
	const ttftMax = Math.max(1, (totals.ttft.p95 ?? 0) * 1.15);
	const latencyMax = Math.max(1, (totals.latency.p95 ?? 0) * 1.15);
	return (
		<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
			<StatCard
				detail={peak && peak.requests ? `峰值 ${(peak.requests / bucketMinutes).toFixed(1)} RPM · ${formatTokens(peak.totalTokens / bucketMinutes)} TPM（${bucketLabel(peak.startAt, bucketMs)}）` : "—"}
				label="吞吐"
				size="sm"
				unit={`RPM · ${formatTokens(totals.totalTokens / minutes)} TPM`}
				value={(totals.requests / minutes).toFixed(2)}
			/>
			<StatCard detail={`P95 ${formatSeconds(totals.ttft.p95, 1)} · 有效样本 ${formatCount(totals.ttft.samples)}`} label="首字延迟 TTFT" size="sm" unit="s" value={totals.ttft.p50 === null ? "—" : (totals.ttft.p50 / 1000).toFixed(2)}>
				<PercentileScale color={VIZ.blue} max={ttftMax} p50={totals.ttft.p50} p95={totals.ttft.p95} />
			</StatCard>
			<StatCard detail={`P95 ${formatSeconds(totals.latency.p95, 1)} · 最长 ${formatDuration(totals.latency.max)}`} label="总延迟" size="sm" unit="s" value={totals.latency.p50 === null ? "—" : (totals.latency.p50 / 1000).toFixed(1)}>
				<PercentileScale color={VIZ.teal} max={latencyMax} p50={totals.latency.p50} p95={totals.latency.p95} />
			</StatCard>
			<StatCard
				detail={
					<span className="flex items-baseline gap-1">
						中位数 · 较上一周期
						<SignedDelta value={totals.outputSpeed !== null && previous?.outputSpeed != null ? totals.outputSpeed - previous.outputSpeed : null} />
					</span>
				}
				label="输出速度"
				size="sm"
				unit="t/s"
				value={totals.outputSpeed?.toFixed(1) ?? "—"}
			/>
		</div>
	);
}

const TREND_CONFIG = {
	band: { label: "P50–P95", color: VIZ.blue },
	p50: { label: "P50", color: VIZ.blueSoft },
	p95: { label: "P95", color: VIZ.blue },
} satisfies ChartConfig;

function LatencyPanel({
	analytics,
	now,
	kind,
	target,
}: {
	analytics: UsageAnalytics;
	now: number;
	kind: "ttft" | "latency";
	target: number;
}) {
	const color = kind === "ttft" ? VIZ.blue : VIZ.teal;
	const { bucketMs, totals, previous } = analytics;
	const span = analytics.endAt - analytics.startAt;
	const data = analytics.series
		.filter((bucket) => bucket.startAt <= now)
		.map((bucket) => {
			const p50 = kind === "ttft" ? bucket.ttftP50 : bucket.latencyP50;
			const p95 = kind === "ttft" ? bucket.ttftP95 : bucket.latencyP95;
			return { at: bucket.startAt, p50: p50 === null ? null : p50 / 1000, p95: p95 === null ? null : p95 / 1000, band: p50 !== null && p95 !== null ? [p50 / 1000, p95 / 1000] : null };
		});
	const measured = data.filter((point) => point.p95 !== null);
	const over = measured.filter((point) => (point.p95 ?? 0) * 1000 > target);
	const worst = measured.reduce<(typeof measured)[number] | null>((best, point) => (best === null || (point.p95 ?? 0) > (best.p95 ?? 0) ? point : best), null);
	const current = kind === "ttft" ? totals.ttft : totals.latency;
	const before = kind === "ttft" ? previous?.ttft : previous?.latency;
	const config = { ...TREND_CONFIG, band: { ...TREND_CONFIG.band, color }, p95: { ...TREND_CONFIG.p95, color } };
	return (
		<div className="flex min-w-0 flex-col gap-3">
			<div className="flex flex-wrap items-center justify-between gap-2">
				<span className="text-[13px] font-semibold">{kind === "ttft" ? "首字延迟 TTFT" : "总延迟"}</span>
				<Legend className="gap-x-3">
					<LegendItem color={color} label="P50" shape="line" value={formatSeconds(current.p50, kind === "ttft" ? 2 : 1)} />
					<LegendItem color={color} label="P95" shape="line" value={formatSeconds(current.p95, 1)} />
					<LegendItem color={VIZ.danger} label="目标" shape="dashed" value={`≤ ${formatDuration(target)}`} />
				</Legend>
			</div>
			<ChartContainer className="aspect-auto h-[200px] w-full" config={config}>
				<ComposedChart data={data} margin={{ top: 8, right: 8, bottom: 0, left: 0 }}>
					<CartesianGrid vertical={false} />
					<XAxis axisLine={false} dataKey="at" minTickGap={28} tickFormatter={(value: number) => bucketTick(value, bucketMs, span)} tickLine={false} tickMargin={8} />
					<YAxis axisLine={false} tickFormatter={(value: number) => `${value}s`} tickLine={false} width={36} />
					<ChartTooltip
						content={
							<ChartTooltipContent
								formatter={(value, name) =>
									name === "band" ? null : (
										<span className="flex w-full justify-between gap-3">
											<span className="text-muted-foreground">{String(name).toUpperCase()}</span>
											<span className="font-medium tabular-nums">{Number(value).toFixed(2)}s</span>
										</span>
									)
								}
								labelFormatter={(_, payload) => bucketLabel(Number(payload[0]?.payload?.at ?? 0), bucketMs)}
							/>
						}
					/>
					<Area connectNulls dataKey="band" fill="var(--color-band)" fillOpacity={0.12} isAnimationActive={false} stroke="none" type="monotone" />
					<Line connectNulls dataKey="p50" dot={false} isAnimationActive={false} stroke="var(--color-band)" strokeOpacity={0.55} strokeWidth={1.25} type="monotone" />
					<Line connectNulls dataKey="p95" dot={false} isAnimationActive={false} stroke="var(--color-band)" strokeWidth={1.75} type="monotone" />
					<ReferenceLine ifOverflow="extendDomain" stroke={VIZ.danger} strokeDasharray="4 4" y={target / 1000} />
				</ComposedChart>
			</ChartContainer>
			<div className="flex flex-wrap gap-1.5 text-[11.5px]">
				<Tag tone={over.length ? "danger" : "neutral"}>
					超标区间 {over.length} / {measured.length}
				</Tag>
				{worst ? (
					<Tag>
						最差 <span className="font-semibold text-foreground">{bucketLabel(worst.at, bucketMs)} · {(worst.p95 ?? 0).toFixed(1)}s</span>
					</Tag>
				) : null}
				{current.p95 !== null && before?.p95 != null ? (
					<Tag>
						较上一周期 P95{" "}
						<span className={cn("font-semibold", current.p95 > before.p95 ? "text-danger" : "text-success")}>
							{current.p95 > before.p95 ? "+" : "−"}
							{(Math.abs(current.p95 - before.p95) / 1000).toFixed(1)}s
						</span>
					</Tag>
				) : null}
			</div>
		</div>
	);
}

/** Popover to edit the P95 targets (kept in the browser). */
export function TargetsButton({ targets, onChange }: { targets: LatencyTargets; onChange: (targets: LatencyTargets) => void }) {
	const [ttft, setTtft] = useState(String(targets.ttftP95Ms / 1000));
	const [latency, setLatency] = useState(String(targets.latencyP95Ms / 1000));
	const valid = Number(ttft) > 0 && Number(latency) > 0;
	return (
		<Popover>
			<PopoverTrigger asChild>
				<Button size="sm" type="button" variant="outline">
					<SlidersHorizontalIcon data-icon="inline-start" />
					延迟目标
				</Button>
			</PopoverTrigger>
			<PopoverContent align="end" className="w-64">
				<PopoverHeader>
					<PopoverTitle>P95 延迟目标</PopoverTitle>
					<PopoverDescription>超出目标的区间会被标记。</PopoverDescription>
				</PopoverHeader>
				<FieldGroup className="gap-3">
					<Field>
						<FieldLabel htmlFor="target-ttft">首字（秒）</FieldLabel>
						<Input id="target-ttft" inputMode="decimal" onChange={(event) => setTtft(event.target.value)} value={ttft} />
					</Field>
					<Field>
						<FieldLabel htmlFor="target-latency">总延迟（秒）</FieldLabel>
						<Input id="target-latency" inputMode="decimal" onChange={(event) => setLatency(event.target.value)} value={latency} />
					</Field>
					<Button disabled={!valid} onClick={() => onChange({ ttftP95Ms: Number(ttft) * 1000, latencyP95Ms: Number(latency) * 1000 })} size="sm" type="button">
						应用
					</Button>
				</FieldGroup>
			</PopoverContent>
		</Popover>
	);
}

/** Figma "card · 延迟趋势". */
export function LatencyTrendCard({
	analytics,
	now,
	targets,
	onTargetsChange,
	bucketMinutes,
	onBucketChange,
}: {
	analytics: UsageAnalytics;
	now: number;
	targets: LatencyTargets;
	onTargetsChange: (targets: LatencyTargets) => void;
	bucketMinutes: string;
	onBucketChange: (value: string) => void;
}) {
	return (
		<SectionCard
			actions={
				<>
					<Segmented
						label="粒度"
						onChange={onBucketChange}
						options={[
							{ value: "60", label: "1 小时" },
							{ value: "360", label: "6 小时" },
							{ value: "1440", label: "1 天" },
						]}
						value={bucketMinutes}
					/>
					<TargetsButton onChange={onTargetsChange} targets={targets} />
				</>
			}
			description="色带为 P50–P95，虚线为 P95 目标"
			title="延迟趋势"
		>
			<div className="grid gap-6 lg:grid-cols-2">
				<LatencyPanel analytics={analytics} kind="ttft" now={now} target={targets.ttftP95Ms} />
				<LatencyPanel analytics={analytics} kind="latency" now={now} target={targets.latencyP95Ms} />
			</div>
		</SectionCard>
	);
}

const OUTCOME_CONFIG = {
	completed: { label: STATUS_META.completed.label, color: STATUS_META.completed.color },
	incomplete: { label: STATUS_META.incomplete.label, color: STATUS_META.incomplete.color },
	failed: { label: STATUS_META.failed.label, color: STATUS_META.failed.color },
	cancelled: { label: STATUS_META.cancelled.label, color: STATUS_META.cancelled.color },
	interrupted: { label: "上游中断", color: STATUS_META.interrupted.color },
} satisfies ChartConfig;

/** Unsuccessful requests are drawn downward, scaled by this factor. */
const FAILURE_SCALE = 10;

function errorChip(status: string, statusCode: number | null) {
	if (statusCode && statusCode >= 400) return <CodeChip tone={statusCode === 499 ? "neutral" : "danger"}>{statusCode}</CodeChip>;
	if (status === "interrupted") return <CodeChip tone="warning">中断</CodeChip>;
	if (status === "incomplete") return <CodeChip tone="warning">未完成</CodeChip>;
	if (status === "cancelled") return <CodeChip tone="neutral">取消</CodeChip>;
	return <CodeChip>{statusCode ?? "—"}</CodeChip>;
}

/** Figma "card · 请求结果": diverging outcomes plus error codes. */
export function OutcomesCard({ analytics, now, onOpenLogs }: { analytics: UsageAnalytics; now: number; onOpenLogs?: () => void }) {
	const { series, bucketMs, totals, errors } = analytics;
	const span = analytics.endAt - analytics.startAt;
	const data = series
		.filter((bucket) => bucket.startAt <= now)
		.map((bucket) => ({
			at: bucket.startAt,
			completed: bucket.completed,
			incomplete: bucket.incomplete,
			failed: -bucket.failed * FAILURE_SCALE,
			cancelled: -bucket.cancelled * FAILURE_SCALE,
			interrupted: -bucket.interrupted * FAILURE_SCALE,
		}));
	const nonSuccess = errors.filter((row) => row.status !== "completed");
	const errorTotal = nonSuccess.reduce((sum, row) => sum + row.requests, 0);
	const maxError = Math.max(1, ...nonSuccess.map((row) => row.requests));
	return (
		<SectionCard
			actions={<Tag tone="success">成功率 {formatPercent(successRate(totals))}</Tag>}
			description={`${bucketMs >= 86_400_000 ? "每天" : `每 ${bucketMs / 3_600_000} 小时`} · 未成功请求向下绘制（放大 ${FAILURE_SCALE} 倍）`}
			title="请求结果"
		>
			<Legend>
				<LegendItem color={STATUS_META.completed.color} label="已完成" value={formatCount(totals.completed)} />
				<LegendItem color={STATUS_META.failed.color} label="失败" value={formatCount(totals.failed)} />
				<LegendItem color={STATUS_META.cancelled.color} label="客户端取消" value={formatCount(totals.cancelled)} />
				<LegendItem color={STATUS_META.interrupted.color} label="上游中断" value={formatCount(totals.interrupted)} />
				<LegendItem color={STATUS_META.incomplete.color} label="未完成" value={formatCount(totals.incomplete)} />
			</Legend>
			<ChartContainer className="aspect-auto h-[200px] w-full" config={OUTCOME_CONFIG}>
				<BarChart data={data} margin={{ top: 8, right: 4, bottom: 0, left: 0 }} stackOffset="sign">
					<CartesianGrid vertical={false} />
					<XAxis axisLine={false} dataKey="at" minTickGap={28} tickFormatter={(value: number) => bucketTick(value, bucketMs, span)} tickLine={false} tickMargin={8} />
					<YAxis axisLine={false} tickFormatter={(value: number) => formatCount(value < 0 ? -value / FAILURE_SCALE : value)} tickLine={false} width={40} />
					<ReferenceLine stroke="var(--border-strong)" y={0} />
					<ChartTooltip
						content={
							<ChartTooltipContent
								formatter={(value, name) => (
									<span className="flex w-full justify-between gap-3">
										<span className="text-muted-foreground">{OUTCOME_CONFIG[name as keyof typeof OUTCOME_CONFIG]?.label ?? name}</span>
										<span className="font-medium tabular-nums">{formatCount(Math.abs(Number(value)) / (Number(value) < 0 ? FAILURE_SCALE : 1))}</span>
									</span>
								)}
								labelFormatter={(_, payload) => bucketLabel(Number(payload[0]?.payload?.at ?? 0), bucketMs)}
							/>
						}
						cursor={{ fill: "var(--surface-hover)" }}
					/>
					<Bar dataKey="completed" fill="var(--color-completed)" stackId="outcome" />
					<Bar dataKey="incomplete" fill="var(--color-incomplete)" radius={[2, 2, 0, 0]} stackId="outcome" />
					<Bar dataKey="failed" fill="var(--color-failed)" stackId="outcome" />
					<Bar dataKey="cancelled" fill="var(--color-cancelled)" stackId="outcome" />
					<Bar dataKey="interrupted" fill="var(--color-interrupted)" stackId="outcome" />
				</BarChart>
			</ChartContainer>
			<div className="flex flex-col">
				<div className="grid grid-cols-[minmax(0,1fr)_56px_96px_88px] gap-3 border-b pb-2 text-[11.5px] text-faint">
					<span>错误</span>
					<span className="text-right">次数</span>
					<span>占比</span>
					<span className="text-right">最近</span>
				</div>
				{nonSuccess.length === 0 ? <NoData title="没有未成功的请求" /> : null}
				{nonSuccess.slice(0, 6).map((row) => (
					<button
						className="grid grid-cols-[minmax(0,1fr)_56px_96px_88px] items-center gap-3 border-b border-border/60 py-2 text-left last:border-b-0 hover:bg-muted/40 disabled:hover:bg-transparent"
						disabled={!onOpenLogs}
						key={`${row.status}:${row.statusCode}:${row.errorCode}`}
						onClick={onOpenLogs}
						type="button"
					>
						<span className="flex min-w-0 items-center gap-2">
							{errorChip(row.status, row.statusCode)}
							<span className="truncate font-mono text-xs">{row.errorCode || STATUS_META[row.status].label}</span>
						</span>
						<span className="text-right text-[12.5px] font-semibold tabular-nums">{formatCount(row.requests)}</span>
						<Meter color={STATUS_META[row.status].color} height={4} ratio={row.requests / maxError} />
						<span className="text-right text-[11px] text-faint tabular-nums">{formatDateTime(row.lastAt)}</span>
					</button>
				))}
				{errorTotal ? <span className="pt-2 text-[11px] text-faint">未成功 {formatCount(errorTotal)} 次</span> : null}
			</div>
		</SectionCard>
	);
}

function edgeLabel(ms: number): string {
	if (ms >= 60_000) return `${ms / 60_000}m`;
	return `${ms / 1000}s`;
}

/** Figma "card · 延迟诊断": TTFT × latency density and TTFT by model. */
export function LatencyDiagnosisCard({ analytics }: { analytics: UsageAnalytics }) {
	const { latencyDensity: density, totals } = analytics;
	const max = Math.max(1, ...density.cells.flat());
	const samples = density.cells.flat().reduce((sum, value) => sum + value, 0);
	const rows = [...density.cells.keys()].reverse();
	const colors = modelColors(analytics);
	const models = analytics.models.filter((model) => model.ttft.samples > 0).slice(0, 5);
	const modelMax = Math.max(1, ...models.map((model) => model.ttft.p95 ?? 0));
	const ttftBin = totals.ttft.p95 === null ? -1 : density.ttftEdgesMs.findLastIndex((edge) => (totals.ttft.p95 ?? 0) >= edge);
	const latencyBin = totals.latency.p95 === null ? -1 : density.latencyEdgesMs.findLastIndex((edge) => (totals.latency.p95 ?? 0) >= edge);
	const ramp = (value: number) => (value === 0 ? "transparent" : `color-mix(in srgb, ${VIZ.blue} ${Math.round(12 + (Math.log1p(value) / Math.log1p(max)) * 80)}%, transparent)`);
	return (
		<SectionCard description="首字延迟 × 总延迟，对数坐标" title="延迟诊断">
			<div className="flex flex-wrap gap-1.5">
				<Tag tone="info">P95 首字 {formatSeconds(totals.ttft.p95, 1)}</Tag>
				<Tag tone="danger">P95 总延迟 {formatSeconds(totals.latency.p95, 1)}</Tag>
				<Tag>样本 {formatCount(samples)}</Tag>
				<Tag>无首字数据 {formatCount(density.missingTtft)}</Tag>
			</div>
			{samples === 0 ? (
				<NoData />
			) : (
				<div className="flex flex-col gap-1">
					<span className="text-[11px] text-faint">总延迟</span>
					{rows.map((latencyIndex) => (
						<div className="grid grid-cols-[2.5rem_1fr] items-center gap-2" key={latencyIndex}>
							<span className="text-right text-[10.5px] text-faint tabular-nums">{latencyIndex % 2 === 0 ? edgeLabel(density.latencyEdgesMs[latencyIndex] ?? 0) : ""}</span>
							<div className="grid gap-[2px]" style={{ gridTemplateColumns: `repeat(${density.ttftEdgesMs.length}, minmax(0, 1fr))` }}>
								{density.cells[latencyIndex]?.map((value, ttftIndex) => (
									<span
										className={cn(
											"h-3.5 rounded-[2px]",
											value === 0 && "bg-surface-subtle/60",
											latencyIndex === latencyBin && "ring-1 ring-danger/40",
											ttftIndex === ttftBin && "outline outline-1 outline-dashed outline-[#4f7cff]/50",
										)}
										key={ttftIndex}
										style={value ? { background: ramp(value) } : undefined}
										title={`首字 ≥ ${edgeLabel(density.ttftEdgesMs[ttftIndex] ?? 0)} · 总延迟 ≥ ${edgeLabel(density.latencyEdgesMs[latencyIndex] ?? 0)}：${value}`}
									/>
								))}
							</div>
						</div>
					))}
					<div className="grid grid-cols-[2.5rem_1fr] gap-2">
						<span />
						<div className="grid" style={{ gridTemplateColumns: `repeat(${density.ttftEdgesMs.length}, minmax(0, 1fr))` }}>
							{density.ttftEdgesMs.map((edge, index) => (
								<span className="text-center text-[10.5px] text-faint tabular-nums" key={edge}>
									{index % 3 === 1 ? edgeLabel(edge) : ""}
								</span>
							))}
						</div>
					</div>
					<span className="self-end text-[11px] text-faint">首字延迟</span>
				</div>
			)}
			<div className="flex flex-col gap-2 border-t border-border/60 pt-4">
				<SectionLabel aside="P50 / P95">按模型的首字延迟</SectionLabel>
				{models.map((model) => (
					<div className="grid grid-cols-[minmax(0,9rem)_1fr_auto] items-center gap-3" key={model.model}>
						<span className="truncate font-mono text-xs">{model.model}</span>
						<RangeBar color={colors.get(model.model) ?? VIZ.gray} from={model.ttft.p50} marker={model.ttft.p50} max={modelMax} to={model.ttft.p95} />
						<span className="text-xs text-muted-foreground tabular-nums">
							{formatSeconds(model.ttft.p50, 1)} / {formatSeconds(model.ttft.p95, 1)}
						</span>
					</div>
				))}
			</div>
		</SectionCard>
	);
}

/** Figma "card · 传输与流式": WebSocket vs HTTP, stream split, compaction and clients. */
export function TransportStreamCard({ analytics }: { analytics: UsageAnalytics }) {
	const { transports, totals } = analytics;
	const ws = transports.transports.find((row) => row.name === "websocket");
	const http = transports.transports.find((row) => row.name === "http");
	const total = totals.requests || 1;
	const unsuccessful = (row: typeof ws) => (row && row.requests ? (row.failed + row.interrupted + row.incomplete) / row.requests : null);
	const rows: { label: string; ws: number | null; http: number | null; format: (value: number | null) => string; lowerIsBetter: true }[] = [
		{ label: "首字 P50", ws: ws?.ttft.p50 ?? null, http: http?.ttft.p50 ?? null, format: (value) => formatSeconds(value), lowerIsBetter: true },
		{ label: "首字 P95", ws: ws?.ttft.p95 ?? null, http: http?.ttft.p95 ?? null, format: (value) => formatSeconds(value, 1), lowerIsBetter: true },
		{ label: "总延迟 P95", ws: ws?.latency.p95 ?? null, http: http?.latency.p95 ?? null, format: (value) => formatSeconds(value, 1), lowerIsBetter: true },
		{ label: "未成功率", ws: unsuccessful(ws), http: unsuccessful(http), format: (value) => formatPercent(value), lowerIsBetter: true },
		{ label: "流中断", ws: ws?.interrupted ?? null, http: http?.interrupted ?? null, format: (value) => formatCount(value), lowerIsBetter: true },
	];
	const stream = transports.streaming.find((row) => row.name === "stream");
	const nonStream = transports.streaming.find((row) => row.name === "non_stream");
	const compaction = transports.endpoints.filter((row) => row.name.includes("compact"));
	const compactRequests = compaction.reduce((sum, row) => sum + row.requests, 0);
	const compact = compaction[0];
	const clients = analytics.clients.clients.slice(0, 4);
	return (
		<SectionCard description="延迟与未成功率对比" title="传输与流式">
			<Legend>
				<LegendItem color={VIZ.blue} label="WebSocket" value={`${formatCount(ws?.requests ?? 0)} · ${formatPercent((ws?.requests ?? 0) / total, 0)}`} />
				<LegendItem color={VIZ.blueSoft} label="HTTP" value={`${formatCount(http?.requests ?? 0)} · ${formatPercent((http?.requests ?? 0) / total, 0)}`} />
			</Legend>
			<div className="flex flex-col">
				<div className="grid grid-cols-[5.5rem_1fr_1fr] gap-3 border-b pb-2 text-[11.5px] text-faint">
					<span>指标</span>
					<span>WebSocket</span>
					<span>HTTP</span>
				</div>
				{rows.map((row) => {
					const max = Math.max(row.ws ?? 0, row.http ?? 0) || 1;
					const worse = row.ws !== null && row.http !== null && row.ws !== row.http ? (row.ws > row.http ? "ws" : "http") : null;
					return (
						<div className="grid grid-cols-[5.5rem_1fr_1fr] items-center gap-3 border-b border-border/60 py-2 last:border-b-0" key={row.label}>
							<span className="text-xs text-muted-foreground">{row.label}</span>
							{(["ws", "http"] as const).map((side) => (
								<span className="grid grid-cols-[3.25rem_1fr] items-center gap-2" key={side}>
									<span className={cn("text-xs font-semibold tabular-nums", worse === side && "text-danger")}>{row.format(row[side])}</span>
									<Meter color={side === "ws" ? VIZ.blue : VIZ.blueSoft} height={4} ratio={(row[side] ?? 0) / max} />
								</span>
							))}
						</div>
					);
				})}
			</div>
			<div className="flex flex-col gap-2 border-t border-border/60 pt-4">
				<SectionLabel aside={`流式 ${formatPercent((stream?.requests ?? 0) / total, 0)}`}>响应方式</SectionLabel>
				<StackedBar
					gap={2}
					height={10}
					segments={[
						{ key: "stream", value: stream?.requests ?? 0, color: VIZ.teal, label: "流式" },
						{ key: "non", value: nonStream?.requests ?? 0, color: VIZ.tealSoft, label: "非流式" },
					]}
				/>
				<Legend className="justify-between">
					<LegendItem color={VIZ.teal} label={`流式 ${formatCount(stream?.requests ?? 0)}`} value={`首字 P95 ${formatSeconds(stream?.ttft.p95 ?? null, 1)}`} />
					<LegendItem color={VIZ.tealSoft} label={`非流式 ${formatCount(nonStream?.requests ?? 0)}`} value={`总延迟 P95 ${formatSeconds(nonStream?.latency.p95 ?? null, 1)}`} />
				</Legend>
			</div>
			{compact ? (
				<div className="flex flex-col gap-2 border-t border-border/60 pt-4">
					<SectionLabel aside={<span className="font-mono">{compact.name}</span>}>上下文压缩</SectionLabel>
					<div className="grid grid-cols-5 gap-2">
						{[
							{ label: "请求", value: formatCount(compactRequests) },
							{ label: "P50 总延迟", value: formatSeconds(compact.latency.p50, 1) },
							{ label: "P95 总延迟", value: formatSeconds(compact.latency.p95, 1) },
							{ label: "未成功率", value: formatPercent(compact.requests ? (compact.failed + compact.interrupted + compact.incomplete) / compact.requests : null) },
							{ label: "平均输入", value: formatTokens(compact.averageInputTokens) },
						].map((item) => (
							<div className="flex min-w-0 flex-col" key={item.label}>
								<span className="text-base leading-6 font-bold tabular-nums">{item.value}</span>
								<span className="truncate text-[11px] text-faint">{item.label}</span>
							</div>
						))}
					</div>
				</div>
			) : null}
			<div className="flex flex-col gap-2 border-t border-border/60 pt-4">
				<SectionLabel aside="首字 P95 · 未成功率">按客户端</SectionLabel>
				{clients.map((client, index) => {
					const transport = client.websocketRequests === client.requests ? "WebSocket" : client.websocketRequests === 0 ? "HTTP" : "WS / HTTP";
					const failure = client.requests ? client.failed / client.requests : null;
					return (
						<div className="grid grid-cols-[minmax(0,1fr)_auto_4rem_3rem_3rem] items-center gap-2 text-xs" key={`${client.client}/${client.version}/${index}`}>
							<span className="truncate font-mono">
								{client.client || "未知"} {client.version}
							</span>
							<Tag className="rounded-md">{transport}</Tag>
							<span className="text-right text-faint tabular-nums">{formatCount(client.requests)} 请求</span>
							<span className={cn("text-right font-semibold tabular-nums", (client.ttft.p95 ?? 0) > 8_000 && "text-danger")}>{formatSeconds(client.ttft.p95, 1)}</span>
							<span className={cn("text-right tabular-nums", (failure ?? 0) > 0.05 && "font-semibold text-danger")}>{formatPercent(failure)}</span>
						</div>
					);
				})}
			</div>
		</SectionCard>
	);
}
