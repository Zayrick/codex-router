import { RefreshCwIcon } from "lucide-react";
import type { ReactNode } from "react";

import { RANGE_LABELS, previousPeriod, type RangeKey } from "@/admin/range";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { formatDateTime, formatDay, formatFullDateTime } from "@/lib/format";
import { cn } from "@/lib/utils";

import { Segmented } from "./segmented";

/**
 * Figma "usage-toolbar": range segmented control, the resolved range with its
 * comparison period, filters on the right and a refresh button.
 */
export function RangeToolbar({
	value,
	onChange,
	options,
	bounds,
	now,
	compare = true,
	children,
	onRefresh,
	loading = false,
	className,
	caption,
}: {
	value: RangeKey;
	onChange: (value: RangeKey) => void;
	options: RangeKey[];
	/** Resolved bounds of the current data, when loaded. */
	bounds: { startAt: number; endAt: number } | null;
	now: number;
	/** Whether to describe the comparison period. */
	compare?: boolean;
	children?: ReactNode;
	onRefresh?: () => void;
	loading?: boolean;
	className?: string;
	/** Replaces the default secondary caption line. */
	caption?: ReactNode;
}) {
	const previous = bounds && compare && value !== "all" ? previousPeriod(bounds.startAt, bounds.endAt, now) : null;
	const end = bounds ? Math.min(bounds.endAt, now) : null;
	return (
		<Card className={cn("flex-row flex-wrap items-center justify-between gap-3 p-2", className)}>
			<div className="flex flex-wrap items-center gap-4">
				<Segmented
					label="时间范围"
					onChange={onChange}
					options={options.map((key) => ({ value: key, label: RANGE_LABELS[key] }))}
					size="default"
					value={value}
				/>
				{bounds && end !== null ? (
					<div className="flex flex-col gap-0.5">
						<span className="text-[12.5px] leading-5 font-medium tabular-nums">
							{formatFullDateTime(bounds.startAt)} — {formatDateTime(end)}
						</span>
						<span className="text-[11.5px] leading-4 text-faint tabular-nums">
							{caption ?? (previous ? `对比上一周期 ${formatDay(previous.startAt)} — ${formatDay(previous.endAt)}` : null)}
						</span>
					</div>
				) : null}
			</div>
			<div className="flex flex-wrap items-center gap-2">
				{children}
				{onRefresh ? (
					<Button aria-label="刷新" disabled={loading} onClick={onRefresh} size="icon" type="button" variant="outline">
						{loading ? <Spinner /> : <RefreshCwIcon />}
					</Button>
				) : null}
			</div>
		</Card>
	);
}
