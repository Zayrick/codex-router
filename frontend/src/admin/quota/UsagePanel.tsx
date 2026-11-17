import type { ReactNode } from "react";

import { Sparkline } from "@/components/app/sparkline";
import { VIZ } from "@/components/app/viz";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";

export interface PanelStat {
	label: string;
	value: ReactNode;
	tone?: "danger" | "default";
}

/**
 * Figma "usage-7d": subtle side panel with 7-day tokens, a sparkline, a 3-column
 * stat grid and a free-form footer (routing sources, identities).
 */
export function UsagePanel({
	tokens,
	unit = "Token",
	aside,
	trend,
	stats,
	children,
	className,
}: {
	tokens: ReactNode;
	unit?: ReactNode;
	aside?: ReactNode;
	trend?: number[] | null;
	stats: PanelStat[];
	children?: ReactNode;
	className?: string;
}) {
	const hasTrend = Boolean(trend?.some((value) => value > 0));
	return (
		<div className={cn("flex min-w-0 flex-col gap-3 rounded-[10px] bg-surface-subtle p-4", className)}>
			<div className="flex items-center gap-2">
				<span className="flex-1 text-[13px] leading-5 font-semibold">7 天用量</span>
				{aside ? <span className="text-[11.5px] leading-4 text-faint">{aside}</span> : null}
			</div>
			<div className="flex items-baseline gap-1.5">
				<span className="text-[22px] leading-7 font-bold tracking-[-0.03em] tabular-nums">{tokens}</span>
				<span className="text-xs text-muted-foreground">{unit}</span>
			</div>
			{hasTrend && trend ? <Sparkline color={VIZ.blue} height={36} points={trend.map((value) => ({ value }))} /> : null}
			<div className="grid grid-cols-3 gap-x-2 gap-y-3">
				{stats.map((stat) => (
					<div className="flex min-w-0 flex-col gap-0.5" key={stat.label}>
						<span className="truncate text-[11.5px] leading-4 text-faint">{stat.label}</span>
						<span className={cn("text-[13px] leading-5 font-semibold tabular-nums", stat.tone === "danger" && "text-danger")}>{stat.value}</span>
					</div>
				))}
			</div>
			{children ? (
				<>
					<Separator />
					{children}
				</>
			) : null}
		</div>
	);
}
