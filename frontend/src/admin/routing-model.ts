// Pure helpers for routes, identities and account groups shared by dialogs and pages.

import type { AccountGroup, AccountGroupStrategy, RouteAssignment } from "@/admin-api";

export const MIN_API_KEY_LENGTH = 11;
export const MAX_API_KEY_LENGTH = 512;
const GENERATED_API_KEY_LENGTH = 20;
export const MAX_ACCOUNT_ID_LENGTH = 256;

export type RouteTargetSelection = Pick<RouteAssignment, "targetType" | "targetId"> | null;

export const STRATEGY_OPTIONS = [
	{ value: "round-robin", label: "轮询" },
	{ value: "weighted-round-robin", label: "额度加权轮询" },
	{ value: "fallback", label: "用户粘性 / Fallback" },
] as const satisfies ReadonlyArray<{
	value: AccountGroupStrategy;
	label: string;
}>;

export function userRoute(routes: RouteAssignment[], userId: string): RouteAssignment | null {
	return routes.find((route) => route.userId === userId) ?? null;
}

export function replaceUserRoute(routes: RouteAssignment[], userId: string, target: RouteTargetSelection): RouteAssignment[] {
	const retained = routes.filter((route) => route.userId !== userId);
	return target ? [...retained, { userId, ...target }] : retained;
}

export function routeTargetValue(target: RouteTargetSelection): string {
	return target ? `${target.targetType}:${target.targetId}` : "";
}

export function parseRouteTargetValue(value: string): RouteTargetSelection {
	const separator = value.indexOf(":");
	if (separator < 1) return null;
	const targetType = value.slice(0, separator);
	const targetId = value.slice(separator + 1);
	if (!targetId || (targetType !== "account" && targetType !== "group")) return null;
	return { targetType, targetId };
}

export function validApiKey(value: string): boolean {
	return value.length >= MIN_API_KEY_LENGTH && value.length <= MAX_API_KEY_LENGTH && /[A-Za-z]/.test(value) && /[0-9]/.test(value) && /[^A-Za-z0-9\s]/.test(value);
}

export function validAccountId(value: string): boolean {
	return value.length > 0 && value.length <= MAX_ACCOUNT_ID_LENGTH && Array.from(value).every((character) => { const code = character.charCodeAt(0); return code >= 0x21 && code <= 0x7e; });
}

export function maskApiKey(value: string): string {
	return value.length < 9 ? "••••••" : `${value.slice(0, 3)}••••••••${value.slice(-4)}`;
}

export function generateApiKey(): string {
	const alphabet = "abcdefghijklmnopqrstuvwxyz0123456789";
	const bytes = new Uint8Array(32);
	for (;;) {
		let value = "";
		while (value.length < GENERATED_API_KEY_LENGTH) {
			crypto.getRandomValues(bytes);
			for (const byte of bytes) {
				if (byte < 252) value += alphabet[byte % alphabet.length];
				if (value.length === GENERATED_API_KEY_LENGTH) break;
			}
		}
		if (/[a-z]/.test(value) && /[0-9]/.test(value)) return `sk-${value}`;
	}
}

export function newGroup(): AccountGroup {
	return {
		id: crypto.randomUUID(),
		name: "",
		accountIds: [],
		strategy: "round-robin",
		sessionAffinity: true,
		sessionAffinityTtl: "1h",
	};
}

export function strategyLabel(value: AccountGroupStrategy): string {
	if (value === "weighted-round-robin") return "额度加权轮询";
	if (value === "fallback") return "Fallback";
	return "轮询";
}

export function strategyHint(value: AccountGroupStrategy): string {
	if (value === "weighted-round-robin") return "按各账户 Codex 额度窗口的平均“剩余百分比 ÷ 剩余分钟”平滑分配。";
	if (value === "fallback") return "每个用户固定使用一个账户，仅在该账户不可用时切换。";
	return "按账户顺序平均轮换请求。";
}

export function formatTtl(value: string): string {
	return value === "unlimited" ? "不限期" : value;
}
