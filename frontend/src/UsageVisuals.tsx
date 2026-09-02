import { useMemo, useState, type CSSProperties } from "react";
import {
	CartesianGrid,
	Cell,
	Legend,
	Line,
	LineChart,
	Pie,
	PieChart,
	ReferenceArea,
	ReferenceDot,
	ResponsiveContainer,
	Tooltip,
	XAxis,
	YAxis,
	type TooltipContentProps,
} from "recharts";
import { Card } from "@/components/ui/card";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { ScrollArea } from "@/components/ui/scroll-area";
import type {
	UsageDashboard,
	UsageIdentityRow,
	UsageModelRow,
	UsageSeriesPoint,
} from "./admin-api";
import { formatCost } from "./usage-format";

const INTEGER_FORMAT = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 });
const COMPACT_FORMAT = new Intl.NumberFormat("zh-CN", {
	notation: "compact",
	maximumFractionDigits: 2,
});
const DONUT_COLORS = [
	"#4f7cff",
	"#8b5cf6",
	"#14b8a6",
	"#f59e0b",
	"#ec4899",
	"#06b6d4",
	"#84cc16",
	"#f97316",
	"#64748b",
	"#a855f7",
	"#10b981",
	"#ef4444",
];

type DonutMetric = "tokens" | "cost";
type DonutRow = {
	id: string;
	label: string;
	meta: string;
	tokens: number;
	cost: number;
};
type DonutChartRow = DonutRow & {
	amount: number;
	color: string;
	percent: number;
};
type TrendChartPoint = UsageSeriesPoint & {
	primaryValue: number | null;
	secondaryValue: number | null;
};
type TrendLineDefinition = {
	color: string;
	dataKey: "primaryValue" | "secondaryValue";
	name: string;
	value: (point: UsageSeriesPoint) => number;
};

export function ActivityHeatmaps({
	now,
	usage,
	stacked = false,
}: {
	now: number;
	usage: UsageDashboard;
	stacked?: boolean;
}) {
	return (
		<div className={`activity-card-grid${stacked ? " activity-card-grid-stacked" : ""}`}>
			<TokenActivityCard now={now} usage={usage} />
			<HealthActivityCard now={now} usage={usage} />
		</div>
	);
}

export function TokenActivityCard({ now, usage }: { now: number; usage: UsageDashboard }) {
	const levels = useMemo(
		() => activityLevels(usage.series.map((point) => point.totalTokens)),
		[usage.series],
	);
	return (
		<ActivityCard
			ariaLabel="Token 活动图"
			className="token-activity-card"
			getCell={(point, index, future) => ({
				className: `activity-level-${future ? 0 : levels[index]}`,
				title: `${formatDateTime(point.startAt)} · ${formatTokens(point.totalTokens)} Token · ${formatCount(point.requests)} 次请求${future ? " · 尚未发生" : ""}`,
			})}
			legend={(
				<>
					<span>少</span>
					<i className="activity-level-0" />
					{[1, 2, 3, 4, 5].map((level) => <i className={`activity-level-${level}`} key={level} />)}
					<span>多</span>
				</>
			)}
			now={now}
			series={usage.series}
			subtitle="按所选时间范围显示 Token 消耗，未来时间保持为空"
			summary={formatTokens(usage.totals.totalTokens)}
			summaryDetail={`输入 ${formatTokens(usage.totals.inputTokens)} · 输出 ${formatTokens(usage.totals.outputTokens)}`}
			title="Token 活动"
		/>
	);
}

export function CostActivityCard({ now, usage }: { now: number; usage: UsageDashboard }) {
	const levels = useMemo(
		() => activityLevels(usage.series.map((point) => point.costUsd)),
		[usage.series],
	);
	return (
		<ActivityCard
			ariaLabel="成本活动图"
			className="cost-activity-card"
			getCell={(point, index, future) => ({
				className: `activity-level-${future ? 0 : levels[index]}`,
				title: `${formatDateTime(point.startAt)} · ${formatCost(point.costUsd)} · ${formatCount(point.requests)} 次请求${future ? " · 尚未发生" : ""}`,
			})}
			legend={(
				<>
					<span>少</span>
					<i className="activity-level-0" />
					{[1, 2, 3, 4, 5].map((level) => <i className={`activity-level-${level}`} key={level} />)}
					<span>多</span>
				</>
			)}
			now={now}
			series={usage.series}
			subtitle="按所选时间范围显示已计价成本活动"
			summary={formatCost(usage.totals.costUsd)}
			summaryDetail={usage.unpricedModels.length > 0
				? `${usage.unpricedModels.length} 个模型尚未计价`
				: "已按模型价格计算"}
			title="成本活动"
		/>
	);
}

