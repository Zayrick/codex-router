import type { CSSProperties, ReactNode } from "react";

import type { WindowForecast } from "@/admin-api";
import { Badge } from "@/components/ui/badge";
import { DAY_MS, HOUR_MS, formatClock, formatDateTime, formatDay, formatWeekday, startOfLocalDay } from "@/lib/format";
import { cn } from "@/lib/utils";

import { formatShortAt, initial, planText, visibleWindows, windowLabel, windowTone } from "./model";

export interface TimelineAccount {
	id: string;
	title: string;
	plan?: string | null;
	windows: WindowForecast[];
}

const INFO_SOFT = "color-mix(in srgb, var(--info) 12%, transparent)";
const INFO_BORDER = "color-mix(in srgb, var(--info) 50%, transparent)";
const INFO_LIVE = "color-mix(in srgb, var(--info) 55%, transparent)";
const DANGER_SOFT = "color-mix(in srgb, var(--danger) 18%, transparent)";
const DANGER_BORDER = "color-mix(in srgb, var(--danger) 80%, transparent)";

interface Span {
	start: number;
	end: number;
}

/**
 * Figma "额度时间轴": one lane per quota window on a shared day (or hour) axis.
 * Shows the elapsed and remaining part of the current cycle, the projected
 * unavailable span (exhaustion → reset) and the following cycles dashed.
 */
export function QuotaTimeline({
	accounts,
	now,
	mode = "days",
	days = 8,
	daysBefore = 2,
	showAccountHeaders = true,
	title = "配额",
	className,
}: {
	accounts: TimelineAccount[];
	now: number;
	mode?: "days" | "hours";
	days?: number;
	daysBefore?: number;
	showAccountHeaders?: boolean;
	/** Label above the lane column. */
	title?: string;
	className?: string;
}) {
	const span = timelineSpan(now, mode, days, daysBefore);
	const ticks = axisTicks(span, mode, now);
	const labelWidth = showAccountHeaders ? 180 : 120;
	const nowPct = position(now, span);
	const style = { "--label-width": `${labelWidth}px` } as CSSProperties;
	return (
		<div className={cn("overflow-x-auto rounded-[10px] border bg-card", className)}>
			<div className="min-w-[680px]" style={style}>
				<Row className="h-[52px] border-b bg-surface-subtle/70">
					<div className="flex h-full items-end px-3 pb-2 text-[11.5px] leading-4 font-medium text-faint">{title}</div>
					<Area span={span} ticks={ticks} now={now}>
						{ticks.map((tick) => (
							<div className="absolute top-2 flex flex-col pl-2 leading-4 whitespace-nowrap" key={tick.at} style={{ left: `${position(tick.at, span)}%` }}>
								<span className="text-[10.5px] text-faint">{tick.secondary}</span>
								<span className={cn("text-[11.5px] font-semibold tabular-nums", tick.current ? "text-foreground" : "text-muted-foreground")}>{tick.label}</span>
							</div>
						))}
						<span
							className="absolute top-1 z-10 -translate-x-1/2 rounded-full bg-danger px-1.5 text-[10px] leading-4 font-semibold text-primary-foreground"
							style={{ left: `${nowPct}%` }}
						>
							现在 {formatClock(now, false)}
						</span>
					</Area>
				</Row>
				{accounts.map((account) => (
					<div key={account.id}>
						{showAccountHeaders ? (
							<Row className="h-9 border-b">
								<div className="col-span-2 flex min-w-0 items-center gap-2 px-4">
									<span className="flex size-5 shrink-0 items-center justify-center rounded-md bg-primary text-[10.5px] font-bold text-primary-foreground">
										{initial(account.title)}
									</span>
									<span className="truncate text-[12.5px] font-semibold">{account.title}</span>
									{account.plan ? (
										<Badge className="h-5 rounded-full px-2 text-[11px]" variant="secondary">
											{planText(account.plan)}
										</Badge>
									) : null}
								</div>
							</Row>
						) : null}
						{visibleWindows(account.windows).length ? (
							visibleWindows(account.windows).map((window) => (
								<Row className="h-10 border-b last:border-b-0" key={window.id}>
									<div className={cn("flex min-w-0 items-center gap-1.5", showAccountHeaders ? "pl-7" : "pl-3")}>
										<span aria-hidden="true" className="size-[7px] shrink-0 rounded-full bg-[var(--info)]" />
										<span className="truncate text-[12.5px] font-semibold">{window.name || "Codex"}</span>
										<span className="shrink-0 rounded-full bg-surface-subtle px-1.5 text-[10.5px] leading-4 text-muted-foreground">{windowLabel(window)}</span>
									</div>
									<Area span={span} ticks={ticks} now={now}>
										<Lane now={now} span={span} window={window} />
									</Area>
								</Row>
							))
						) : (
							<Row className="h-10 border-b">
								<div className={cn("text-xs text-faint", showAccountHeaders ? "pl-7" : "pl-3")}>暂无额度</div>
								<Area span={span} ticks={ticks} now={now} />
							</Row>
						)}
					</div>
				))}
			</div>
		</div>
	);
}

