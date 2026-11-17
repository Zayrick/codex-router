import type { ReactNode } from "react";

import { Legend, LegendItem } from "@/components/app/bars";
import { cn } from "@/lib/utils";


export interface HeatmapRow {
	key: string;
	label: ReactNode;
	/** Emphasize the row label (today). */
	current?: boolean;
	/** `null` = not reached yet (future hour); 0 / undefined handled by `color`. */
	cells: (number | null)[];
}

/**
 * Day × hour grid (Figma "heatmap"): a label column, one cell per column and
 * hour ticks underneath. Callers decide each cell's color and tooltip.
 */
export function Heatmap({
	rows,
	color,
	title,
	columnTicks = [0, 6, 12, 18, 23],
	className,
}: {
	rows: HeatmapRow[];
	color: (value: number, row: number, column: number) => string;
	title?: (value: number | null, row: number, column: number) => string;
	/** Column indices labeled below the grid (as two-digit hours). */
	columnTicks?: number[];
	className?: string;
}) {
	const columns = Math.max(0, ...rows.map((row) => row.cells.length));
	return (
		<div className={cn("flex flex-col gap-1", className)} role="img">
			{rows.map((row, rowIndex) => (
				<div className="grid grid-cols-[4rem_1fr] items-center gap-2" key={row.key}>
					<span className={cn("truncate text-[11px] leading-4 text-muted-foreground tabular-nums", row.current && "font-semibold text-foreground")}>{row.label}</span>
					<div className="grid gap-[3px]" style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}>
						{row.cells.map((value, column) => (
							<span
								className={cn("h-4 rounded-[3px]", value === null && "border border-dashed border-border bg-transparent")}
								key={column}
								style={value === null ? undefined : { background: color(value, rowIndex, column) }}
								title={title?.(value, rowIndex, column)}
							/>
						))}
					</div>
				</div>
			))}
			<div className="grid grid-cols-[4rem_1fr] gap-2 pt-1">
				<span />
				<div className="relative h-4">
					{columnTicks.map((tick) => (
						<span
							className="absolute -translate-x-1/2 font-mono text-[10.5px] leading-4 text-faint"
							key={tick}
							style={{ left: `${columns ? ((tick + 0.5) / columns) * 100 : 0}%` }}
						>
							{String(tick).padStart(2, "0")}
						</span>
					))}
				</div>
			</div>
		</div>
	);
}

/** Legend under a heatmap; `future` adds the dashed "未到" swatch. */
export function HeatmapLegend({ items, future = false }: { items: { color: string; label: string }[]; future?: boolean }) {
	return (
		<Legend className="gap-x-4">
			{items.map((item) => (
				<LegendItem className="text-[11px]" color={item.color} key={item.label} label={item.label} />
			))}
			{future ? (
				<span className="inline-flex items-center gap-1.5 text-[11px] leading-4 text-muted-foreground">
					<span aria-hidden="true" className="size-2 rounded-[2px] border border-dashed border-border-strong" />
					未到
				</span>
			) : null}
		</Legend>
	);
}

