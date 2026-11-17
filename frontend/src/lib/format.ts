// Shared number, duration and time formatting for every console view.

const INTEGER = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 });
const GROUPED = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });

export const SECOND_MS = 1_000;
export const MINUTE_MS = 60 * SECOND_MS;
export const HOUR_MS = 60 * MINUTE_MS;
export const DAY_MS = 24 * HOUR_MS;

/** Integer with thousands separators: 3,484. */
export function formatCount(value: number | null | undefined): string {
	if (value === null || value === undefined || !Number.isFinite(value)) return "—";
	return GROUPED.format(Math.round(value));
}

/** Large counts in Chinese units: 9,120 · 5.84万 · 1.90亿. */
export function formatTokens(value: number | null | undefined): string {
	if (value === null || value === undefined || !Number.isFinite(value)) return "—";
	const abs = Math.abs(value);
	if (abs >= 1e8) return `${trimFixed(value / 1e8, abs >= 1e10 ? 0 : 2)}亿`;
	if (abs >= 1e4) return `${trimFixed(value / 1e4, abs >= 1e6 ? 0 : abs >= 1e5 ? 1 : 2)}万`;
	return INTEGER.format(Math.round(value));
}

/** Dollar amounts; small values keep three decimals: $0.019. */
export function formatCost(value: number | null | undefined, options: { signed?: boolean } = {}): string {
	if (value === null || value === undefined || !Number.isFinite(value)) return "—";
	const abs = Math.abs(value);
	const digits = abs > 0 && abs < 1 ? 3 : 2;
	const text = `$${abs.toLocaleString("en-US", { minimumFractionDigits: digits, maximumFractionDigits: digits })}`;
	if (value < 0) return `-${text}`;
	return options.signed ? `+${text}` : text;
}

/** Ratio (0–1) as a percentage: 97.2%. */
export function formatPercent(ratio: number | null | undefined, digits = 1): string {
	if (ratio === null || ratio === undefined || !Number.isFinite(ratio)) return "—";
	return `${(ratio * 100).toFixed(digits)}%`;
}

/** Already-percent value (0–100): 77%. */
export function formatPercentValue(value: number | null | undefined, digits = 0): string {
	if (value === null || value === undefined || !Number.isFinite(value)) return "—";
	return `${value.toFixed(digits)}%`;
}

export function ratio(part: number, total: number): number | null {
	return total > 0 ? part / total : null;
}

/** Durations: 0.31s · 18.4s · 2m 14s · 1h 03m. */
export function formatDuration(ms: number | null | undefined): string {
	if (ms === null || ms === undefined || !Number.isFinite(ms)) return "—";
	if (ms < 10 * SECOND_MS) return `${(ms / SECOND_MS).toFixed(ms < SECOND_MS ? 2 : 1).replace(/\.?0+$/, "") || "0"}s`;
	if (ms < MINUTE_MS) return `${(ms / SECOND_MS).toFixed(1)}s`;
	if (ms < HOUR_MS) {
		const minutes = Math.floor(ms / MINUTE_MS);
		const seconds = Math.round((ms % MINUTE_MS) / SECOND_MS);
		return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
	}
	const hours = Math.floor(ms / HOUR_MS);
	const minutes = Math.round((ms % HOUR_MS) / MINUTE_MS);
	return `${hours}h ${String(minutes).padStart(2, "0")}m`;
}

/** Fixed-precision seconds for latency columns: 1.38s. */
export function formatSeconds(ms: number | null | undefined, digits = 2): string {
	if (ms === null || ms === undefined || !Number.isFinite(ms)) return "—";
	if (ms >= MINUTE_MS) return formatDuration(ms);
	return `${(ms / SECOND_MS).toFixed(digits)}s`;
}

/** Spoken span until/since: 3h 33m · 2 天 · 57 分钟. */
export function formatSpan(ms: number): string {
	const abs = Math.abs(ms);
	if (abs < MINUTE_MS) return `${Math.max(1, Math.round(abs / SECOND_MS))} 秒`;
	if (abs < HOUR_MS) return `${Math.round(abs / MINUTE_MS)} 分钟`;
	if (abs < DAY_MS) {
		const hours = Math.floor(abs / HOUR_MS);
		const minutes = Math.round((abs % HOUR_MS) / MINUTE_MS);
		return minutes ? `${hours}h ${minutes}m` : `${hours} 小时`;
	}
	const days = abs / DAY_MS;
	return `${days >= 10 ? Math.round(days) : trimFixed(days, 1)} 天`;
}

