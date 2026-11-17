import { BotIcon, GitBranchIcon, Layers3Icon, MessageSquareMoreIcon, TriangleAlertIcon, UserRoundIcon } from "lucide-react";
import type { ReactNode } from "react";

import type { UsageEventStatus, UsageSessionKind } from "@/admin-api";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

import { STATUS_META } from "./viz";

/** Figma Log/Status: colored dot + label. */
export function LogStatus({ status, className, label }: { status: UsageEventStatus; className?: string; label?: ReactNode }) {
	const meta = STATUS_META[status];
	return (
		<span className={cn("inline-flex items-center gap-1.5 text-[13px] leading-5 font-medium whitespace-nowrap", meta.text, className)}>
			<span aria-hidden="true" className="size-1.5 shrink-0 rounded-full" style={{ background: meta.color }} />
			{label ?? meta.label}
		</span>
	);
}

const KIND_STYLE: Record<Exclude<UsageSessionKind, "">, { label: string; className: string; icon: ReactNode }> = {
	main: { label: "主会话", className: "bg-surface-subtle text-muted-foreground", icon: <MessageSquareMoreIcon /> },
	subagent: { label: "子代理", className: "bg-[#8b5cf6]/12 text-[#7444d6] dark:text-[#b69cf8]", icon: <BotIcon /> },
	fork: { label: "分叉", className: "bg-[#14b8a6]/12 text-[#0d8a7d] dark:text-[#5fd5c6]", icon: <GitBranchIcon /> },
	compaction: { label: "压缩", className: "bg-warning-soft text-warning", icon: <Layers3Icon /> },
};

/** Figma Log/Kind: session_kind chip. Renders nothing for requests without a session. */
export function LogKind({ kind, className }: { kind: UsageSessionKind; className?: string }) {
	if (!kind) return null;
	const style = KIND_STYLE[kind];
	return (
		<Badge className={cn("h-5 gap-1 rounded-md px-1.5 text-[11px] [&>svg]:size-3!", style.className, className)} variant="secondary">
			{style.icon}
			{style.label}
		</Badge>
	);
}

/** Figma Route Target v2: account group / Codex account / unassigned. */
export function RouteTarget({ type, label, className }: { type: "group" | "account" | "unassigned"; label: string; className?: string }) {
	return (
		<Badge
			className={cn(
				"h-6 max-w-full gap-1.5 rounded-full border pr-2.5 pl-2 text-xs font-medium [&>svg]:size-3.25!",
				type === "unassigned" ? "border-warning-border bg-warning-soft text-warning" : "border-border bg-surface-subtle text-foreground",
				className,
			)}
			variant="outline"
		>
			{type === "group" ? <Layers3Icon /> : type === "account" ? <UserRoundIcon /> : <TriangleAlertIcon />}
			<span className="truncate">{label}</span>
		</Badge>
	);
}

/** Small gray tag: "API Key", "Plus", "主力池 · 3 个账户". */
export function Tag({ children, className, tone = "neutral" }: { children: ReactNode; className?: string; tone?: "neutral" | "success" | "warning" | "danger" | "info" }) {
	return (
		<Badge
			className={cn(
				"h-5 rounded-full px-1.5 text-[11px] font-medium",
				tone === "neutral" && "bg-surface-subtle text-muted-foreground",
				tone === "success" && "bg-success-soft text-success",
				tone === "warning" && "bg-warning-soft text-warning",
				tone === "danger" && "bg-danger-soft text-danger",
				tone === "info" && "bg-[#4f7cff]/10 text-[#3b63d9] dark:text-[#9fb5ff]",
				className,
			)}
			variant="secondary"
		>
			{children}
		</Badge>
	);
}


/** HTTP status / error code chip: 429, 502, 中断. */
export function CodeChip({ children, tone = "danger" }: { children: ReactNode; tone?: "danger" | "warning" | "neutral" }) {
	return (
		<span
			className={cn(
				"inline-flex h-5 items-center rounded-md px-1.5 font-mono text-[11px] font-medium",
				tone === "danger" && "bg-danger-soft text-danger",
				tone === "warning" && "bg-warning-soft text-warning",
				tone === "neutral" && "bg-surface-subtle text-muted-foreground",
			)}
		>
			{children}
		</span>
	);
}

/** Pulse dot with label: "运行正常", "已启用". */
export function StatusDot({ tone, children, className }: { tone: "success" | "warning" | "danger" | "neutral"; children?: ReactNode; className?: string }) {
	return (
		<span className={cn("inline-flex items-center gap-1.5", className)}>
			<span
				aria-hidden="true"
				className={cn(
					"size-1.5 shrink-0 rounded-full",
					tone === "success" && "bg-success",
					tone === "warning" && "bg-warning",
					tone === "danger" && "bg-danger",
					tone === "neutral" && "bg-faint",
				)}
			/>
			{children}
		</span>
	);
}
