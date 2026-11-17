import { useId } from "react";
import { Area, AreaChart, Line, ReferenceDot, YAxis } from "recharts";

import { ChartContainer, type ChartConfig } from "@/components/ui/chart";
import { cn } from "@/lib/utils";

export interface SparkPoint {
	value: number | null;
	/** Optional comparison series drawn dashed (e.g. yesterday). */
	compare?: number | null;
}

/**
 * Small area sparkline used in KPI cards. A comparison series renders as a
 * dashed gray line; `markLast` puts a dot on the latest real value.
 */
export function Sparkline({
	points,
	color,
	height = 40,
	className,
	markLast = false,
	fill = true,
}: {
	points: SparkPoint[];
	color: string;
	height?: number;
	className?: string;
	markLast?: boolean;
	fill?: boolean;
}) {
	const id = useId().replace(/:/g, "");
	const data = points.map((point, index) => ({ index, value: point.value, compare: point.compare ?? null }));
	const hasCompare = points.some((point) => point.compare !== undefined && point.compare !== null);
	let lastIndex = -1;
	for (let index = data.length - 1; index >= 0; index -= 1) {
		if (data[index]?.value !== null && data[index]?.value !== undefined) {
			lastIndex = index;
			break;
		}
	}
	const last = lastIndex >= 0 ? data[lastIndex] : undefined;
	const config = { value: { label: "当前", color }, compare: { label: "对比", color: "#989188" } } satisfies ChartConfig;
	return (
		<ChartContainer className={cn("aspect-auto w-full", className)} config={config} style={{ height }}>
			<AreaChart data={data} margin={{ top: 4, right: 4, bottom: 2, left: 0 }}>
				<defs>
					<linearGradient id={`spark-${id}`} x1="0" x2="0" y1="0" y2="1">
						<stop offset="0%" stopColor={color} stopOpacity={0.22} />
						<stop offset="100%" stopColor={color} stopOpacity={0.02} />
					</linearGradient>
				</defs>
				<YAxis domain={[0, "dataMax"]} hide />
				{hasCompare ? (
					<Line dataKey="compare" dot={false} isAnimationActive={false} stroke="#989188" strokeDasharray="3 3" strokeWidth={1.25} type="monotone" />
				) : null}
				<Area
					connectNulls={false}
					dataKey="value"
					dot={false}
					fill={fill ? `url(#spark-${id})` : "transparent"}
					isAnimationActive={false}
					stroke={color}
					strokeWidth={1.75}
					type="monotone"
				/>
				{markLast && last && last.value !== null ? (
					<ReferenceDot fill={color} r={3} stroke="var(--card)" strokeWidth={1.5} x={last.index} y={last.value} />
				) : null}
			</AreaChart>
		</ChartContainer>
	);
}
