import type { CSSProperties, ReactNode } from "react";

import { Progress } from "@/components/ui/progress";
import { cn } from "@/lib/utils";

/**
 * Single-value meter on shadcn Progress. `ratio` is 0–1; `color` is any CSS
 * color (palette constant or var()).
 */
export function Meter({
	ratio,
	color,
	className,
	height = 6,
	label,
}: {
	ratio: number | null | undefined;
	color?: string;
	className?: string;
	height?: number;
	label?: string;
}) {
	const value = Math.max(0, Math.min(1, ratio ?? 0)) * 100;
	return (
		<Progress
			aria-label={label}
			className={cn(
				"bg-surface-hover [&>[data-slot=progress-indicator]]:rounded-full [&>[data-slot=progress-indicator]]:bg-(--meter-color)",
				className,
			)}
			style={{ height, "--meter-color": color ?? "var(--primary)" } as CSSProperties}
			value={value}
		/>
	);
}

export interface BarSegment {
	key: string;
	value: number;
	color: string;
	label?: string;
}

/** Multi-segment horizontal bar (token composition, outcome mix, shares). */
export function StackedBar({
	segments,
	total,
	height = 8,
	className,
	gap = 0,
}: {
	segments: BarSegment[];
	/** Defaults to the sum of the segments; a larger total leaves track visible. */
	total?: number;
	height?: number;
	className?: string;
	gap?: number;
}) {
	const sum = segments.reduce((acc, segment) => acc + Math.max(0, segment.value), 0);
	const base = Math.max(total ?? sum, sum);
	return (
		<div
			className={cn("flex w-full overflow-hidden rounded-full bg-surface-hover", className)}
			role="img"
			aria-label={segments.map((segment) => `${segment.label ?? segment.key} ${segment.value}`).join("，")}
			style={{ height, gap }}
		>
			{base > 0
				? segments.map((segment) =>
						segment.value > 0 ? (
							<div
								className="h-full shrink-0"
								key={segment.key}
								style={{ width: `${(segment.value / base) * 100}%`, background: segment.color }}
								title={segment.label}
							/>
						) : null,
					)
				: null}
		</div>
	);
}

/** Legend entry: color swatch, label and optional bold value. */
export function LegendItem({
	color,
	label,
	value,
	shape = "square",
	className,
}: {
	color: string;
	label: ReactNode;
	value?: ReactNode;
	shape?: "square" | "line" | "dashed" | "dot";
	className?: string;
}) {
	return (
		<span className={cn("inline-flex items-center gap-1.5 text-xs leading-4 text-muted-foreground", className)}>
			<span
				aria-hidden="true"
				className={cn(
					"shrink-0",
					shape === "square" && "size-2 rounded-[2px]",
					shape === "dot" && "size-1.5 rounded-full",
					shape === "line" && "h-0.5 w-3 rounded-full",
					shape === "dashed" && "h-0 w-3 border-t-[1.5px] border-dashed",
				)}
				style={shape === "dashed" ? { borderColor: color } : { background: color }}
			/>
			<span>{label}</span>
			{value !== undefined ? <span className="font-semibold text-foreground tabular-nums">{value}</span> : null}
		</span>
	);
}

export function Legend({ children, className }: { children: ReactNode; className?: string }) {
	return <div className={cn("flex flex-wrap items-center gap-x-4 gap-y-1", className)}>{children}</div>;
}

/**
 * Figma Spark Bar v2: one bar per slice, level 0 = no requests (gray
 * baseline), 1–8 = relative height in 3px steps.
 */
export function SparkBars({ values, color = "#4f7cff", className }: { values: number[]; color?: string; className?: string }) {
	const max = Math.max(0, ...values);
	return (
		<div className={cn("flex h-6 items-end gap-1", className)} aria-hidden="true">
			{values.map((value, index) => {
				const level = value > 0 && max > 0 ? Math.max(1, Math.round((value / max) * 8)) : 0;
				return (
					<span
						className={cn("w-1.5 rounded-[1.5px]", level === 0 && "rounded-[1px] bg-border-strong")}
						key={index}
						style={level === 0 ? { height: 2 } : { height: level * 3, background: color }}
					/>
				);
			})}
		</div>
	);
}

/** Thin range bar (P50–P95) on a shared scale, used by performance tables. */
export function RangeBar({
	from,
	to,
	max,
	marker,
	color,
	targetRatio,
	className,
	scale = "linear",
}: {
	from: number | null;
	to: number | null;
	max: number;
	marker?: number | null;
	color: string;
	/** Target line position on the same scale. */
	targetRatio?: number | null;
	className?: string;
	scale?: "linear" | "log";
}) {
	const position = (value: number) => {
		if (max <= 0) return 0;
		if (scale === "log") return Math.min(1, Math.log1p(Math.max(0, value)) / Math.log1p(max));
		return Math.min(1, Math.max(0, value) / max);
	};
	const start = from === null ? null : position(from);
	const end = to === null ? null : position(to);
	return (
		<div className={cn("relative h-3 w-full", className)} aria-hidden="true">
			<div className="absolute inset-x-0 top-1/2 h-px -translate-y-1/2 bg-border" />
			{start !== null && end !== null ? (
				<div
					className="absolute top-1/2 h-1 -translate-y-1/2 rounded-full opacity-60"
					style={{ left: `${start * 100}%`, width: `${Math.max(0.5, (end - start) * 100)}%`, background: color }}
				/>
			) : null}
			{marker !== null && marker !== undefined ? (
				<div className="absolute top-0 h-3 w-0.5 rounded-full" style={{ left: `${position(marker) * 100}%`, background: color }} />
			) : null}
			{targetRatio !== null && targetRatio !== undefined ? (
				<div className="absolute top-0 h-3 w-px bg-danger" style={{ left: `${Math.min(1, targetRatio) * 100}%` }} />
			) : null}
		</div>
	);
}