function HealthActivityCard({ now, usage }: { now: number; usage: UsageDashboard }) {
	const success = usage.series.reduce((sum, point) => sum + point.successfulRequests, 0);
	const failed = usage.series.reduce((sum, point) => sum + point.failedRequests, 0);
	const rate = success + failed > 0 ? success / (success + failed) * 100 : null;
	return (
		<ActivityCard
			ariaLabel="周期服务健康活动图"
			className="health-activity-card"
			getCell={(point, _index, future) => {
				const level = future ? 0 : healthLevel(point.successfulRequests, point.failedRequests);
				const total = point.successfulRequests + point.failedRequests;
				const pointRate = total > 0 ? point.successfulRequests / total * 100 : 0;
				return {
					className: `health-level-${level}`,
					title: `${formatDateTime(point.startAt)} · 成功 ${formatCount(point.successfulRequests)} · 失败 ${formatCount(point.failedRequests)}${total > 0 ? ` · ${pointRate.toFixed(1)}%` : ""}${future ? " · 尚未发生" : ""}`,
				};
			}}
			legend={(
				<>
					<span>异常</span>
					{[0, 1, 2, 3, 4, 5].map((level) => <i className={`health-level-${level}`} key={level} />)}
					<span>健康</span>
				</>
			)}
			now={now}
			series={usage.series}
			subtitle="按成功率观察服务状态，未完成与失败请求计为异常"
			summary={rate === null ? "—" : `${rate.toFixed(1)}%`}
			summaryDetail={`成功 ${formatCount(success)} · 失败 ${formatCount(failed)}`}
			title="服务健康"
		/>
	);
}

function ActivityCard({
	ariaLabel,
	className,
	getCell,
	legend,
	now,
	series,
	subtitle,
	summary,
	summaryDetail,
	title,
}: {
	ariaLabel: string;
	className: string;
	getCell: (point: UsageSeriesPoint, index: number, future: boolean) => { className: string; title: string };
	legend: React.ReactNode;
	now: number;
	series: UsageSeriesPoint[];
	subtitle: string;
	summary: string;
	summaryDetail: string;
	title: string;
}) {
	const columns = Math.max(1, Math.ceil(series.length / 7));
	const minimumWidth = columns * 8 + Math.max(0, columns - 1) * 3;
	return (
		<Card className={`usage-activity-card grid min-w-0 gap-3 p-3 ${className}`}>
			<header className="usage-activity-card-header">
				<div>
					<h3>{title}</h3>
					<p>{subtitle}</p>
				</div>
				<div className="usage-activity-summary">
					<strong>{summary}</strong>
					<span>{summaryDetail}</span>
				</div>
			</header>
			<div className="activity-card-visual">
				{series.length > 0 ? (
					<ScrollArea className="activity-heatmap-scroll w-full" scrollbars="horizontal">
						<div className="p-[0.35rem]">
							<div
								aria-label={ariaLabel}
								className="activity-heatmap"
								role="img"
								style={{
									"--activity-columns": columns,
									"--activity-min-width": `${minimumWidth}px`,
								} as CSSProperties}
							>
								{series.map((point, index) => {
									const future = point.startAt > now;
									const cell = getCell(point, index, future);
									return (
										<span
											aria-label={cell.title}
											className={`activity-cell ${future ? "activity-cell-future" : cell.className}`}
											key={point.startAt}
											title={cell.title}
										/>
									);
								})}
							</div>
						</div>
					</ScrollArea>
				) : <div className="visual-empty">当前范围暂无活动数据</div>}
			</div>
			<div className="activity-legend">{legend}</div>
		</Card>
	);
}

export function UsageLineCharts({ now, usage }: { now: number; usage: UsageDashboard }) {
	return (
		<div className="usage-line-grid">
			<LineTrendCard
				formatValue={formatTokens}
				now={now}
				primary={{
					color: "#4f7cff",
					dataKey: "primaryValue",
					name: "总 Token",
					value: (point) => point.totalTokens,
				}}
				secondary={{
					color: "#8b5cf6",
					dataKey: "secondaryValue",
					name: "缓存 Token",
					value: (point) => point.cachedInputTokens,
				}}
				series={usage.series}
				subtitle="总 Token 与缓存 Token 随时间变化"
				title="Token 趋势"
			/>
			<LineTrendCard
				formatValue={formatCost}
				now={now}
				primary={{
					color: "#14b8a6",
					dataKey: "primaryValue",
					name: "成本",
					value: (point) => point.costUsd,
				}}
				series={usage.series}
				subtitle="已配置模型价格覆盖的成本"
				title="成本趋势"
			/>
		</div>
	);
}

