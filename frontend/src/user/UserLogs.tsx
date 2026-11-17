// User "调用日志" tab (Figma "User v2 / 调用日志", user-main 38:5793):
// toolbar, six-metric summary with distribution, and the request table with
// inline details.

import { useCallback, useMemo, useState, type ReactNode } from "react";
import { CopyIcon, DownloadIcon, FileJsonIcon, ListFilterIcon, RefreshCwIcon, ScrollTextIcon, SearchIcon } from "lucide-react";
import { toast } from "sonner";

import type { EventPageQuery, EventView, UsageEvent, UsageEventStatus } from "@/admin-api";
import type { ColumnKey } from "@/admin/logs/columns";
import { bucketLabel, copyText, generationMs, isDowngraded, outputSpeed, sessionText, tierText, transportText, upstreamName } from "@/admin/logs/event-utils";
import { RequestTable } from "@/admin/logs/request-table";
import { EMPTY_FILTERS, buildQuery, filtersFromParams, resolveRange, type LogFilters, type LogRange } from "@/admin/logs/state";
import { Distribution } from "@/admin/logs/summary";
import { PAGE_SIZE, sessionSummary, useLogs, useModelOptions } from "@/admin/logs/use-logs";
import { Legend, LegendItem, StackedBar } from "@/components/app/bars";
import { SectionCard } from "@/components/app/section-card";
import { Segmented } from "@/components/app/segmented";
import { SESSION_KIND_META, STATUS_META, STATUS_ORDER, TOKEN_PARTS, VIZ, seriesColor } from "@/components/app/viz";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { ScrollArea, ScrollBar } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { formatClock, formatCost, formatCount, formatDateTime, formatFullDateTime, formatPercent, formatSeconds, formatTokens, ratio, DAY_MS } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { UserApiClient } from "@/user-api";

const REFRESH_MS = 30_000;
const ALL = "__all__";
const COLUMNS: ReadonlySet<ColumnKey> = new Set(["model", "upstream", "session", "status", "latency", "tokens", "cost"]);
const RANGES: { value: LogRange; label: string }[] = [
	{ value: "1h", label: "1 小时" },
	{ value: "24h", label: "24 小时" },
	{ value: "7d", label: "7 天" },
	{ value: "30d", label: "30 天" },
];

/** Card-header views. */
type UserView = "all" | "failed" | "slow" | "rate_limited";

