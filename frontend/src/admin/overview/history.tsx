import { CheckCircle2Icon } from "lucide-react";

import type { UsageEvent } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { CodeChip } from "@/components/app/badges";
import { Heatmap, HeatmapLegend, type HeatmapRow } from "@/components/app/heatmap";
import { EMPTY_HEAT_COLOR, HEAT_BLUE, HEAT_HEALTH, healthColor, rampColor } from "@/components/app/heatmap-scale";
import { SectionCard } from "@/components/app/section-card";
import { STATUS_META } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { DAY_MS, formatClock, formatCount, formatDay, formatDuration, formatPercent, formatWeekday } from "@/lib/format";

import { FootChip } from "./activity";
import { failedCount, shortAccountName, successRate, type OverviewData } from "./data";

const FAILURE_ROWS = 5;

/** Result chip: HTTP status for failures, a label for interruptions and cancellations. */
function ResultChip({ event }: { event: UsageEvent }) {
	if (event.status === "interrupted") return <CodeChip tone="warning">中断</CodeChip>;
	if (event.status === "cancelled") return <CodeChip tone="neutral">取消</CodeChip>;
	if (event.status === "incomplete") return <CodeChip tone="warning">未完成</CodeChip>;
	return <CodeChip>{event.statusCode ?? "失败"}</CodeChip>;
}

function resultText(event: UsageEvent): string {
	if (event.errorCode) return event.errorCode;
	if (event.status === "cancelled") return "客户端取消请求";
	if (event.status === "interrupted") return `上游${event.transport === "websocket" ? " WebSocket" : ""} 断开`;
	return STATUS_META[event.status].label;
}

/** Figma "card · 最近失败": today's latest unsuccessful requests. */
export function FailuresCard({ data }: { data: OverviewData }) {
	const { navigate } = useAdmin();
	const today = data.today;
	const actions = (
		<Button onClick={() => navigate("logs", { view: "failed" })} size="sm" type="button" variant="ghost">
			调用日志 →
		</Button>
	);
	if (!today) {
		return (
			<SectionCard actions={actions} description={`今日最近 ${FAILURE_ROWS} 次未成功请求`} title="最近失败">
				<Skeleton className="h-52" />
			</SectionCard>
		);
	}
	const events = today.recentFailures.slice(0, FAILURE_ROWS);
	const totals = today.totals;
	const unsuccessful = failedCount(totals) + totals.cancelled;
	return (
		<SectionCard actions={actions} description={`今日最近 ${FAILURE_ROWS} 次未成功请求`} title="最近失败">
			{events.length === 0 ? (
				<Empty className="py-8">
					<EmptyHeader>
						<EmptyMedia variant="icon">
							<CheckCircle2Icon />
						</EmptyMedia>
						<EmptyTitle>今日没有失败请求</EmptyTitle>
					</EmptyHeader>
				</Empty>
			) : (
				<Table className="text-[12.5px]">
					<TableHeader>
						<TableRow className="hover:bg-transparent">
							<TableHead className="h-8 w-18 text-[11.5px] font-medium text-faint">时间</TableHead>
							<TableHead className="h-8 text-[11.5px] font-medium text-faint">用户 → 账户</TableHead>
							<TableHead className="h-8 text-[11.5px] font-medium text-faint">模型</TableHead>
							<TableHead className="h-8 text-[11.5px] font-medium text-faint">结果</TableHead>
							<TableHead className="h-8 w-14 text-right text-[11.5px] font-medium text-faint">耗时</TableHead>
						</TableRow>
					</TableHeader>
					<TableBody>
						{events.map((event) => (
							<TableRow className="cursor-pointer" key={event.id} onClick={() => navigate("logs", { q: event.requestId })}>
								<TableCell className="py-2 font-mono text-xs text-muted-foreground">{formatClock(event.recordedAt)}</TableCell>
								<TableCell className="py-2">
									<span className="font-medium">{event.userName}</span>
									{event.codexAccountName ? (
										<>
											<span className="mx-1 text-xs text-faint">→</span>
											<span className="text-muted-foreground">{shortAccountName(event.codexAccountName)}</span>
										</>
									) : null}
								</TableCell>
								<TableCell className="py-2 font-mono text-xs">{event.model || event.requestedModel || "—"}</TableCell>
								<TableCell className="py-2">
									<span className="flex items-center gap-2">
										<ResultChip event={event} />
										<span className="truncate font-mono text-xs text-muted-foreground">{resultText(event)}</span>
									</span>
								</TableCell>
								<TableCell className="py-2 text-right font-mono text-xs text-muted-foreground">{formatDuration(event.latencyMs)}</TableCell>
							</TableRow>
						))}
					</TableBody>
				</Table>
			)}
			<div className="flex flex-wrap items-center gap-2">
				<FootChip label="今日失败" tone="danger" value={formatCount(totals.failed)} />
				<FootChip label="中断" tone="warning" value={formatCount(totals.interrupted)} />
				<FootChip label="取消" value={formatCount(totals.cancelled)} />
				<FootChip label="其中 429" tone="danger" value={formatCount(totals.rateLimited)} />
				<span className="ml-auto text-[11.5px] text-muted-foreground tabular-nums">
					未成功 {formatCount(unsuccessful)} 次 · {formatPercent(totals.requests ? unsuccessful / totals.requests : null)}
				</span>
			</div>
		</SectionCard>
	);
}

