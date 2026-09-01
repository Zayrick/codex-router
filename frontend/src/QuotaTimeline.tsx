import { useMemo, type CSSProperties } from "react";
import { Card } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import "./QuotaTimeline.css";

const HOUR_MS = 60 * 60 * 1_000;
const DAY_MS = 24 * HOUR_MS;
const MOBILE_DAY_HEIGHT_PX = 96;

export type QuotaTimelineKind =
	| "five_hour"
	| "weekly"
	| "monthly"
	| "primary"
	| "secondary";

export type QuotaTimelineCategory = "codex" | "code_review" | "additional";

export interface QuotaTimelineWindow {
	id: string;
	category: QuotaTimelineCategory;
	name: string;
	kind: QuotaTimelineKind;
	usedPercent?: number | null;
	remainingPercent?: number | null;
	limitWindowSeconds?: number | null;
	resetAt?: number | null;
}

interface TimelineSegment {
	start: number;
	end: number;
	left: number;
	width: number;
	state: "live" | "future";
	isReportedCycle: boolean;
}

type TimelineStyle = CSSProperties & Record<`--${string}`, string | number>;

interface QuotaTimelineProps {
	windows: QuotaTimelineWindow[];
	sampledAt: number;
	now: number;
	planType?: string | null | undefined;
	className?: string;
}

export default function QuotaTimeline({
	windows,
	sampledAt,
	now,
	planType,
	className,
}: QuotaTimelineProps) {
	const span = useMemo(
		() => quotaSpan(windows, validTimestamp(sampledAt) ?? now),
		[windows, sampledAt, now],
	);
	const days = useMemo(() => dayTicks(span.start, span.end), [span]);
	const nowLeft = percentAt(now, span.start, span.end);
	const timelineClassName = className
		? `timeline-card ${className}`
		: "timeline-card";

	return (
		<Card className={`${timelineClassName} gap-0 py-0`}>
			<ScrollArea className="timeline-scroll" scrollbars="horizontal">
				<div
					className="timeline-grid"
					style={{
						"--lane-count": windows.length,
						"--mobile-min-width": `${52 + windows.length * 60}px`,
						"--timeline-height": `${Math.max(384, days.length * MOBILE_DAY_HEIGHT_PX)}px`,
					} as TimelineStyle}
				>
					<div className="timeline-axis">
						<div className="axis-heading">
							<span>配额</span>
							{planType ? <b>{formatPlan(planType)}</b> : null}
						</div>
						<div className="axis-days">
							{days.map((day) => (
								<div
									className={day.isToday ? "axis-day today" : "axis-day"}
									key={day.at}
									style={{ "--day-size": `${day.width}%` } as TimelineStyle}
								>
									<span>{day.weekday}</span>
									<b>{day.label}</b>
								</div>
							))}
						</div>
					</div>
					{nowLeft !== null ? (
						<span
							className="timeline-now-overlay"
							aria-hidden="true"
							style={{ "--now-position": `${nowLeft}%` } as TimelineStyle}
						>
							<i />
						</span>
					) : null}

					{windows.map((window, index) => (
						<QuotaLane
							key={window.id}
							days={days}
							index={index}
							now={now}
							span={span}
							window={window}
						/>
					))}
				</div>
			</ScrollArea>
			<footer className="timeline-legend">
				<span><i className="legend-live" />当前周期</span>
				<span><i className="legend-future" />后续周期</span>
				<span className="legend-note">
					同步于 <time dateTime={new Date(sampledAt).toISOString()}>{formatSampledAt(sampledAt)}</time>
				</span>
			</footer>
		</Card>
	);
}

function QuotaLane({ window, span, now, days, index }: {
	window: QuotaTimelineWindow;
	span: { start: number; end: number };
	now: number;
	days: ReturnType<typeof dayTicks>;
	index: number;
}) {
	const segments = useMemo(
		() => projectWindow(window, span.start, span.end, now),
		[window, span, now],
	);
	const remaining = finitePercent(window.remainingPercent);
	const resetAt = validTimestamp(window.resetAt);
	const categoryLabel = window.category === "code_review" ? "代码审查" : window.name || "Codex";

	return (
		<div
			className={`quota-lane category-${window.category}`}
			style={{ "--lane-column": index + 2 } as TimelineStyle}
		>
			<div className="lane-heading">
				<div><span className="lane-dot" /><strong>{categoryLabel}</strong><em>{periodLabel(window)}</em></div>
				{remaining !== null || resetAt !== null ? (
					<span className="lane-mobile-summary">
						{remaining !== null ? <b>{Math.round(remaining)}%</b> : null}
						{resetAt !== null ? <small>{formatRefreshDate(resetAt)}</small> : null}
					</span>
				) : null}
			</div>
			<div className="lane-track">
				<div className="track-days">
					{days.map((day) => (
						<span
							className={day.isToday ? "today" : ""}
							key={day.at}
							style={{ "--day-size": `${day.width}%` } as TimelineStyle}
						/>
					))}
				</div>
				{segments.length === 0 ? <span className="lane-empty">暂无可投影的重置时间</span> : null}
				{segments.map((segment) => (
					<div
						className={`quota-segment ${segment.state}${segment.isReportedCycle && segment.width <= 12 ? " narrow-current" : ""}`}
						key={segment.start}
						style={{
							"--segment-offset": `${segment.left}%`,
							"--segment-size": `${segment.width}%`,
						} as TimelineStyle}
						title={`${formatDateTime(segment.start)} → ${formatDateTime(segment.end)}${segment.isReportedCycle && remaining !== null ? `\n剩余 ${Math.round(remaining)}%` : ""}`}
					>
						{segment.isReportedCycle && remaining !== null ? (
							<span
								className="segment-remaining"
								style={{ "--remaining": `${remaining}%` } as TimelineStyle}
							/>
						) : null}
						{segment.isReportedCycle && remaining !== null ? (
							<b className="segment-summary">{Math.round(remaining)}% · {formatRefreshDate(segment.end)}</b>
						) : null}
					</div>
				))}
			</div>
		</div>
	);
}