function LineTrendCard({
	formatValue,
	now,
	primary,
	secondary,
	series,
	subtitle,
	title,
}: {
	formatValue: (value: number) => string;
	now: number;
	primary: TrendLineDefinition;
	secondary?: TrendLineDefinition;
	series: UsageSeriesPoint[];
	subtitle: string;
	title: string;
}) {
	const lines = secondary ? [primary, secondary] : [primary];
	const chartData: TrendChartPoint[] = series.map((point) => ({
		...point,
		primaryValue: point.startAt <= now ? Math.max(0, primary.value(point)) : null,
		secondaryValue: point.startAt <= now && secondary ? Math.max(0, secondary.value(point)) : null,
	}));
	const actual = chartData.filter((point): point is TrendChartPoint & { primaryValue: number } => point.primaryValue !== null);
	const maximum = Math.max(
		1,
		...actual.flatMap((point) => lines.map((line) => point[line.dataKey] ?? 0)),
	);
	const total = actual.reduce((sum, point) => sum + point.primaryValue, 0);
	const firstAt = series[0]?.startAt ?? 0;
	const lastAt = series.at(-1)?.startAt ?? firstAt;
	const domainEnd = lastAt > firstAt ? lastAt : firstAt + 1;
	const ticks = firstAt === lastAt ? [firstAt] : [firstAt, lastAt];
	const latest = actual.at(-1);
	const futureStart = latest?.startAt ?? firstAt;
	return (
		<Card className="line-trend-card min-w-0 gap-3 p-3">
		<header>
			<div><h3>{title}</h3><p>{subtitle}</p></div>
			<strong style={{ color: primary.color }}>{formatValue(total)}</strong>
		</header>
		{series.length > 0 ? (
			<div className="trend-chart">
				<ResponsiveContainer height="100%" minWidth={0} width="100%">
					<LineChart
						accessibilityLayer
						aria-label={`${title}折线图`}
						data={chartData}
						margin={{ top: secondary ? 2 : 15, right: 12, bottom: 0, left: 4 }}
						role="img"
					>
						{secondary ? (
							<Legend
								align="right"
								height={24}
								iconSize={14}
								iconType="plainline"
								verticalAlign="top"
								wrapperStyle={{ color: "var(--text-secondary)", fontSize: "0.62rem" }}
							/>
						) : null}
						<CartesianGrid stroke="var(--border)" strokeDasharray="3 4" vertical={false} />
						<XAxis
							axisLine={false}
							dataKey="startAt"
							domain={[firstAt, domainEnd]}
							height={24}
							tick={{ fill: "var(--text-tertiary)", fontSize: 9 }}
							tickFormatter={formatShortDate}
							tickLine={false}
							tickMargin={9}
							ticks={ticks}
							type="number"
						/>
						<YAxis domain={[0, maximum]} hide ticks={[0, maximum / 2, maximum]} />
						{futureStart < lastAt ? (
							<ReferenceArea fill="var(--border)" fillOpacity={0.32} stroke="none" x1={futureStart} x2={lastAt} />
						) : null}
						{lines.map((line) => (
							<Line
								activeDot={{ fill: line.color, r: 4, stroke: "var(--surface)", strokeWidth: 2 }}
								connectNulls={false}
								dataKey={line.dataKey}
								dot={false}
								isAnimationActive={false}
								key={line.dataKey}
								name={line.name}
								stroke={line.color}
								strokeWidth={2.4}
								type="monotone"
							/>
						))}
						{latest ? lines.map((line) => {
							const lineValue = latest[line.dataKey];
							return lineValue === null ? null : (
								<ReferenceDot fill={line.color} key={line.dataKey} r={3.4} stroke="none" x={latest.startAt} y={lineValue} />
							);
						}) : null}
						<Tooltip
							content={(props) => <TrendChartTooltip {...props} formatValue={formatValue} lines={lines} />}
							cursor={{ stroke: "var(--border-strong)", strokeDasharray: "3 3" }}
						/>
					</LineChart>
				</ResponsiveContainer>
			</div>
		) : <div className="visual-empty">当前范围暂无趋势数据</div>}
	</Card>
	);
}

