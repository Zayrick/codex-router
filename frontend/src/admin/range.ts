import type { AnalyticsQuery } from "@/admin-api";
import { DAY_MS, startOfLocalDay } from "@/lib/format";

/** Time ranges offered by toolbars; `today` and `cycle` resolve to custom bounds. */
export type RangeKey = "cycle" | "today" | "24h" | "7d" | "30d" | "all";

export const RANGE_LABELS: Record<RangeKey, string> = {
	cycle: "本周期",
	today: "今天",
	"24h": "24 小时",
	"7d": "7 天",
	"30d": "30 天",
	all: "全部",
};

export interface Bounds {
	startAt: number;
	endAt: number;
}

/**
 * Query bounds for a range key. `cycle` needs the bounds of the quota window
 * it refers to (falls back to 7 days without them).
 */
export function rangeQuery(key: RangeKey, now: number, cycle?: Bounds | null): Pick<AnalyticsQuery, "range" | "startAt" | "endAt"> {
	switch (key) {
		case "today": {
			const startAt = startOfLocalDay(now);
			return { range: "custom", startAt, endAt: startAt + DAY_MS };
		}
		case "cycle":
			return cycle ? { range: "custom", startAt: cycle.startAt, endAt: cycle.endAt } : { range: "7d" };
		default:
			return { range: key };
	}
}

/** Previous period the backend compares with: same length, same elapsed time. */
export function previousPeriod(startAt: number, endAt: number, now: number): Bounds {
	const length = endAt - startAt;
	return { startAt: startAt - length, endAt: Math.min(endAt, now) - length };
}

export function parseRangeKey(value: string | null, fallback: RangeKey = "7d"): RangeKey {
	return value && value in RANGE_LABELS ? (value as RangeKey) : fallback;
}