function Row({ children, className }: { children: ReactNode; className?: string }) {
	return <div className={cn("grid grid-cols-[var(--label-width)_minmax(0,1fr)] items-center", className)}>{children}</div>;
}

interface Tick {
	at: number;
	label: string;
	secondary: string;
	current: boolean;
}

/** Day columns, today band and the now line behind every row. */
function Area({ span, ticks, now, children }: { span: Span; ticks: Tick[]; now: number; children?: ReactNode }) {
	const current = ticks.find((tick) => tick.current);
	const next = current ? ticks[ticks.indexOf(current) + 1] : undefined;
	return (
		<div className="relative h-full border-l">
			{current ? (
				<div
					aria-hidden="true"
					className="absolute inset-y-0 bg-surface-hover/60"
					style={{ left: `${position(current.at, span)}%`, width: `${position(next?.at ?? span.end, span) - position(current.at, span)}%` }}
				/>
			) : null}
			{ticks.map((tick) => (
				<div aria-hidden="true" className="absolute inset-y-0 w-px bg-border/60" key={tick.at} style={{ left: `${position(tick.at, span)}%` }} />
			))}
			<div aria-hidden="true" className="absolute inset-y-0 z-[5] w-[1.5px] bg-danger" style={{ left: `${position(now, span)}%` }} />
			{children}
		</div>
	);
}

function Lane({ window, span, now }: { window: WindowForecast; span: Span; now: number }) {
	const reset = window.resetAt;
	const length = (window.limitWindowSeconds ?? 0) * 1000;
	if (!reset || length <= 0) return null;
	const start = window.windowStartAt ?? reset - length;
	const tone = windowTone(window, now);
	const exhaust = window.exhaustsBeforeReset && window.exhaustAt !== null ? Math.max(now, Math.min(window.exhaustAt, reset)) : null;
	const liveEnd = exhaust ?? reset;
	const future: Span[] = [];
	for (let cycle = reset; cycle < span.end && future.length < 60; cycle += length) {
		future.push({ start: cycle, end: cycle + length });
	}
	const used = window.usedPercent;
	const throttledWidth = exhaust !== null ? position(reset, span) - position(exhaust, span) : 0;
	return (
		<>
			<Segment className="h-4 rounded-l-full border" span={span} style={{ background: INFO_SOFT, borderColor: INFO_BORDER }} value={{ start, end: now }} />
			<Segment className={cn("h-4", exhaust === null && "rounded-r-full")} span={span} style={{ background: INFO_LIVE }} value={{ start: now, end: liveEnd }} />
			{exhaust !== null ? (
				<>
					<Segment
						className="h-4 rounded-r-full border border-dashed"
						span={span}
						style={{ background: DANGER_SOFT, borderColor: DANGER_BORDER }}
						value={{ start: exhaust, end: reset }}
					/>
					<div className="absolute top-1/2 z-[6] h-6 w-0.5 -translate-y-1/2 bg-danger" style={{ left: `${position(exhaust, span)}%` }} />
					{throttledWidth > 9 ? (
						<span
							className="absolute top-1/2 z-[6] -translate-y-1/2 pl-1.5 text-[10.5px] leading-4 font-semibold whitespace-nowrap text-danger"
							style={{ left: `${position(exhaust, span)}%` }}
						>
							预计 {formatDateTime(exhaust)} 耗尽
						</span>
					) : null}
				</>
			) : null}
			{future.map((cycle) => (
				<Segment
					className="h-3 rounded-full border border-dashed border-border-strong"
					inset={length < DAY_MS ? 1 : 2}
					key={cycle.start}
					span={span}
					value={cycle}
				/>
			))}
			{used !== null ? (
				<span
					className={cn(
						"absolute top-1/2 z-[6] flex -translate-y-1/2 items-center text-[10px] leading-3.5 font-semibold whitespace-nowrap tabular-nums",
						tone === "danger" ? "text-danger" : "text-muted-foreground",
					)}
					style={labelPlacement(position(start, span), position(liveEnd, span))}
				>
					{Math.round(used)}% · {formatShortAt(reset, now)}
				</span>
			) : null}
		</>
	);
}

