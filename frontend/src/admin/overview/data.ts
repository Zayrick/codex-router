import type { AnalyticsBucket, AnalyticsTotals, UsageAnalytics, WindowForecast } from "@/admin-api";
import { useAnalytics } from "@/admin/hooks";
import { DAY_MS, HOUR_MS, MINUTE_MS, startOfLocalDay } from "@/lib/format";

export const OVERVIEW_REFRESH_MS = 30_000;

export interface OverviewData {
	today: UsageAnalytics | null;
	yesterday: UsageAnalytics | null;
	week: UsageAnalytics | null;
	lastHour: UsageAnalytics | null;
	lastQuarter: UsageAnalytics | null;
	loading: boolean;
	error: string | null;
	midnight: number;
	reload: () => void;
}

/**
 * Data behind the overview: today (hourly, compared with yesterday's same
 * time), yesterday (full day), the last 7 local days, the last hour (5-min
 * buckets) and the last 15 minutes (active sessions).
 */
export function useOverviewData(now: number): OverviewData {
	const midnight = startOfLocalDay(now);
	const minute = Math.floor(now / MINUTE_MS) * MINUTE_MS;
	const today = useAnalytics({ range: "custom", startAt: midnight, endAt: midnight + DAY_MS, bucketMinutes: 60 }, { refreshMs: OVERVIEW_REFRESH_MS });
	const yesterday = useAnalytics({ range: "custom", startAt: midnight - DAY_MS, endAt: midnight, bucketMinutes: 60 });
	const week = useAnalytics(
		{ range: "custom", startAt: midnight - 6 * DAY_MS, endAt: midnight + DAY_MS, bucketMinutes: 60 },
		{ refreshMs: OVERVIEW_REFRESH_MS },
	);
	// `now` ticks every 30 s, so these rolling windows also move on their own.
	const lastHour = useAnalytics({ range: "custom", startAt: minute - HOUR_MS, endAt: minute, bucketMinutes: 5 }, { refreshMs: OVERVIEW_REFRESH_MS });
	const lastQuarter = useAnalytics({ range: "custom", startAt: minute - 15 * MINUTE_MS, endAt: minute, bucketMinutes: 15 }, { refreshMs: OVERVIEW_REFRESH_MS });
	const resources = [today, yesterday, week, lastHour, lastQuarter];
	return {
		today: today.data,
		yesterday: yesterday.data,
		week: week.data,
		lastHour: lastHour.data,
		lastQuarter: lastQuarter.data,
		loading: resources.some((resource) => resource.loading),
		error: resources.find((resource) => resource.error)?.error ?? null,
		midnight,
		reload: () => resources.forEach((resource) => resource.reload()),
	};
}

/** Completed or incomplete share of requests that were not cancelled. */
export function successRate(value: Pick<AnalyticsTotals, "requests" | "completed" | "incomplete" | "cancelled">): number | null {
	const base = value.requests - value.cancelled;
	return base > 0 ? (value.completed + value.incomplete) / base : null;
}

export function failedCount(value: Pick<AnalyticsBucket, "failed" | "interrupted">): number {
	return value.failed + value.interrupted;
}

/** Cached share of input tokens. */
export function cacheRate(value: Pick<AnalyticsTotals, "inputTokens" | "cachedInputTokens">): number | null {
	return value.inputTokens > 0 ? value.cachedInputTokens / value.inputTokens : null;
}

/** Index of the hourly bucket containing `now` (−1 before the range). */
export function currentBucket(analytics: UsageAnalytics, now: number): number {
	return Math.floor((now - analytics.startAt) / analytics.bucketMs);
}

/** "5 小时" / "7 天" for a quota window. */
export function windowLabel(window: Pick<WindowForecast, "kind" | "name" | "limitWindowSeconds">): string {
	switch (window.kind) {
		case "five_hour":
			return "5 小时";
		case "weekly":
			return "7 天";
		case "monthly":
			return "30 天";
		default: {
			const seconds = window.limitWindowSeconds;
			if (!seconds) return window.name;
			return seconds >= 86_400 ? `${Math.round(seconds / 86_400)} 天` : `${Math.round(seconds / 3_600)} 小时`;
		}
	}
}

/** Short account label: the local part of an email address. */
export function shortAccountName(name: string): string {
	const at = name.indexOf("@");
	return at > 0 ? name.slice(0, at) : name;
}