/** Rows of the 7-day grid (oldest first); `null` marks hours not reached yet. */
function weekRows<T>(data: OverviewData, now: number, pick: (bucket: NonNullable<OverviewData["week"]>["series"][number]) => T): { rows: HeatmapRow[]; values: (T | null)[][] } {
	const week = data.week;
	const rows: HeatmapRow[] = [];
	const values: (T | null)[][] = [];
	if (!week) return { rows, values };
	for (let day = 0; day < 7; day += 1) {
		const dayStart = week.startAt + day * DAY_MS;
		const isToday = dayStart === data.midnight;
		const cells: (T | null)[] = [];
		for (let hour = 0; hour < 24; hour += 1) {
			const bucket = week.series[day * 24 + hour];
			const start = dayStart + hour * 3_600_000;
			cells.push(start > now || !bucket ? null : pick(bucket));
		}
		values.push(cells);
		rows.push({
			key: String(dayStart),
			label: `${isToday ? "今天" : formatWeekday(dayStart)} ${formatDay(dayStart)}`,
			current: isToday,
			cells: cells.map((cell) => (cell === null ? null : 0)),
		});
	}
	return { rows, values };
}

/** Figma "card · 近 7 天请求活跃度": requests per hour. */
export function WeekActivityCard({ data }: { data: OverviewData }) {
	const { now, navigate } = useAdmin();
	const { rows, values } = weekRows(data, now, (bucket) => bucket.requests);
	const max = Math.max(0, ...values.flat().map((value) => value ?? 0));
	const total = data.week?.totals.requests ?? 0;
	return (
		<SectionCard
			actions={
				<Button onClick={() => navigate("usage")} size="sm" type="button" variant="ghost">
					用量分析 →
				</Button>
			}
			description={`每格 1 小时 · 共 ${formatCount(total)} 请求`}
			title="近 7 天请求活跃度"
		>
			{!data.week ? (
				<Skeleton className="h-40" />
			) : (
				<>
					<Heatmap
						color={(_, row, column) => rampColor(values[row]?.[column] ?? 0, max, HEAT_BLUE)}
						rows={rows}
						title={(value, row, column) => (value === null ? "未到" : `${String(column).padStart(2, "0")}:00 · ${formatCount(values[row]?.[column] ?? 0)} 请求`)}
					/>
					<HeatmapLegend
						future
						items={[
							{ color: EMPTY_HEAT_COLOR, label: "无请求" },
							{ color: HEAT_BLUE[1], label: "少" },
							{ color: HEAT_BLUE[3], label: "多" },
						]}
					/>
				</>
			)}
		</SectionCard>
	);
}

/** Figma "card · 近 7 天健康度": hourly success rate. */
export function WeekHealthCard({ data }: { data: OverviewData }) {
	const { now, navigate } = useAdmin();
	const { rows, values } = weekRows(data, now, (bucket) => ({ rate: bucket.requests ? successRate(bucket) : null, failed: failedCount(bucket), limited: bucket.rateLimited }));
	let worst: { day: number; failed: number; limited: number } | null = null;
	values.forEach((cells, day) => {
		const failed = cells.reduce((sum, cell) => sum + (cell?.failed ?? 0), 0);
		const limited = cells.reduce((sum, cell) => sum + (cell?.limited ?? 0), 0);
		if (failed > 0 && (!worst || failed > worst.failed)) worst = { day, failed, limited };
	});
	const worstDay = worst as { day: number; failed: number; limited: number } | null;
	const week = data.week;
	const summary = worstDay && week ? `${formatDay(week.startAt + worstDay.day * DAY_MS)} 失败最多（${worstDay.failed} 次${worstDay.limited ? `，429 × ${worstDay.limited}` : ""}）` : "无失败";
	return (
		<SectionCard
			actions={
				<Button onClick={() => navigate("performance")} size="sm" type="button" variant="ghost">
					性能与可靠性 →
				</Button>
			}
			description={`每小时成功率 · ${summary}`}
			title="近 7 天健康度"
		>
			{!week ? (
				<Skeleton className="h-40" />
			) : (
				<>
					<Heatmap
						color={(_, row, column) => healthColor(values[row]?.[column]?.rate ?? null)}
						rows={rows}
						title={(value, row, column) => {
							if (value === null) return "未到";
							const cell = values[row]?.[column];
							return `${String(column).padStart(2, "0")}:00 · ${cell?.rate === null || cell === null || cell === undefined ? "无请求" : `成功率 ${formatPercent(cell.rate)}`}`;
						}}
					/>
					<HeatmapLegend
						items={[
							{ color: HEAT_HEALTH[0], label: "≥ 99%" },
							{ color: HEAT_HEALTH[1], label: "95–99%" },
							{ color: HEAT_HEALTH[2], label: "80–95%" },
							{ color: HEAT_HEALTH[3], label: "< 80%" },
							{ color: EMPTY_HEAT_COLOR, label: "无请求" },
						]}
					/>
				</>
			)}
		</SectionCard>
	);
}
