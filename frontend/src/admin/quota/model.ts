// Quota window labels, risk tones and forecast copy shared by the Codex
// account, account group and user route-quota views.

import type { AccountQuota, CodexAccount, UpstreamAccountStats, WindowForecast } from "@/admin-api";
import { VIZ } from "@/components/app/viz";
import { DAY_MS, HOUR_MS, formatClock, formatDateTime, formatSpan, startOfLocalDay } from "@/lib/format";

export type RiskTone = "danger" | "warning" | "success" | "neutral";

/** Exhaustion closer than this is shown in danger tone; later ones in warning. */
const URGENT_MS = DAY_MS;
/** Exhaustion closer than this is spoken relative ("约 57 分钟后"). */
const RELATIVE_MS = 3 * HOUR_MS;

/** Codex windows only when present; other categories otherwise. */
export function visibleWindows(windows: WindowForecast[]): WindowForecast[] {
	const codex = windows.filter((window) => window.category === "codex");
	return codex.length ? codex : windows;
}

/** "5 小时" / "7 天" from the window length. */
export function windowLabel(window: Pick<WindowForecast, "kind" | "limitWindowSeconds">): string {
	const seconds = window.limitWindowSeconds;
	if (seconds && seconds > 0) {
		const hours = seconds / 3600;
		if (hours < 24) return `${Number.isInteger(hours) ? hours : hours.toFixed(1)} 小时`;
		const days = hours / 24;
		return `${Number.isInteger(days) ? days : days.toFixed(1)} 天`;
	}
	switch (window.kind) {
		case "five_hour":
			return "5 小时";
		case "weekly":
			return "7 天";
		case "monthly":
			return "30 天";
		default:
			return "额度";
	}
}

export function windowTone(window: WindowForecast, now: number): RiskTone {
	if (window.limitReached || (window.usedPercent ?? 0) >= 100) return "danger";
	if (window.exhaustsBeforeReset && window.exhaustAt !== null) return window.exhaustAt - now < URGENT_MS ? "danger" : "warning";
	return "success";
}

export const TONE_COLOR: Record<RiskTone, string> = {
	danger: VIZ.danger,
	warning: VIZ.amber,
	success: VIZ.success,
	neutral: VIZ.muted,
};

export function worstTone(tones: RiskTone[]): RiskTone {
	if (tones.includes("danger")) return "danger";
	if (tones.includes("warning")) return "warning";
	if (tones.includes("success")) return "success";
	return "neutral";
}

export function accountTone(quota: AccountQuota | undefined, now: number): RiskTone {
	if (!quota || !quota.windows.length) return "neutral";
	return worstTone(visibleWindows(quota.windows).map((window) => windowTone(window, now)));
}

/** "今天 16:42" / "11/21 09:15". */
export function formatResetAt(at: number | null, now: number): string {
	if (!at) return "—";
	return startOfLocalDay(at) === startOfLocalDay(now) ? `今天 ${formatClock(at, false)}` : formatDateTime(at);
}

/** Short time for pills: "16:42" today, "11/21 09:15" otherwise. */
export function formatShortAt(at: number, now: number): string {
	return startOfLocalDay(at) === startOfLocalDay(now) ? formatClock(at, false) : formatDateTime(at);
}

export interface ForecastCopy {
	tone: RiskTone;
	/** Headline: "约 57 分钟后耗尽" / "重置前不会耗尽". */
	title: string;
	/** Secondary line: "早于重置 2h 36m" / "预计重置时剩余 52%". */
	detail: string | null;
	exhausts: boolean;
}

export function forecastCopy(window: WindowForecast, now: number): ForecastCopy {
	const tone = windowTone(window, now);
	if (window.limitReached || (window.usedPercent ?? 0) >= 100) {
		return { tone, title: "已耗尽", detail: window.resetAt ? `${formatResetAt(window.resetAt, now)} 重置` : null, exhausts: true };
	}
	if (window.exhaustsBeforeReset && window.exhaustAt !== null) {
		const until = window.exhaustAt - now;
		const title = until < RELATIVE_MS ? `约 ${formatSpan(until)}后耗尽` : `约 ${formatDateTime(window.exhaustAt)} 耗尽`;
		const detail = window.resetAt ? `早于重置 ${formatSpan(window.resetAt - window.exhaustAt)}` : null;
		return { tone, title, detail, exhausts: true };
	}
	const used = window.usedPercent;
	const burn = window.burnPercentPerHour;
	let detail: string | null = null;
	if (used !== null && burn !== null && burn > 0 && window.resetAt) {
		const left = 100 - used - (burn * Math.max(0, window.resetAt - now)) / HOUR_MS;
		detail = `预计重置时剩余 ${Math.max(0, Math.round(left))}%`;
	} else if (burn === 0 || burn === null) {
		detail = "当前无消耗";
	}
	return { tone, title: "重置前不会耗尽", detail, exhausts: false };
}

export function formatBurn(value: number | null): string {
	if (value === null || !Number.isFinite(value)) return "—";
	const digits = value < 1 ? 2 : value < 10 ? 1 : 0;
	return `${value.toFixed(digits)}%/h`;
}

/** Display name of a Codex account: its email when known. */
export function accountTitle(account: Pick<CodexAccount, "name" | "oauth">): string {
	return account.oauth?.email || account.name;
}

/** "ops-team" from "ops-team@example.com". */
export function shortAccountName(value: string): string {
	return value.split("@")[0] || value;
}

export function initial(value: string): string {
	return (value.trim()[0] ?? "?").toUpperCase();
}

/** acc_••••c210 */
export function maskAccountId(value: string | null | undefined): string {
	if (!value) return "—";
	if (value.length <= 8) return value;
	return `${value.slice(0, 4)}••••${value.slice(-4)}`;
}

export function successRate(stats: Pick<UpstreamAccountStats, "requests" | "completed" | "incomplete" | "cancelled">): number | null {
	const decided = stats.requests - stats.cancelled;
	return decided > 0 ? (stats.completed + stats.incomplete) / decided : null;
}

export function cacheRate(stats: { inputTokens: number; cachedInputTokens: number }): number | null {
	return stats.inputTokens > 0 ? stats.cachedInputTokens / stats.inputTokens : null;
}

/** Risk sort key: soonest projected exhaustion first, then most used. */
export function riskScore(quota: AccountQuota | undefined, now: number): number {
	if (!quota) return Number.POSITIVE_INFINITY;
	let best = Number.POSITIVE_INFINITY;
	for (const window of visibleWindows(quota.windows)) {
		if (window.limitReached) return now;
		if (window.exhaustsBeforeReset && window.exhaustAt !== null) best = Math.min(best, window.exhaustAt);
	}
	if (best !== Number.POSITIVE_INFINITY) return best;
	const used = Math.max(0, ...visibleWindows(quota.windows).map((window) => window.usedPercent ?? 0));
	return now + 365 * DAY_MS * (1 - used / 100);
}

export function planText(plan: string | null | undefined): string {
	if (!plan) return "";
	return plan.replace(/[_-]+/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}