function TrendChartTooltip({
	active,
	formatValue,
	lines,
	payload,
}: TooltipContentProps & {
	formatValue: (value: number) => string;
	lines: TrendLineDefinition[];
}) {
	const point = payload[0]?.payload as TrendChartPoint | undefined;
	if (!active || !point || point.primaryValue === null) return null;
	return (
		<div className="chart-tooltip">
			<strong>{formatDateTime(point.startAt)}</strong>
			<div className="chart-tooltip-series-list">
				{lines.map((line) => {
					const value = point[line.dataKey];
					return value === null ? null : (
						<span className="chart-tooltip-series" key={line.dataKey}>
							<i style={{ background: line.color }} />
							<b>{line.name}</b>
							<em>{formatValue(value)}</em>
						</span>
					);
				})}
			</div>
			<small>{formatCount(point.requests)} 次请求</small>
		</div>
	);
}

export function UsageBreakdownDonuts({ usage }: { usage: UsageDashboard }) {
	return (
		<div className="usage-donut-grid">
			<DonutBreakdownCard rows={modelRows(usage.models)} title="模型用量" />
			<DonutBreakdownCard rows={identityRows(usage.identities)} title="身份用量" />
		</div>
	);
}

export function ModelTokenDonut({ usage }: { usage: UsageDashboard }) {
	return (
		<DonutBreakdownCard
			fixedMetric="tokens"
			rows={modelRows(usage.models)}
			subtitle="各模型在所选范围总 Token 中的占比"
			title="模型 Token 占比"
		/>
	);
}

export function DownstreamCostDonut({ usage }: { usage: UsageDashboard }) {
	return (
		<DonutBreakdownCard
			fixedMetric="cost"
			rows={identityRows(usage.identities)}
			split
			subtitle="各下游身份在所选范围成本中的占比"
			title="下游成本分布"
		/>
	);
}

function DonutBreakdownCard({
	fixedMetric,
	rows,
	split = false,
	subtitle,
	title,
}: {
	fixedMetric?: DonutMetric;
	rows: DonutRow[];
	split?: boolean;
	subtitle?: string;
	title: string;
}) {
	const [selectedMetric, setSelectedMetric] = useState<DonutMetric>("tokens");
	const metric = fixedMetric ?? selectedMetric;
	const values = rows.map((row) => metric === "tokens" ? row.tokens : row.cost);
	const total = values.reduce((sum, value) => sum + Math.max(0, value), 0);
	const chartRows = rows.flatMap((row, index): DonutChartRow[] => {
		const amount = values[index] ?? 0;
		if (!(amount > 0) || !(total > 0)) return [];
		return [{
			...row,
			amount,
			color: DONUT_COLORS[index % DONUT_COLORS.length] ?? DONUT_COLORS[0]!,
			percent: amount / total * 100,
		}];
	});
	return (
		<Card className={`donut-card grid min-w-0 gap-3 p-3${split ? " donut-card-split" : ""}`}>
		<header>
			<div>
				<h3>{title}</h3>
				<p>{subtitle ?? "按 Token 或成本查看占比"}</p>
			</div>
			{fixedMetric ? null : (
				<Select onValueChange={(value) => setSelectedMetric(value as DonutMetric)} value={metric}>
					<SelectTrigger aria-label={`${title}统计指标`} size="sm">
						<SelectValue />
					</SelectTrigger>
					<SelectContent align="end" position="popper">
						<SelectGroup>
							<SelectItem value="tokens">Token</SelectItem>
							<SelectItem value="cost">成本</SelectItem>
						</SelectGroup>
					</SelectContent>
				</Select>
			)}
		</header>
		{total > 0 ? (
			<div className="donut-content">
				<div className="donut-figure">
					<div className="donut-chart">
						<ResponsiveContainer height="100%" minWidth={0} width="100%">
							<PieChart
								accessibilityLayer
								aria-label={`${title}${metric === "tokens" ? "Token" : "成本"}占比圆环图`}
								margin={{ top: 0, right: 0, bottom: 0, left: 0 }}
								role="img"
							>
								<Pie
									data={[{ amount: total }]}
									dataKey="amount"
									endAngle={-270}
									fill="var(--border)"
									innerRadius="58%"
									isAnimationActive={false}
									outerRadius="78%"
									startAngle={90}
									stroke="none"
									tooltipType="none"
								/>
								<Pie
									data={chartRows}
									dataKey="amount"
									endAngle={-270}
									innerRadius="58%"
									isAnimationActive={false}
									nameKey="label"
									outerRadius="78%"
									startAngle={90}
									stroke="none"
								>
									{chartRows.map((row) => <Cell fill={row.color} key={row.id} />)}
								</Pie>
								<Tooltip
									content={(props) => <DonutChartTooltip {...props} metric={metric} />}
									cursor={false}
								/>
							</PieChart>
						</ResponsiveContainer>
						<div className="donut-center"><span>{metric === "tokens" ? "总 Token" : "总成本"}</span><strong>{metric === "tokens" ? formatTokens(total) : formatCost(total)}</strong></div>
					</div>
				</div>
				<div className="donut-legend">
					{chartRows.map((row) => (
						<div className="donut-legend-row" key={row.id}>
							<i style={{ background: row.color }} />
							<div><strong title={row.label}>{row.label}</strong><span>{row.meta}</span></div>
							<b>{formatPercent(row.percent)}</b>
							<small>{metric === "tokens" ? formatTokens(row.amount) : formatCost(row.amount)}</small>
						</div>
					))}
				</div>
			</div>
		) : (
			<div className="visual-empty donut-empty">
				{metric === "cost" ? "配置模型价格后显示成本占比" : "当前范围暂无可展示的用量"}
			</div>
		)}
	</Card>
	);
}