/** Room (in % of the lane) a "36% · 20:45" label needs. */
const LABEL_WIDTH = 9;

/** Inside the current window when it is wide enough, else in the free space before or after it. */
function labelPlacement(start: number, end: number): CSSProperties {
	const left = Math.max(0, start);
	if (end - left >= LABEL_WIDTH) return { left: `${left}%`, marginLeft: 4 };
	if (left >= LABEL_WIDTH) return { right: `${100 - left}%`, marginRight: 6 };
	return { left: `${Math.min(100, end)}%`, marginLeft: 6 };
}

function Segment({ value, span, className, style, inset = 0 }: { value: Span; span: Span; className?: string; style?: CSSProperties; inset?: number }) {
	const left = position(value.start, span);
	const right = position(value.end, span);
	if (right - left <= 0) return null;
	return (
		<div
			aria-hidden="true"
			className={cn("absolute top-1/2 -translate-y-1/2", className)}
			style={{ ...style, left: `calc(${left}% + ${inset}px)`, width: `max(2px, calc(${right - left}% - ${inset * 2}px))` }}
		/>
	);
}

function timelineSpan(now: number, mode: "days" | "hours", days: number, daysBefore: number): Span {
	if (mode === "hours") {
		const start = Math.floor((now - 6 * HOUR_MS) / HOUR_MS) * HOUR_MS;
		return { start, end: start + 24 * HOUR_MS };
	}
	const start = startOfLocalDay(now) - daysBefore * DAY_MS;
	return { start, end: start + days * DAY_MS };
}

function axisTicks(span: Span, mode: "days" | "hours", now: number): Tick[] {
	const ticks: Tick[] = [];
	if (mode === "hours") {
		const step = 3 * HOUR_MS;
		for (let at = Math.ceil(span.start / step) * step; at < span.end; at += step) {
			ticks.push({ at, label: formatClock(at, false), secondary: formatDay(at), current: now >= at && now < at + step });
		}
		return ticks;
	}
	for (let at = span.start; at < span.end; at = startOfLocalDay(at + DAY_MS + HOUR_MS)) {
		ticks.push({ at, label: formatDay(at), secondary: formatWeekday(at), current: startOfLocalDay(now) === at });
	}
	return ticks;
}

function position(at: number, span: Span): number {
	return Math.min(100, Math.max(0, ((at - span.start) / (span.end - span.start)) * 100));
}

export function TimelineLegend({ className, note }: { className?: string; note?: ReactNode }) {
	return (
		<div className={cn("flex flex-wrap items-center gap-x-4 gap-y-1 text-[11.5px] leading-4 text-muted-foreground", className)}>
			<LegendSwatch style={{ background: INFO_SOFT, border: `1px solid ${INFO_BORDER}` }}>本周期已过</LegendSwatch>
			<LegendSwatch style={{ background: INFO_LIVE }}>本周期剩余</LegendSwatch>
			<LegendSwatch style={{ background: DANGER_SOFT, border: `1px dashed ${DANGER_BORDER}` }}>预测耗尽 → 重置（不可用）</LegendSwatch>
			<LegendSwatch className="border border-dashed border-border-strong">后续周期</LegendSwatch>
			{note ? <span className="ml-auto text-faint">{note}</span> : null}
		</div>
	);
}

function LegendSwatch({ children, style, className }: { children: ReactNode; style?: CSSProperties; className?: string }) {
	return (
		<span className="inline-flex items-center gap-1.5">
			<span aria-hidden="true" className={cn("h-2 w-3.5 rounded-full", className)} style={style} />
			{children}
		</span>
	);
}