function quotaSpan(windows: QuotaTimelineWindow[], now: number): { start: number; end: number } {
	const starts = windows.flatMap((window) => {
		const resetAt = validTimestamp(window.resetAt);
		const period = windowPeriodMs(window);
		return resetAt !== null && period !== null
			? [resetAt - period]
			: [];
	});
	const firstCycleStart = starts.length > 0 ? Math.min(...starts) : now;
	const start = startOfLocalDay(firstCycleStart);
	const end = startOfNextLocalDay(Math.max(firstCycleStart + 7 * DAY_MS, now + 7 * DAY_MS));
	return { start, end };
}

function startOfLocalDay(value: number): number {
	const date = new Date(value);
	date.setHours(0, 0, 0, 0);
	return date.getTime();
}

function startOfNextLocalDay(value: number): number {
	const date = new Date(value);
	date.setHours(24, 0, 0, 0);
	return date.getTime();
}

function dayTicks(start: number, end: number) {
	const today = new Date();
	today.setHours(0, 0, 0, 0);
	const result: { at: number; label: string; weekday: string; isToday: boolean; width: number }[] = [];
	const cursor = new Date(start);
	while (cursor.getTime() < end) {
		const at = cursor.getTime();
		const next = new Date(cursor);
		next.setHours(24, 0, 0, 0);
		const cellEnd = Math.min(next.getTime(), end);
		result.push({
			at,
			label: new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit" }).format(cursor),
			weekday: new Intl.DateTimeFormat("zh-CN", { weekday: "short" }).format(cursor),
			isToday: at <= today.getTime() && cellEnd > today.getTime(),
			width: ((cellEnd - at) / (end - start)) * 100,
		});
		cursor.setTime(cellEnd);
	}
	return result;
}

function projectWindow(
	window: QuotaTimelineWindow,
	start: number,
	end: number,
	now: number,
): TimelineSegment[] {
	const resetAt = validTimestamp(window.resetAt);
	const period = windowPeriodMs(window);
	if (resetAt === null || period === null) return [];
	if (period < 60_000 || (end - start) / period > 1_000) return [];
	let windowEnd = resetAt;
	while (windowEnd <= start) windowEnd += period;
	const segments: TimelineSegment[] = [];
	while (windowEnd - period < end) {
		const windowStart = windowEnd - period;
		const clippedStart = Math.max(start, windowStart);
		const clippedEnd = Math.min(end, windowEnd);
		if (clippedEnd > clippedStart && windowEnd > now) {
			segments.push({
				start: windowStart,
				end: windowEnd,
				left: ((clippedStart - start) / (end - start)) * 100,
				width: ((clippedEnd - clippedStart) / (end - start)) * 100,
				state: windowStart <= now ? "live" : "future",
				isReportedCycle: Math.abs(windowEnd - resetAt) < 1_000,
			});
		}
		windowEnd += period;
	}
	return segments;
}

function validTimestamp(value: unknown): number | null {
	return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}

function finitePercent(value: number | null | undefined): number | null {
	return typeof value === "number" && Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : null;
}

function percentAt(value: number, start: number, end: number): number | null {
	return value >= start && value < end ? ((value - start) / (end - start)) * 100 : null;
}

function periodLabel(window: QuotaTimelineWindow): string {
	if (window.kind === "five_hour") return "5 小时";
	if (window.kind === "weekly") return "7 天";
	if (window.kind === "monthly") return "月度";
	if (window.limitWindowSeconds) {
		const hours = window.limitWindowSeconds / 3600;
		return hours < 24 ? `${Math.round(hours)} 小时` : `${Math.round(hours / 24)} 天`;
	}
	return window.kind === "primary" ? "主要额度" : "次要额度";
}

function windowPeriodMs(window: QuotaTimelineWindow): number | null {
	if (window.kind === "five_hour") return 5 * HOUR_MS;
	if (window.kind === "weekly") return 7 * DAY_MS;
	const seconds = finiteNumber(window.limitWindowSeconds);
	return seconds !== null && seconds > 0 ? seconds * 1_000 : null;
}

function finiteNumber(value: unknown): number | null {
	return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function formatPlan(value: string): string {
	return value.replace(/[_-]+/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function formatDateTime(value: number): string {
	return new Intl.DateTimeFormat("zh-CN", {
		month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false,
	}).format(new Date(value));
}

function formatRefreshDate(value: number): string {
	return new Intl.DateTimeFormat("zh-CN", {
		month: "numeric",
		day: "numeric",
		hour: "2-digit",
		minute: "2-digit",
		hour12: false,
	}).format(new Date(value));
}

function formatSampledAt(value: number): string {
	return new Intl.DateTimeFormat("zh-CN", {
		dateStyle: "medium",
		timeStyle: "short",
	}).format(new Date(value));
}
