// Presentational pieces shared by the analytics cards.

import type { ReactNode } from "react";

import { Card } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

/** Ranked list row: name + tag + value/share, a composition bar and a detail line. */
export function RankRow({
	rank,
	color,
	name,
	tag,
	value,
	share,
	bar,
	detail,
	indent = false,
	onClick,
}: {
	rank?: ReactNode;
	color?: string;
	name: ReactNode;
	tag?: ReactNode;
	value: ReactNode;
	share?: ReactNode;
	bar: ReactNode;
	detail?: ReactNode;
	indent?: boolean;
	onClick?: () => void;
}) {
	const content = (
		<>
			<div className="flex items-center justify-between gap-3">
				<div className="flex min-w-0 items-center gap-2">
					{rank !== undefined ? <span className="w-4 shrink-0 text-xs text-faint tabular-nums">{rank}</span> : null}
					{color ? <span aria-hidden="true" className="size-2 shrink-0 rounded-[2px]" style={{ background: color }} /> : null}
					<span className="truncate text-[13px] leading-5 font-semibold">{name}</span>
					{tag}
				</div>
				<div className="flex shrink-0 items-baseline gap-2">
					<span className="text-[13px] leading-5 font-semibold tabular-nums">{value}</span>
					{share !== undefined ? <span className="w-10 text-right text-[11.5px] text-faint tabular-nums">{share}</span> : null}
				</div>
			</div>
			{bar}
			{detail ? <div className="text-xs leading-4 text-muted-foreground tabular-nums">{detail}</div> : null}
		</>
	);
	const className = cn("flex flex-col gap-2 border-b border-border/60 py-3 last:border-b-0", indent && "pl-6");
	return onClick ? (
		<button className={cn(className, "w-full text-left hover:bg-muted/40")} onClick={onClick} type="button">
			{content}
		</button>
	) : (
		<div className={className}>{content}</div>
	);
}

/** Small numeric stat: bold value over a muted label. */
export function MiniStat({ value, label, className, valueClassName }: { value: ReactNode; label: ReactNode; className?: string; valueClassName?: string }) {
	return (
		<div className={cn("flex min-w-0 flex-col", className)}>
			<span className={cn("text-xl leading-7 font-bold tracking-[-0.02em] tabular-nums", valueClassName)}>{value}</span>
			<span className="text-xs leading-4 text-muted-foreground">{label}</span>
		</div>
	);
}

/** Uppercase-free section label inside a card. */
export function SectionLabel({ children, aside }: { children: ReactNode; aside?: ReactNode }) {
	return (
		<div className="flex items-center justify-between gap-3 text-xs leading-4">
			<span className="font-semibold text-foreground">{children}</span>
			{aside ? <span className="text-faint">{aside}</span> : null}
		</div>
	);
}

export function CardSkeleton({ className, height = 280 }: { className?: string; height?: number }) {
	return (
		<Card className={cn("gap-4 p-6", className)}>
			<Skeleton className="h-5 w-32" />
			<Skeleton className="h-4 w-48" />
			<Skeleton className="w-full" style={{ height }} />
		</Card>
	);
}

export function KpiSkeletonRow({ count = 4 }: { count?: number }) {
	return (
		<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
			{Array.from({ length: count }, (_, index) => (
				<Card className="gap-3 p-6" key={index}>
					<Skeleton className="h-4 w-16" />
					<Skeleton className="h-9 w-28" />
					<Skeleton className="h-4 w-40" />
					<Skeleton className="h-10 w-full" />
				</Card>
			))}
		</div>
	);
}

export function NoData({ title = "暂无数据", description }: { title?: string; description?: string }) {
	return (
		<Empty className="border-0 py-8">
			<EmptyHeader>
				<EmptyTitle className="text-sm">{title}</EmptyTitle>
				{description ? <EmptyDescription>{description}</EmptyDescription> : null}
			</EmptyHeader>
		</Empty>
	);
}

/** Inline error for a failed analytics request. */
export function LoadError({ message, onRetry }: { message: string; onRetry?: () => void }) {
	return (
		<Card className="flex-row items-center justify-between gap-3 border-danger/30 bg-danger-soft px-6 py-4 text-sm text-danger">
			<span>{message}</span>
			{onRetry ? (
				<button className="text-sm font-medium underline-offset-4 hover:underline" onClick={onRetry} type="button">
					重试
				</button>
			) : null}
		</Card>
	);
}

/** Signed delta text with tone: +3.6 / -2.5. */
export function SignedDelta({ value, digits = 1, higherIsBetter = true, suffix = "" }: { value: number | null; digits?: number; higherIsBetter?: boolean; suffix?: string }) {
	if (value === null || !Number.isFinite(value) || Math.abs(value) < 10 ** -digits / 2) return null;
	const good = value > 0 === higherIsBetter;
	return (
		<span className={cn("text-[11px] leading-4 font-medium tabular-nums", good ? "text-success" : "text-danger")}>
			{value > 0 ? "+" : "−"}
			{Math.abs(value).toFixed(digits)}
			{suffix}
		</span>
	);
}