function DonutChartTooltip({
	active,
	metric,
	payload,
}: TooltipContentProps & { metric: DonutMetric }) {
	const row = payload[0]?.payload as DonutChartRow | undefined;
	if (!active || !row) return null;
	return (
		<div className="chart-tooltip">
			<strong>{row.label}</strong>
			<span>{metric === "tokens" ? formatTokens(row.amount) : formatCost(row.amount)}</span>
			<small>{formatPercent(row.percent)} · {row.meta}</small>
		</div>
	);
}

function modelRows(rows: UsageModelRow[]): DonutRow[] {
	return rows.map((row) => ({
		id: row.model,
		label: row.model,
		meta: `${formatCount(row.requests)} 次请求`,
		tokens: row.totalTokens,
		cost: row.costUsd,
	}));
}

function identityRows(rows: UsageIdentityRow[]): DonutRow[] {
	return rows.map((row) => ({
		id: `${row.identityType}:${row.identityId}`,
		label: row.identityName,
		meta: row.identityType === "auth_proxy" ? "Account ID" : "API Key",
		tokens: row.totalTokens,
		cost: row.costUsd,
	}));
}

function activityLevels(values: number[]): number[] {
	const positive = values.filter((value) => value > 0).sort((left, right) => left - right);
	if (positive.length === 0) return values.map(() => 0);
	const low = positive[Math.max(0, Math.ceil(positive.length * 0.05) - 1)] ?? 0;
	const high = positive[Math.max(0, Math.ceil(positive.length * 0.95) - 1)] ?? low;
	if (low === high) return values.map((value) => value > 0 ? 5 : 0);
	const logLow = Math.log1p(low);
	const logRange = Math.log1p(high) - logLow;
	return values.map((value) => {
		if (!(value > 0)) return 0;
		const ratio = (Math.log1p(Math.min(high, Math.max(low, value))) - logLow) / logRange;
		return Math.max(1, Math.min(5, 1 + Math.floor(ratio * 5)));
	});
}

function healthLevel(success: number, failed: number): number {
	const total = Math.max(0, success) + Math.max(0, failed);
	if (total === 0) return 0;
	const rate = Math.max(0, success) / total;
	if (rate < 0.5) return 1;
	if (rate < 0.65) return 2;
	if (rate < 0.8) return 3;
	if (rate < 0.95) return 4;
	return 5;
}

function formatPercent(value: number): string {
	return `${value < 0.1 ? "<0.1" : value.toFixed(1)}%`;
}

function formatTokens(value: number): string {
	const normalized = Math.max(0, Number.isFinite(value) ? value : 0);
	return normalized < 10_000 ? INTEGER_FORMAT.format(normalized) : COMPACT_FORMAT.format(normalized);
}

function formatCount(value: number): string {
	return INTEGER_FORMAT.format(Math.max(0, Number.isFinite(value) ? value : 0));
}

function formatDateTime(value: number): string {
	return new Intl.DateTimeFormat("zh-CN", {
		month: "2-digit",
		day: "2-digit",
		hour: "2-digit",
		minute: "2-digit",
		hour12: false,
	}).format(new Date(value));
}

function formatShortDate(value: number): string {
	return new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit" }).format(new Date(value));
}
