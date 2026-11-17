// Visualization palette and labels shared by every console page.
// Colors come from the Figma "Admin v2" charts; keep them in one place.

import type { UsageEventStatus, UsageSessionKind } from "@/admin-api";

export const VIZ = {
	blue: "#4f7cff",
	blueSoft: "#a9bdff",
	teal: "#14b8a6",
	tealSoft: "#99e2d9",
	violet: "#8b5cf6",
	amber: "#f59e0b",
	pink: "#ec4899",
	gray: "#a8a29e",
	success: "#0e9f6e",
	danger: "#c65746",
	warning: "#b66b16",
	muted: "#989188",
} as const;

/** Series colors in order of rank (models, identities, accounts). */
export const SERIES_COLORS = [VIZ.blue, VIZ.violet, VIZ.teal, VIZ.amber, VIZ.pink, VIZ.gray] as const;

export function seriesColor(index: number): string {
	return SERIES_COLORS[index % SERIES_COLORS.length] ?? VIZ.gray;
}

/** Token composition: uncached input, cache read, output, reasoning. */
export const TOKEN_PARTS = {
	input: { label: "未缓存输入", short: "输入", color: VIZ.blue },
	cached: { label: "缓存读取", short: "缓存", color: VIZ.blueSoft },
	output: { label: "输出", short: "输出", color: VIZ.teal },
	reasoning: { label: "推理", short: "推理", color: VIZ.violet },
	cacheRate: { label: "缓存命中率", short: "缓存命中率", color: VIZ.amber },
} as const;

export interface TokenSplit {
	input: number;
	cached: number;
	output: number;
	reasoning: number;
}

/** Splits token totals into non-overlapping parts for stacked bars. */
export function tokenSplit(value: {
	inputTokens: number;
	cachedInputTokens: number;
	outputTokens: number;
	reasoningOutputTokens: number;
}): TokenSplit {
	return {
		input: Math.max(0, value.inputTokens - value.cachedInputTokens),
		cached: value.cachedInputTokens,
		output: Math.max(0, value.outputTokens - value.reasoningOutputTokens),
		reasoning: value.reasoningOutputTokens,
	};
}

export const STATUS_META: Record<UsageEventStatus, { label: string; color: string; text: string }> = {
	completed: { label: "已完成", color: VIZ.success, text: "text-foreground" },
	failed: { label: "失败", color: VIZ.danger, text: "text-destructive" },
	cancelled: { label: "客户端取消", color: VIZ.muted, text: "text-muted-foreground" },
	interrupted: { label: "中断", color: VIZ.warning, text: "text-warning" },
	incomplete: { label: "未完成", color: VIZ.amber, text: "text-warning" },
};

export const STATUS_ORDER: UsageEventStatus[] = ["completed", "failed", "cancelled", "interrupted", "incomplete"];

export const SESSION_KIND_META: Record<Exclude<UsageSessionKind, "">, { label: string; color: string }> = {
	main: { label: "主会话", color: VIZ.blue },
	subagent: { label: "子代理", color: VIZ.violet },
	fork: { label: "分叉", color: VIZ.teal },
	compaction: { label: "上下文压缩", color: VIZ.amber },
};

export function sessionKindLabel(kind: UsageSessionKind): string {
	return kind ? SESSION_KIND_META[kind].label : "未标识";
}

export function sessionKindColor(kind: UsageSessionKind): string {
	return kind ? SESSION_KIND_META[kind].color : "#d6d3d1";
}

/** Requested or answered service tier. */
export function tierLabel(tier: string): string {
	switch (tier) {
		case "priority":
			return "Fast";
		case "default":
			return "标准";
		case "auto":
			return "自动";
		case "flex":
			return "Flex";
		case "":
			return "—";
		default:
			return tier;
	}
}

export function transportLabel(transport: string): string {
	return transport === "websocket" ? "WebSocket" : transport === "http" ? "HTTP" : transport || "—";
}

export function identityTypeLabel(type: string): string {
	return type === "api_key" ? "API Key" : type === "auth_proxy" ? "下游账户" : type;
}

export function planLabel(plan: string | null | undefined): string {
	if (!plan) return "";
	return plan.charAt(0).toUpperCase() + plan.slice(1);
}