export default function UserLogs({
	api,
	now,
	onSessionExpired,
	initialParams,
}: {
	api: UserApiClient;
	now: number;
	onSessionExpired: () => void;
	/** Filters to apply (URL contract of src/admin/logs/state.ts); re-applied when the object changes. */
	initialParams?: URLSearchParams;
}) {
	const [filters, setFilters] = useState<LogFilters>(() => paramsFilters(initialParams).filters);
	const [search, setSearch] = useState(() => paramsFilters(initialParams).filters.search);
	const [view, setView] = useState<UserView>(() => paramsFilters(initialParams).view);
	const [appliedParams, setAppliedParams] = useState(initialParams);
	if (appliedParams !== initialParams) {
		setAppliedParams(initialParams);
		const next = paramsFilters(initialParams);
		setFilters(next.filters);
		setSearch(next.filters.search);
		setView(next.view);
	}
	const [anchor, setAnchor] = useState(() => Date.now());
	const [expandedId, setExpandedId] = useState<number | null>(null);

	const range = useMemo(() => resolveRange(filters, anchor), [anchor, filters]);
	const backendView: EventView = view;
	const query = useMemo(() => buildQuery({ ...filters, search }, range, { view: backendView, limit: PAGE_SIZE, grouped: false }), [backendView, filters, range, search]);
	const getEvents = useCallback((value: EventPageQuery, signal: AbortSignal) => api.getEvents(value, signal), [api]);
	const logs = useLogs(getEvents, query, { refreshMs: REFRESH_MS, onSessionExpired, onError: (message) => toast.error(message) });
	const data = logs.page;
	const models = useModelOptions(data?.models, filters.models, []);
	const modelColor = useCallback((model: string) => seriesColor(Math.max(0, models.indexOf(model))), [models]);
	const events = logs.events;

	const [expandedKey, setExpandedKey] = useState(JSON.stringify(query));
	if (expandedKey !== JSON.stringify(query)) {
		setExpandedKey(JSON.stringify(query));
		setExpandedId(null);
	}

	const set = (patch: Partial<LogFilters>) => setFilters((current) => ({ ...current, ...patch }));
	const filterSession = (sessionId: string) => {
		setView("all");
		set({ sessionId });
	};

	return (
		<div className="flex flex-col gap-4">
			<Card className="flex-row flex-wrap items-center justify-between gap-3 p-2">
				<div className="flex flex-wrap items-center gap-4">
					<Segmented label="时间范围" onChange={(value) => set({ range: value, custom: null })} options={RANGES} size="default" value={filters.range} />
					<div className="flex flex-col gap-0.5">
						<span className="text-[12.5px] leading-5 font-medium tabular-nums">
							{formatFullDateTime(range.startAt)} — {formatDateTime(Math.min(range.endAt, now))}
						</span>
						<span className="text-[11px] leading-4 text-faint">按时间倒序 · 每 30 秒自动刷新</span>
					</div>
				</div>
				<div className="flex flex-wrap items-center gap-2">
					<InputGroup className="w-[248px]">
						<InputGroupAddon>
							<SearchIcon />
						</InputGroupAddon>
						<InputGroupInput aria-label="搜索请求" onChange={(event) => setSearch(event.target.value)} placeholder="搜索会话 ID / 请求 ID" value={search} />
					</InputGroup>
					<SingleSelect
						allLabel="全部模型"
						onChange={(value) => set({ models: value ? [value] : [] })}
						options={models.map((model) => ({ value: model, label: model }))}
						value={filters.models[0] ?? ""}
					/>
					<SingleSelect
						allLabel="全部状态"
						onChange={(value) => set({ statuses: value ? [value as UsageEventStatus] : [] })}
						options={STATUS_ORDER.map((status) => ({ value: status, label: STATUS_META[status].label }))}
						value={filters.statuses[0] ?? ""}
					/>
					<SingleSelect
						allLabel="全部会话类型"
						onChange={(value) => set({ sessionKinds: value ? [value] : [] })}
						options={Object.entries(SESSION_KIND_META).map(([value, meta]) => ({ value, label: meta.label }))}
						value={filters.sessionKinds[0] ?? ""}
					/>
					<Button
						aria-label="刷新"
						disabled={logs.loading}
						onClick={() => {
							setAnchor(Date.now());
							logs.reload();
						}}
						size="icon"
						type="button"
						variant="outline"
					>
						{logs.loading ? <Spinner /> : <RefreshCwIcon />}
					</Button>
					<Button asChild type="button" variant="outline">
						<a download href={api.eventsCsvUrl(query)}>
							<DownloadIcon data-icon="inline-start" />
							导出 CSV
						</a>
					</Button>
				</div>
			</Card>

			{logs.error ? (
				<Alert variant="destructive">
					<AlertDescription>{logs.error}</AlertDescription>
				</Alert>
			) : null}

			<UserSummary data={data} now={now} onSelectRange={(custom) => set({ range: "custom", custom })} rangeLength={range.endAt - range.startAt} />

			<SectionCard
				actions={
					<Segmented<UserView>
						label="视图"
						onChange={setView}
						options={[
							{ value: "all", label: "全部" },
							{ value: "failed", label: "失败" },
							{ value: "slow", label: "慢请求" },
							{ value: "rate_limited", label: "限流 429" },
						]}
						value={view}
					/>
				}
				description={filters.sessionId ? `会话 ${filters.sessionId} · 点击行查看详情` : "点击行查看详情"}
				title="请求明细"
			>
				{filters.sessionId ? (
					<div className="-mt-2 flex items-center gap-2 text-xs text-muted-foreground">
						<ListFilterIcon className="size-3.5" />
						只看此会话
						<Button onClick={() => set({ sessionId: "" })} size="xs" type="button" variant="link">
							取消
						</Button>
					</div>
				) : null}
				{!data ? (
					<div className="flex flex-col gap-2">
						{Array.from({ length: 6 }, (_, index) => (
							<Skeleton className="h-12" key={index} />
						))}
					</div>
				) : events.length === 0 ? (
					<Empty className="border border-dashed py-10">
						<EmptyHeader>
							<EmptyMedia variant="icon">
								<ScrollTextIcon />
							</EmptyMedia>
							<EmptyTitle>没有匹配的请求</EmptyTitle>
							<EmptyDescription>调整时间范围或筛选条件后再试。</EmptyDescription>
						</EmptyHeader>
					</Empty>
				) : (
					<ScrollArea className="-mx-6 px-6">
						<RequestTable
							columnLabels={{ upstream: "上游账户", session: "会话" }}
							columns={COLUMNS}
							events={events}
							expandedId={expandedId}
							modelColor={modelColor}
							now={now}
							onOpen={() => undefined}
							onSessionFilter={filterSession}
							onToggle={(id) => setExpandedId((current) => (current === id ? null : id))}
							renderExpanded={(event) => <InlineDetail event={event} onSessionFilter={filterSession} session={sessionSummary(event, logs.sessionCounts)} />}
							sessionSummary={(event) => sessionSummary(event, logs.sessionCounts)}
							slowThresholdMs={data.slowThresholdMs}
						/>
						<ScrollBar orientation="horizontal" />
					</ScrollArea>
				)}
				{data && events.length > 0 ? (
					<div className="flex items-center justify-between gap-3 pt-1">
						<span className="text-[12.5px] text-muted-foreground tabular-nums">
							显示 {formatCount(events.length)} / {formatCount(data.total)} 条
						</span>
						{logs.canLoadMore ? (
							<Button disabled={logs.loadingMore} onClick={logs.loadMore} size="sm" type="button" variant="outline">
								{logs.loadingMore ? <Spinner data-icon="inline-start" /> : null}
								加载更多
							</Button>
						) : null}
					</div>
				) : null}
			</SectionCard>
		</div>
	);
}

