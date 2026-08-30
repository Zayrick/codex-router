import { useId, useMemo, useState, type CSSProperties } from "react";
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

function TokenActivityCard({ now, usage }: { now: number; usage: UsageDashboard }) {
	const levels = useMemo(
		() => activityLevels(usage.series.map((point) => point.totalTokens)),
		[usage.series],
	);
	return (
		<ActivityCard
			ariaLabel="周期 Token 活动图"
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
			subtitle="按周期时间格显示 Token 消耗，未来时间保持为空"
			summary={formatTokens(usage.totals.totalTokens)}
			summaryDetail={`输入 ${formatTokens(usage.totals.inputTokens)} · 输出 ${formatTokens(usage.totals.outputTokens)}`}
			title="Token 活动"
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
	return (
		<article className={`usage-activity-card ${className}`}>
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
					<div className="activity-heatmap-scroll">
						<div
							aria-label={ariaLabel}
							className="activity-heatmap"
							role="img"
							style={{ "--activity-columns": columns } as CSSProperties}
						>
							{series.map((point, index) => {
								const future = point.startAt > now;
								const cell = getCell(point, index, future);
								return (
									<span
										aria-label={cell.title}
										className={`activity-cell ${cell.className}${future ? " activity-cell-future" : ""}`}
										key={point.startAt}
										title={cell.title}
									/>
								);
							})}
						</div>
					</div>
				) : <div className="visual-empty">当前范围暂无活动数据</div>}
			</div>
			<div className="activity-legend">{legend}</div>
		</article>
	);
}

export function UsageLineCharts({ now, usage }: { now: number; usage: UsageDashboard }) {
	return (
		<div className="usage-line-grid">
			<LineTrendCard
				color="#4f7cff"
				formatValue={formatTokens}
				now={now}
				series={usage.series}
				subtitle="总 Token 随时间变化"
				title="Token 趋势"
				value={(point) => point.totalTokens}
			/>
			<LineTrendCard
				color="#14b8a6"
				formatValue={formatCost}
				now={now}
				series={usage.series}
				subtitle="已配置模型价格覆盖的成本"
				title="成本趋势"
				value={(point) => point.costUsd}
			/>
		</div>
	);
}

