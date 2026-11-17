// Metric helpers shared by the analytics cards of the admin and user pages.

import type { AnalyticsBucket, OutcomeCounts, TokenCounts, UsageAnalytics } from "@/admin-api";
import { seriesColor } from "@/components/app/viz";
import { DAY_MS, HOUR_MS, formatClock, formatDateTime, formatDay } from "@/lib/format";

/** Latency targets drawn on charts and used to flag models (frontend constants). */
export const LATENCY_TARGETS = { ttftP95Ms: 8_000, latencyP95Ms: 60_000 } as const;

export function successCount(value: OutcomeCounts): number {
	return value.completed + value.incomplete;
}

/** Share of requests that finished (completed or incomplete). */
export function successRate(value: OutcomeCounts): number | null {
	return value.requests > 0 ? successCount(value) / value.requests : null;
}

/** Failed, interrupted or incomplete: the "未成功" side of performance views. */
export function unsuccessfulCount(value: OutcomeCounts): number {
	return value.failed + value.interrupted + value.incomplete;
}

export function cacheRate(value: Pick<TokenCounts, "inputTokens" | "cachedInputTokens">): number | null {
	return value.inputTokens > 0 ? value.cachedInputTokens / value.inputTokens : null;
}

/** Elapsed length of the analytics range in ms (never in the future). */
export function elapsedMs(analytics: Pick<UsageAnalytics, "startAt" | "endAt">, now: number): number {
	return Math.max(HOUR_MS / 60, Math.min(analytics.endAt, now) - analytics.startAt);
}

export function elapsedDays(analytics: Pick<UsageAnalytics, "startAt" | "endAt">, now: number): number {
	return Math.max(1 / 24, elapsedMs(analytics, now) / DAY_MS);
}

/** Model → palette color by rank of token usage. */
export function modelColors(analytics: UsageAnalytics | null): Map<string, string> {
	const colors = new Map<string, string>();
	analytics?.models.forEach((model, index) => colors.set(model.model, seriesColor(index)));
	return colors;
}

/** Axis/tooltip label for a bucket start, adapted to the bucket size. */
export function bucketLabel(at: number, bucketMs: number): string {
	return bucketMs >= DAY_MS ? formatDay(at) : formatDateTime(at);
}

export function bucketTick(at: number, bucketMs: number, spanMs: number): string {
	if (spanMs <= 2 * DAY_MS && bucketMs < DAY_MS) return formatClock(at, false);
	return formatDay(at);
}

/** Buckets that lie in the past (future buckets of "today" stay empty). */
export function pastBuckets(series: AnalyticsBucket[], now: number): AnalyticsBucket[] {
	return series.filter((bucket) => bucket.startAt <= now);
}
