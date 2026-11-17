import { toast } from "sonner";

import type { UsageEvent } from "@/admin-api";
import { sessionKindLabel, tierLabel, transportLabel } from "@/components/app/viz";
import { shortId } from "@/lib/format";

export function isSuccess(event: UsageEvent): boolean {
	return event.status === "completed" || event.status === "incomplete";
}

export function isDowngraded(event: UsageEvent): boolean {
	return event.serviceTier === "priority" && event.responseServiceTier !== "" && event.responseServiceTier !== "priority";
}

/** Time spent generating after the first token. */
export function generationMs(event: UsageEvent): number | null {
	return event.ttftMs === null ? null : Math.max(0, event.latencyMs - event.ttftMs);
}

export function outputSpeed(event: UsageEvent): number | null {
	return event.status === "completed" && event.outputTokens > 0 && event.latencyMs > 0 ? (event.outputTokens * 1000) / event.latencyMs : null;
}

export function transportText(event: UsageEvent): string {
	return `${transportLabel(event.transport)} · ${event.stream ? "流式" : "非流式"}`;
}

export function upstreamName(event: UsageEvent): string {
	return event.accountGroupName || (event.codexAccountName ? "直连" : "—");
}

export function sessionText(id: string, kind: UsageEvent["sessionKind"]): string {
	if (!id) return "—";
	return kind ? `${shortId(id)} · ${sessionKindLabel(kind)}` : shortId(id);
}

/** "Fast" / "标准"; downgrades read "Fast→标准". */
export function tierText(event: UsageEvent): string {
	const requested = tierLabel(event.serviceTier);
	return isDowngraded(event) ? `${requested}→${tierLabel(event.responseServiceTier)}` : requested;
}

export function copyText(value: string, label: string): void {
	if (!value) return;
	navigator.clipboard
		.writeText(value)
		.then(() => toast.success(`已复制 ${label}`))
		.catch(() => toast.error("复制失败。"));
}

/** "10 分钟", "6 小时", "1 天". */
export function bucketLabel(ms: number): string {
	const minutes = Math.round(ms / 60_000);
	if (minutes < 60) return `${minutes} 分钟`;
	if (minutes < 1440) return `${minutes / 60} 小时`;
	return `${minutes / 1440} 天`;
}