/** Applies the logs URL contract; the user tab defaults to 7 days. */
function paramsFilters(params: URLSearchParams | undefined): { filters: LogFilters; view: UserView } {
	if (!params) return { filters: { ...EMPTY_FILTERS, range: "7d" }, view: "all" };
	const parsed = filtersFromParams(params);
	const filters = { ...parsed.filters, downstream: null, upstream: null, ...(params.has("range") ? {} : { range: "7d" as LogRange }) };
	return { filters, view: parsed.view === "failed" || parsed.view === "slow" || parsed.view === "rate_limited" ? parsed.view : "all" };
}

function SingleSelect({ value, onChange, options, allLabel }: { value: string; onChange: (value: string) => void; options: { value: string; label: string }[]; allLabel: string }) {
	return (
		<Select onValueChange={(next) => onChange(next === ALL ? "" : next)} value={value || ALL}>
			<SelectTrigger aria-label={allLabel} className="h-8 min-w-24">
				<SelectValue />
			</SelectTrigger>
			<SelectContent position="popper">
				<SelectItem value={ALL}>{allLabel}</SelectItem>
				{options.map((option) => (
					<SelectItem key={option.value} value={option.value}>
						{option.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

/** Figma "logs-summary": six metrics and the request distribution. */
function UserSummary({
	data,
	now,
	rangeLength,
	onSelectRange,
}: {
	data: ReturnType<typeof useLogs>["page"];
	now: number;
	rangeLength: number;
	onSelectRange: (range: { startAt: number; endAt: number }) => void;
}) {
	if (!data) {
		return (
			<Card className="gap-4 p-6">
				<div className="grid grid-cols-2 gap-4 md:grid-cols-6">
					{Array.from({ length: 6 }, (_, index) => (
						<Skeleton className="h-16" key={index} />
					))}
				</div>
				<Skeleton className="h-20" />
			</Card>
		);
	}
	const summary = data.summary;
	const success = summary.completed + summary.incomplete;
	const successRate = ratio(success, summary.requests - summary.cancelled);
	const days = Math.max(1, rangeLength / DAY_MS);
	const failures = summary.failed + summary.interrupted;
	return (
		<Card className="gap-4 px-6 pt-4 pb-4">
			<div className="grid grid-cols-2 gap-y-4 md:grid-cols-3 xl:grid-cols-6">
				<Metric detail={rangeLength > DAY_MS ? `日均 ${formatCount(summary.requests / days)}` : `${formatCount(summary.sessions)} 个会话`} label="请求" value={formatCount(summary.requests)} />
				<Metric
					className={successRate !== null && successRate < 0.95 ? "text-warning" : "text-success"}
					detail={`已完成 ${formatCount(summary.completed)}`}
					label="成功率"
					value={formatPercent(successRate)}
				/>
				<Metric
					className={failures ? "text-danger" : undefined}
					detail={`429 × ${summary.rateLimited} · 取消 ${summary.cancelled}`}
					label="失败 / 异常"
					value={formatCount(failures)}
				/>
				<Metric detail={`缓存命中 ${formatPercent(ratio(summary.cachedInputTokens, summary.inputTokens))}`} label="Token" value={formatTokens(summary.totalTokens)} />
				<Metric detail={`每请求 ${formatCost(summary.requests ? summary.costUsd / summary.requests : null)}`} label="成本" value={formatCost(summary.costUsd)} />
				<Metric
					detail={`总延迟 P95 ${formatSeconds(summary.latency.p95, 1)}`}
					label="首字 P50 / P95"
					value={`${formatSeconds(summary.ttft.p50)} / ${formatSeconds(summary.ttft.p95, 1)}`}
				/>
			</div>
			<div className="flex flex-col gap-2 border-t pt-4">
				<div className="flex flex-wrap items-center gap-x-4 gap-y-1">
					<span className="text-[12.5px] leading-5 font-bold">请求分布</span>
					<Legend className="flex-1">
						<LegendItem color={VIZ.blue} label="已完成" value={formatCount(summary.completed)} />
						<LegendItem color={VIZ.danger} label="失败" value={formatCount(summary.failed)} />
						<LegendItem color={VIZ.muted} label="取消" value={formatCount(summary.cancelled)} />
						<LegendItem color={VIZ.warning} label="中断 / 未完成" value={formatCount(summary.interrupted + summary.incomplete)} />
					</Legend>
					<span className="text-[11.5px] text-faint">每 {bucketLabel(data.bucketMs)} · 拖选可缩小范围</span>
				</div>
				<Distribution bucketMs={data.bucketMs} now={now} onSelectRange={onSelectRange} series={data.series} />
			</div>
		</Card>
	);
}

function Metric({ label, value, detail, className }: { label: string; value: string; detail: string; className?: string | undefined }) {
	return (
		<div className="flex min-w-0 flex-col gap-0.5 pr-4 xl:border-l xl:pl-4 xl:first:border-l-0 xl:first:pl-0">
			<span className="text-xs leading-4 text-muted-foreground">{label}</span>
			<span className={cn("text-[22px] leading-7 font-bold tracking-[-0.03em] tabular-nums", className)}>{value}</span>
			<span className="truncate text-xs leading-4 text-faint">{detail}</span>
		</div>
	);
}

/** Figma "log-detail": timing, token breakdown, request fields and actions. */
function InlineDetail({ event, session, onSessionFilter }: { event: UsageEvent; session: { total: number; text: string } | null; onSessionFilter: (id: string) => void }) {
	const generation = generationMs(event);
	const speed = outputSpeed(event);
	const startedAt = event.recordedAt - event.latencyMs;
	const uncached = Math.max(0, event.inputTokens - event.cachedInputTokens);
	const output = Math.max(0, event.outputTokens - event.reasoningOutputTokens);
	const share = (value: number) => formatPercent(ratio(value, event.totalTokens));
	const sessionId = event.parentSessionId || event.sessionId;
	return (
		<div className="flex flex-col gap-5 px-2 pt-2 pb-1">
			<Block
				aside={
					<span className="flex flex-wrap gap-3">
						<Pair label="首字" value={formatSeconds(event.ttftMs, 1)} />
						<Pair label="生成" value={formatSeconds(generation, 1)} />
						<Pair label="总计" value={formatSeconds(event.latencyMs, 1)} />
						{speed !== null ? <Pair label="输出速度" value={`${speed.toFixed(1)} t/s`} /> : null}
					</span>
				}
				title="耗时"
			>
				<StackedBar
					segments={[
						{ key: "ttft", value: event.ttftMs ?? 0, color: VIZ.blue },
						{ key: "gen", value: generation ?? event.latencyMs, color: VIZ.blueSoft },
					]}
				/>
				<div className="flex flex-wrap justify-between gap-2 text-[11px] leading-4">
					<Stamp at={startedAt} label="发出请求" />
					{event.ttftMs !== null ? <Stamp at={startedAt + event.ttftMs} label="首个 Token" /> : null}
					<Stamp at={event.recordedAt} label={`完成 · ${event.status}`} />
				</div>
			</Block>
			<Block
				aside={
					<span className="text-[11.5px] text-faint">
						合计 <span className="font-mono text-xs font-bold text-foreground">{formatCount(event.totalTokens)}</span> · 成本 {event.totalTokens ? formatCost(event.costUsd) : "—"}
					</span>
				}
				title="Token 明细"
			>
				<StackedBar
					segments={[
						{ key: "input", value: uncached, color: TOKEN_PARTS.input.color },
						{ key: "cached", value: event.cachedInputTokens, color: TOKEN_PARTS.cached.color },
						{ key: "output", value: output, color: TOKEN_PARTS.output.color },
						{ key: "reasoning", value: event.reasoningOutputTokens, color: TOKEN_PARTS.reasoning.color },
					]}
				/>
				<div className="grid grid-cols-2 gap-2 sm:grid-cols-5">
					<TokenCard color={TOKEN_PARTS.input.color} detail={share(uncached)} label="未缓存输入" value={uncached} />
					<TokenCard color={TOKEN_PARTS.cached.color} detail={`命中 ${formatPercent(ratio(event.cachedInputTokens, event.inputTokens))}`} label="缓存读取" value={event.cachedInputTokens} />
					<TokenCard color="var(--border-strong)" detail="—" label="缓存写入" value={event.cacheCreationInputTokens} />
					<TokenCard color={TOKEN_PARTS.output.color} detail={share(output)} label="输出（不含推理）" value={output} />
					<TokenCard color={TOKEN_PARTS.reasoning.color} detail={share(event.reasoningOutputTokens)} label="推理输出" value={event.reasoningOutputTokens} />
				</div>
			</Block>
			<Block title="请求字段">
				<div className="grid grid-cols-2 gap-x-4 gap-y-3 md:grid-cols-4">
					<Field action={<IconAction label="复制请求 ID" onClick={() => copyText(event.requestId, "请求 ID")} icon={<CopyIcon />} />} label="请求 ID" mono value={event.requestId} />
					<Field action={<IconAction label="复制响应 ID" onClick={() => copyText(event.responseId, "响应 ID")} icon={<CopyIcon />} />} label="响应 ID" mono value={event.responseId} />
					<Field
						action={event.sessionId ? <IconAction label="只看此会话" onClick={() => onSessionFilter(event.sessionId)} icon={<ListFilterIcon />} /> : null}
						label="会话"
						mono
						value={sessionText(event.sessionId, event.sessionKind)}
					/>
					<Field
						action={event.parentSessionId ? <IconAction label="只看父会话" onClick={() => onSessionFilter(event.parentSessionId)} icon={<ListFilterIcon />} /> : null}
						label="父会话"
						mono
						value={sessionText(event.parentSessionId, event.parentSessionId ? "main" : "")}
					/>
					<Field label="客户端" mono value={event.userAgent} />
					<Field label="客户端 IP" mono value={event.clientIp} />
					<Field label="端点" mono value={event.endpoint} />
					<Field label="传输" value={transportText(event)} />
					<Field label="模型" mono value={event.model} />
					<Field label="推理强度" mono value={event.reasoningEffort} />
					<Field
						className={isDowngraded(event) ? "font-semibold text-warning" : undefined}
						label="速度档位"
						value={isDowngraded(event) ? `请求 ${tierText(event).replace("→", " → 实际 ")}` : tierText(event)}
					/>
					<Field label="上游账户" value={event.codexAccountName ? `${event.codexAccountName} · ${upstreamName(event)}` : ""} />
				</div>
			</Block>
			<div className="flex flex-wrap items-center gap-2">
				{sessionId ? (
					<Button onClick={() => onSessionFilter(sessionId)} size="sm" type="button" variant="outline">
						<ListFilterIcon data-icon="inline-start" />
						只看此会话{session ? ` · ${session.total}` : ""}
					</Button>
				) : null}
				<Button disabled={!event.requestId} onClick={() => copyText(event.requestId, "请求 ID")} size="sm" type="button" variant="outline">
					<CopyIcon data-icon="inline-start" />
					复制请求 ID
				</Button>
				<Button onClick={() => copyText(JSON.stringify(event, null, 2), "JSON")} size="sm" type="button" variant="ghost">
					<FileJsonIcon data-icon="inline-start" />
					复制为 JSON
				</Button>
				{session?.text ? <span className="ml-auto text-[11.5px] text-faint">同会话：{session.text}</span> : null}
			</div>
		</div>
	);
}

function Block({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
	return (
		<section className="flex flex-col gap-2">
			<div className="flex flex-wrap items-center justify-between gap-2">
				<h4 className="text-xs leading-4 font-bold">{title}</h4>
				{aside}
			</div>
			{children}
		</section>
	);
}

function Pair({ label, value }: { label: string; value: string }) {
	return (
		<span className="flex items-baseline gap-1">
			<span className="text-[11.5px] text-faint">{label}</span>
			<span className="text-xs font-semibold tabular-nums">{value}</span>
		</span>
	);
}

function Stamp({ at, label }: { at: number; label: string }) {
	return (
		<span className="flex gap-1.5">
			<span className="font-mono text-muted-foreground">
				{formatClock(at)}.{String(Math.floor(at % 1000)).padStart(3, "0")}
			</span>
			<span className="text-faint">{label}</span>
		</span>
	);
}

function TokenCard({ color, label, value, detail }: { color: string; label: string; value: number; detail: string }) {
	return (
		<div className="flex min-w-0 flex-col gap-0.5 rounded-lg border bg-card px-3 py-2">
			<span className="flex items-center gap-1.5 text-[11px] leading-4 text-muted-foreground">
				<span aria-hidden="true" className="size-2 shrink-0 rounded-[2px]" style={{ background: color }} />
				{label}
			</span>
			<span className="font-mono text-sm leading-5 font-bold tabular-nums">{formatCount(value)}</span>
			<span className="text-[11px] leading-4 text-faint">{detail}</span>
		</div>
	);
}

function Field({ label, value, mono = false, action, className }: { label: string; value: string; mono?: boolean; action?: ReactNode; className?: string | undefined }) {
	return (
		<div className="flex min-w-0 flex-col gap-1">
			<span className="text-[11px] leading-4 text-faint">{label}</span>
			<div className="flex items-center gap-1.5">
				<span className={cn("min-w-0 flex-1 text-xs leading-4 break-all", mono && "font-mono", !value && "text-faint", className)}>{value || "—"}</span>
				{action}
			</div>
		</div>
	);
}

function IconAction({ label, icon, onClick }: { label: string; icon: ReactNode; onClick: () => void }) {
	return (
		<Button aria-label={label} className="size-5 text-muted-foreground [&_svg]:size-3" onClick={onClick} size="icon-xs" title={label} type="button" variant="ghost">
			{icon}
		</Button>
	);
}