function LineTrendCard({
	color,
	formatValue,
	now,
	series,
	subtitle,
	title,
	value,
}: {
	color: string;
	formatValue: (value: number) => string;
	now: number;
	series: UsageSeriesPoint[];
	subtitle: string;
	title: string;
	value: (point: UsageSeriesPoint) => number;
}) {
	const gradientId = `trend-${useId().replaceAll(":", "")}`;
	const width = 600;
	const height = 180;
	const left = 18;
	const right = 12;
	const top = 15;
	const bottom = 28;
	const chartWidth = width - left - right;
	const chartHeight = height - top - bottom;
	const actual = series
		.map((point, index) => ({ point, index, value: Math.max(0, value(point)) }))
		.filter((entry) => entry.point.startAt <= now);
	const maximum = Math.max(1, ...actual.map((entry) => entry.value));
	const denominator = Math.max(1, series.length - 1);
	const coordinates = actual.map((entry) => ({
		x: left + entry.index / denominator * chartWidth,
		y: top + (1 - entry.value / maximum) * chartHeight,
		value: entry.value,
	}));
	const linePath = coordinates.map((point, index) => `${index === 0 ? "M" : "L"}${point.x.toFixed(2)},${point.y.toFixed(2)}`).join(" ");
	const areaPath = coordinates.length > 0
		? `${linePath} L${coordinates.at(-1)!.x.toFixed(2)},${top + chartHeight} L${coordinates[0]!.x.toFixed(2)},${top + chartHeight} Z`
		: "";
	const futureStart = coordinates.at(-1)?.x ?? left;
	const total = actual.reduce((sum, entry) => sum + entry.value, 0);
	return (
		<article className="line-trend-card">
		<header>
			<div><h3>{title}</h3><p>{subtitle}</p></div>
			<strong style={{ color }}>{formatValue(total)}</strong>
		</header>
		{series.length > 0 ? (
			<svg aria-label={`${title}折线图`} role="img" viewBox={`0 0 ${width} ${height}`}>
				<defs>
					<linearGradient id={gradientId} x1="0" x2="0" y1="0" y2="1">
						<stop offset="0%" stopColor={color} stopOpacity="0.26" />
						<stop offset="100%" stopColor={color} stopOpacity="0" />
					</linearGradient>
				</defs>
				{[0, 0.5, 1].map((ratio) => (
					<line className="trend-grid-line" key={ratio} x1={left} x2={width - right} y1={top + ratio * chartHeight} y2={top + ratio * chartHeight} />
				))}
				{futureStart < width - right ? <rect className="trend-future-area" height={chartHeight} width={width - right - futureStart} x={futureStart} y={top} /> : null}
				{areaPath ? <path d={areaPath} fill={`url(#${gradientId})`} /> : null}
				{linePath ? <path className="trend-line" d={linePath} fill="none" stroke={color} /> : null}
				{coordinates.length > 0 ? <circle cx={coordinates.at(-1)?.x} cy={coordinates.at(-1)?.y} fill={color} r="3.4" /> : null}
				<text className="trend-axis-label" x={left} y={height - 6}>{formatShortDate(series[0]!.startAt)}</text>
				<text className="trend-axis-label" textAnchor="end" x={width - right} y={height - 6}>{formatShortDate(series.at(-1)?.startAt ?? series[0]!.startAt)}</text>
			</svg>
		) : <div className="visual-empty">当前范围暂无趋势数据</div>}
	</article>
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

export function DownstreamCostDonut({ usage }: { usage: UsageDashboard }) {
	return (
		<DonutBreakdownCard
			fixedMetric="cost"
			rows={identityRows(usage.identities)}
			split
			subtitle="各下游身份在当前周期成本中的占比"
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
	const segments = donutSegments(values, total);
	return (
		<article className={`donut-card${split ? " donut-card-split" : ""}`}>
		<header>
			<div>
				<h3>{title}</h3>
				<p>{subtitle ?? "按 Token 或成本查看占比"}</p>
			</div>
			{fixedMetric ? null : (
				<select aria-label={`${title}统计指标`} onChange={(event) => setSelectedMetric(event.target.value as DonutMetric)} value={metric}>
					<option value="tokens">Token</option>
					<option value="cost">成本</option>
				</select>
			)}
		</header>
		{total > 0 ? (
			<div className="donut-content">
				<div className="donut-figure">
					<div className="donut-chart">
						<svg aria-label={`${title}${metric === "tokens" ? "Token" : "成本"}占比圆环图`} role="img" viewBox="0 0 180 180">
							<circle className="donut-track" cx="90" cy="90" fill="none" r="62" strokeWidth="18" />
							{segments.map((segment) => (
								<path
									d={arcPath(90, 90, 62, segment.start, segment.end)}
									fill="none"
									key={segment.index}
									stroke={DONUT_COLORS[segment.index % DONUT_COLORS.length]}
									strokeLinecap="butt"
									strokeWidth="18"
								/>
							))}
						</svg>
						<div><span>{metric === "tokens" ? "总 Token" : "总成本"}</span><strong>{metric === "tokens" ? formatTokens(total) : formatCost(total)}</strong></div>
					</div>
				</div>
				<div className="donut-legend">
					{rows.map((row, index) => {
						const amount = values[index] ?? 0;
						if (!(amount > 0)) return null;
						return (
							<div className="donut-legend-row" key={row.id}>
								<i style={{ background: DONUT_COLORS[index % DONUT_COLORS.length] }} />
								<div><strong title={row.label}>{row.label}</strong><span>{row.meta}</span></div>
								<b>{amount / total * 100 < 0.1 ? "<0.1" : (amount / total * 100).toFixed(1)}%</b>
								<small>{metric === "tokens" ? formatTokens(amount) : formatCost(amount)}</small>
							</div>
						);
					})}
				</div>
			</div>
		) : (
			<div className="visual-empty donut-empty">
				{metric === "cost" ? "配置模型价格后显示成本占比" : "当前范围暂无可展示的用量"}
			</div>
		)}
	</article>
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

function donutSegments(values: number[], total: number) {
	let cursor = -90;
	return values.flatMap((value, index) => {
		if (!(value > 0) || !(total > 0)) return [];
		const sweep = value / total * 360;
		const start = cursor;
		const end = cursor + sweep;
		cursor += sweep;
		return [{ index, start, end }];
	});
}

function arcPath(cx: number, cy: number, radius: number, start: number, end: number): string {
	const startPoint = polarPoint(cx, cy, radius, start);
	if (end - start >= 360 - Number.EPSILON * 360) {
		const oppositePoint = polarPoint(cx, cy, radius, start + 180);
		return `M ${startPoint.x} ${startPoint.y} A ${radius} ${radius} 0 1 1 ${oppositePoint.x} ${oppositePoint.y} A ${radius} ${radius} 0 1 1 ${startPoint.x} ${startPoint.y} Z`;
	}
	const endPoint = polarPoint(cx, cy, radius, end);
	return `M ${startPoint.x} ${startPoint.y} A ${radius} ${radius} 0 ${end - start > 180 ? 1 : 0} 1 ${endPoint.x} ${endPoint.y}`;
}

function polarPoint(cx: number, cy: number, radius: number, angle: number) {
	const radians = angle * Math.PI / 180;
	return { x: cx + radius * Math.cos(radians), y: cy + radius * Math.sin(radians) };
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