/** 44 秒前 · 3 分钟前 · 2 小时前 · 5 天前. */
export function formatAgo(at: number | null | undefined, now: number): string {
	if (!at) return "—";
	const diff = Math.max(0, now - at);
	if (diff < MINUTE_MS) return `${Math.max(1, Math.round(diff / SECOND_MS))} 秒前`;
	if (diff < HOUR_MS) return `${Math.round(diff / MINUTE_MS)} 分钟前`;
	if (diff < DAY_MS) return `${Math.round(diff / HOUR_MS)} 小时前`;
	return `${Math.round(diff / DAY_MS)} 天前`;
}

/** 2h 10m 后 · 3 天后. */
export function formatUntil(at: number | null | undefined, now: number): string {
	if (!at) return "—";
	const diff = at - now;
	if (diff <= 0) return "已到期";
	return `${formatSpan(diff)}后`;
}

const pad = (value: number) => String(value).padStart(2, "0");

/** 14:31:48 */
export function formatClock(at: number, seconds = true): string {
	const date = new Date(at);
	const time = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
	return seconds ? `${time}:${pad(date.getSeconds())}` : time;
}

/** 11/17 */
export function formatDay(at: number): string {
	const date = new Date(at);
	return `${date.getMonth() + 1}/${pad(date.getDate())}`;
}

/** 11/17 18:05 */
export function formatDateTime(at: number | null | undefined): string {
	if (!at) return "—";
	return `${formatDay(at)} ${formatClock(at, false)}`;
}

/** 2026/11/10 14:32 */
export function formatFullDateTime(at: number): string {
	const date = new Date(at);
	return `${date.getFullYear()}/${date.getMonth() + 1}/${pad(date.getDate())} ${formatClock(at, false)}`;
}

/** 2026-11-17 14:23:05.218 */
export function formatTimestamp(at: number): string {
	const date = new Date(at);
	return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${formatClock(at)}.${String(date.getMilliseconds()).padStart(3, "0")}`;
}

const WEEKDAYS = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

export function formatWeekday(at: number): string {
	return WEEKDAYS[new Date(at).getDay()] ?? "";
}

export function startOfLocalDay(at: number): number {
	const date = new Date(at);
	date.setHours(0, 0, 0, 0);
	return date.getTime();
}

export function isWeekend(at: number): boolean {
	const day = new Date(at).getDay();
	return day === 0 || day === 6;
}

export interface Delta {
	text: string;
	/** Whether the change is good, bad or neutral for the metric. */
	tone: "success" | "danger" | "neutral";
}

/** Relative change "↑ 8.2%"; `higherIsBetter` decides the tone. */
export function percentDelta(current: number, previous: number | null | undefined, higherIsBetter = true): Delta | null {
	if (previous === null || previous === undefined || !Number.isFinite(previous) || previous <= 0) return null;
	const change = (current - previous) / previous;
	// A near-empty previous period makes the ratio meaningless.
	if (!Number.isFinite(change) || Math.abs(change) > 9.99) return null;
	const arrow = change >= 0 ? "↑" : "↓";
	const good = change === 0 ? null : change > 0 === higherIsBetter;
	return { text: `${arrow} ${Math.abs(change * 100).toFixed(1)}%`, tone: good === null ? "neutral" : good ? "success" : "danger" };
}

/** Change of a ratio in percentage points "↑ 2.1 pt". */
export function pointDelta(current: number | null, previous: number | null, higherIsBetter = true): Delta | null {
	if (current === null || previous === null) return null;
	const change = (current - previous) * 100;
	if (!Number.isFinite(change)) return null;
	const arrow = change >= 0 ? "↑" : "↓";
	const good = Math.abs(change) < 0.05 ? null : change > 0 === higherIsBetter;
	return { text: `${arrow} ${Math.abs(change).toFixed(1)} pt`, tone: good === null ? "neutral" : good ? "success" : "danger" };
}

export function maskSecret(value: string, visible = 4): string {
	if (value.length <= visible + 3) return value;
	return `${value.slice(0, 3)}••••••••${value.slice(-visible)}`;
}

/** sess_9f2c41…7be0 */
export function shortId(value: string, head = 11, tail = 4): string {
	if (value.length <= head + tail + 1) return value;
	return `${value.slice(0, head)}…${value.slice(-tail)}`;
}

function trimFixed(value: number, digits: number): string {
	return value.toFixed(digits);
}
