import type { ReactNode } from "react";

import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import type { Delta } from "@/lib/format";
import { cn } from "@/lib/utils";

/** Figma "delta" pill: ↑ 8.2% in success / danger tones. */
export function DeltaBadge({ delta, className }: { delta: Delta | null | undefined; className?: string }) {
	if (!delta) return null;
	return (
		<Badge
			className={cn(
				"h-5 rounded-full px-1.5 text-[11.5px] font-semibold",
				delta.tone === "success" && "bg-success-soft text-success",
				delta.tone === "danger" && "bg-danger-soft text-danger",
				delta.tone === "neutral" && "bg-surface-subtle text-muted-foreground",
				className,
			)}
			variant="secondary"
		>
			{delta.text}
		</Badge>
	);
}

/**
 * Figma "kpi · …" cards: label (+ icon), delta, 30px value with optional unit,
 * a detail line and an optional visual (sparkline, meter, legend).
 */
export function StatCard({
	label,
	icon,
	delta,
	aside,
	value,
	unit,
	valueClassName,
	detail,
	children,
	className,
	size = "default",
}: {
	label: ReactNode;
	icon?: ReactNode;
	delta?: Delta | null;
	/** Right side of the header when there is no delta (e.g. a status icon). */
	aside?: ReactNode;
	value: ReactNode;
	unit?: ReactNode;
	valueClassName?: string;
	detail?: ReactNode;
	children?: ReactNode;
	className?: string;
	/** `sm` uses the 24px value of summary rows. */
	size?: "default" | "sm";
}) {
	return (
		<Card className={cn("min-w-0 gap-2 px-6 py-6", size === "sm" && "py-5", className)}>
			<div className="flex items-center justify-between gap-2">
				<span className="flex min-w-0 items-center gap-1 text-[12.5px] leading-5 font-medium text-muted-foreground [&_svg]:size-3.5">
					{icon}
					<span className="truncate">{label}</span>
				</span>
				{delta ? <DeltaBadge delta={delta} /> : aside}
			</div>
			<div className="flex items-baseline gap-1.5">
				<span
					className={cn(
						"font-bold tracking-[-0.035em] tabular-nums",
						size === "sm" ? "text-2xl leading-8" : "text-[30px] leading-10",
						valueClassName,
					)}
				>
					{value}
				</span>
				{unit ? <span className="text-[12.5px] text-muted-foreground">{unit}</span> : null}
			</div>
			{detail ? <div className="text-xs leading-4 text-muted-foreground">{detail}</div> : null}
			{children}
		</Card>
	);
}

/** Label over a bold value, used in status strips and detail grids. */
export function Metric({
	label,
	value,
	detail,
	className,
	valueClassName,
}: {
	label: ReactNode;
	value: ReactNode;
	detail?: ReactNode;
	className?: string;
	valueClassName?: string;
}) {
	return (
		<div className={cn("flex min-w-0 flex-col gap-1", className)}>
			<span className="text-[11.5px] leading-4 font-medium text-faint">{label}</span>
			<div className="flex items-baseline gap-2">
				<span className={cn("text-xl leading-7 font-bold tracking-[-0.02em] tabular-nums", valueClassName)}>{value}</span>
				{detail ? <span className="text-xs leading-4 text-muted-foreground">{detail}</span> : null}
			</div>
		</div>
	);
}
