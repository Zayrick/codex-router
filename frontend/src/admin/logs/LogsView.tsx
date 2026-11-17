import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { Columns3Icon, ScrollTextIcon } from "lucide-react";

import type { EventPageQuery, EventView, UsageEvent, UsageEventPage } from "@/admin-api";
import { SectionCard } from "@/components/app/section-card";
import { Segmented } from "@/components/app/segmented";
import { seriesColor } from "@/components/app/viz";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuCheckboxItem,
	DropdownMenuContent,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { ScrollArea, ScrollBar } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { formatClock, formatCount, formatDay, startOfLocalDay } from "@/lib/format";
import { randomUuid } from "@/lib/uuid";

import { toast } from "sonner";

import { RequestSheet } from "./detail-sheet";
import { LogFilterBar, type FilterOptions } from "./filter-bar";
import { COLUMNS, type ColumnKey } from "./columns";
import { RequestTable } from "./request-table";
import { LogSummary } from "./summary";
import { SessionLegend, SessionTable } from "./session-table";
import { PAGE_SIZE, sessionSummary, useLogs, useModelOptions } from "./use-logs";
import {
	EMPTY_FILTERS,
	buildQuery,
	filtersFromParams,
	loadSavedViews,
	resolveRange,
	storeSavedViews,
	type LogFilters,
	type SavedView,
} from "./state";

const LIVE_REFRESH_MS = 10_000;

export interface LogsDataSource {
	/** `user` hides identity filters and columns. */
	scope: "admin" | "user";
	getEvents: (query: EventPageQuery, signal: AbortSignal) => Promise<UsageEventPage>;
	eventsCsvUrl: (query: EventPageQuery) => string;
	onSessionExpired: () => void;
}

/**
 * The request log: filters, summary with distribution, a request table (or
 * session groups) and the request detail sheet. Shared by the admin page and
 * the user tab.
 */
export function LogsView({
	source,
	options,
	initialSearch,
	now,
	onExportUrl,
	toolbarTrailing,
}: {
	source: LogsDataSource;
	options: FilterOptions;
	/** Search string with the filters to start with (URL contract in ./state.ts). */
	initialSearch?: string;
	now: number;
	/** Receives the CSV URL of the current filters. */
	onExportUrl?: (url: string) => void;
	/** Extra controls after the refresh button (user export). */
	toolbarTrailing?: (exportUrl: string) => ReactNode;
}) {
	const initial = useMemo(
		() => (initialSearch ? filtersFromParams(new URLSearchParams(initialSearch)) : { filters: EMPTY_FILTERS, view: "all" as EventView, grouped: false }),
		[initialSearch],
	);
	const [filters, setFilters] = useState<LogFilters>(initial.filters);
	const [view, setView] = useState<EventView>(initial.view);
	const [grouped, setGrouped] = useState(initial.grouped);
	const [live, setLive] = useState(true);
	const [anchor, setAnchor] = useState(() => Date.now());
	const [expandedId, setExpandedId] = useState<number | null>(null);
	const [sheetId, setSheetId] = useState<number | null>(null);
	const [columns, setColumns] = useState<ReadonlySet<ColumnKey>>(() => loadColumns(source.scope));
	const [savedViews, setSavedViews] = useState<SavedView[]>(() => loadSavedViews(source.scope));
	const [activeSaved, setActiveSaved] = useState<string | null>(null);

	const [appliedInitial, setAppliedInitial] = useState(initial);
	if (appliedInitial !== initial) {
		setAppliedInitial(initial);
		setFilters(initial.filters);
		setView(initial.view);
		setGrouped(initial.grouped);
		setActiveSaved(null);
	}

	// The 1-hour range slides with live refreshes.
	useEffect(() => {
		if (!live || filters.range !== "1h") return;
		const timer = window.setInterval(() => {
			if (!document.hidden) setAnchor(Date.now());
		}, LIVE_REFRESH_MS);
		return () => window.clearInterval(timer);
	}, [filters.range, live]);

	const range = useMemo(() => resolveRange(filters, anchor), [anchor, filters]);
	const baseQuery = useMemo(() => buildQuery(filters, range, { view, limit: PAGE_SIZE, grouped }), [filters, grouped, range, view]);
	const baseKey = JSON.stringify(baseQuery);
	const [pagedKey, setPagedKey] = useState(baseKey);
	if (pagedKey !== baseKey) {
		setPagedKey(baseKey);
		setExpandedId(null);
	}
	const query = baseQuery;
	const exportUrl = source.eventsCsvUrl(query);

	useEffect(() => onExportUrl?.(exportUrl), [exportUrl, onExportUrl]);

	const page = useLogs(source.getEvents, query, {
		onSessionExpired: source.onSessionExpired,
		onError: (message) => toast.error(message),
		...(live && filters.range !== "1h" ? { refreshMs: LIVE_REFRESH_MS } : {}),
	});
	const data = page.page;
	const events = page.events;

	const models = useModelOptions(data?.models, filters.models, options.models);
	const modelColor = useCallback((model: string) => seriesColor(Math.max(0, models.indexOf(model))), [models]);

	const changeFilters = useCallback((next: LogFilters) => {
		setFilters(next);
		setActiveSaved(null);
	}, []);

	function saveView(name: string) {
		const saved: SavedView = { id: randomUuid(), name, filters, view };
		const next = [...savedViews, saved];
		setSavedViews(next);
		storeSavedViews(source.scope, next);
		setActiveSaved(saved.id);
	}

	function deleteView(id: string) {
		const next = savedViews.filter((saved) => saved.id !== id);
		setSavedViews(next);
		storeSavedViews(source.scope, next);
		if (activeSaved === id) setActiveSaved(null);
	}

	function filterSession(sessionId: string) {
		setSheetId(null);
		setGrouped(false);
		changeFilters({ ...filters, sessionId });
	}

	function toggleColumn(key: ColumnKey, visible: boolean) {
		const next = new Set(columns);
		if (visible) next.add(key);
		else next.delete(key);
		setColumns(next);
		storeColumns(source.scope, next);
	}

	const availableColumns = COLUMNS.filter((column) => source.scope === "admin" || !column.adminOnly);
	const shownColumns = new Set([...columns].filter((key) => availableColumns.some((column) => column.key === key)));
	const sheetIndex = sheetId === null ? -1 : events.findIndex((event) => event.id === sheetId);
	const sheetEvent = sheetIndex >= 0 ? (events[sheetIndex] ?? null) : null;
	const rangeLabel = filters.range === "today" ? "今日" : filters.range === "1h" ? "近 1 小时" : "区间内";
	const total = data?.total ?? 0;
	const loaded = grouped ? page.sessions.length : events.length;
	const oldest = events[events.length - 1]?.recordedAt;

	return (
		<div className="flex flex-col gap-4">
			<LogFilterBar
				activeSavedView={activeSaved}
				counts={data?.views ?? null}
				filters={filters}
				live={live}
				loading={page.loading}
				onApplySavedView={(saved) => {
					setFilters(saved.filters);
					setView(saved.view);
					setActiveSaved(saved.id);
				}}
				onChange={changeFilters}
				onDeleteSavedView={deleteView}
				onLiveChange={setLive}
				onRefresh={() => {
					setAnchor(Date.now());
					page.reload();
				}}
				onSaveView={saveView}
				onViewChange={(next) => {
					setView(next);
					setActiveSaved(null);
				}}
				options={{ ...options, models }}
				range={range}
				savedViews={savedViews}
				trailing={toolbarTrailing?.(exportUrl)}
				view={view}
			/>

			{page.error ? (
				<Alert variant="destructive">
					<AlertDescription>{page.error}</AlertDescription>
				</Alert>
			) : null}

			<LogSummaryBlock
				data={data}
				loading={page.loading}
				now={now}
				onSelectRange={(custom) => changeFilters({ ...filters, range: "custom", custom })}
				rangeLabel={rangeLabel}
			/>

			<SectionCard
				actions={
					<>
						<span className="text-[12.5px] text-muted-foreground">分组</span>
						<Segmented
							label="分组"
							onChange={(value) => setGrouped(value === "session")}
							options={[
								{ value: "request", label: "逐条请求" },
								{ value: "session", label: "按会话" },
							]}
							value={grouped ? "session" : "request"}
						/>
						{grouped ? null : (
							<DropdownMenu>
								<DropdownMenuTrigger asChild>
									<Button size="sm" type="button" variant="outline">
										<Columns3Icon data-icon="inline-start" />
										列 {shownColumns.size + 1} / {availableColumns.length + 1}
									</Button>
								</DropdownMenuTrigger>
								<DropdownMenuContent align="end" className="min-w-44">
									<DropdownMenuLabel className="text-xs text-muted-foreground">显示列</DropdownMenuLabel>
									<DropdownMenuSeparator />
									<DropdownMenuCheckboxItem checked disabled>
										时间
									</DropdownMenuCheckboxItem>
									{availableColumns.map((column) => (
										<DropdownMenuCheckboxItem
											checked={shownColumns.has(column.key)}
											key={column.key}
											onCheckedChange={(checked) => toggleColumn(column.key, checked === true)}
											onSelect={(event) => event.preventDefault()}
										>
											{column.label}
										</DropdownMenuCheckboxItem>
									))}
								</DropdownMenuContent>
							</DropdownMenu>
						)}
					</>
				}
				description={
					data
						? grouped
							? `${formatCount(total)} 组 · 按最近活动排序`
							: `${formatCount(total)} 条 · 按时间倒序`
						: "加载中…"
				}
				title={source.scope === "user" ? "请求明细" : "请求"}
			>
				{!data ? (
					<div className="flex flex-col gap-2">
						{Array.from({ length: 6 }, (_, index) => (
							<Skeleton className="h-12" key={index} />
						))}
					</div>
				) : loaded === 0 ? (
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
						{grouped ? (
							<SessionTable
								groups={page.sessions}
								onOpen={(event) => setSheetId(event.id)}
								onSessionFilter={filterSession}
								showIdentity={source.scope === "admin"}
							/>
						) : (
							<RequestTable
								columns={shownColumns}
								events={events}
								expandedId={expandedId}
								modelColor={modelColor}
								now={now}
								onOpen={(event) => setSheetId(event.id)}
								onSessionFilter={filterSession}
								sessionSummary={(event) => sessionSummary(event, page.sessionCounts)}
								onToggle={(id) => setExpandedId((current) => (current === id ? null : id))}
								slowThresholdMs={data.slowThresholdMs}
							/>
						)}
						<ScrollBar orientation="horizontal" />
					</ScrollArea>
				)}
				{data && loaded > 0 ? (
					<div className="flex flex-wrap items-center gap-x-4 gap-y-2 pt-1">
						<span className="text-xs text-muted-foreground tabular-nums">
							{grouped ? `已显示 ${loaded} / ${formatCount(total)} 组` : `已加载 ${loaded} / ${formatCount(total)} 条`}
						</span>
						{!grouped && oldest ? (
							<span className="text-xs text-faint tabular-nums">
								最早 {oldest < startOfLocalDay(now) ? `${formatDay(oldest)} ` : ""}
								{formatClock(oldest, false)}
								{live ? " · 实时追加新请求" : ""}
							</span>
						) : null}
						<span className="flex-1" />
						{grouped ? <SessionLegend /> : null}
						{page.canLoadMore ? (
							<Button disabled={page.loadingMore} onClick={page.loadMore} size="sm" type="button" variant="outline">
								{page.loadingMore ? <Spinner data-icon="inline-start" /> : null}
								加载更多 {Math.min(PAGE_SIZE, total - loaded)} {grouped ? "组" : "条"}
							</Button>
						) : null}
					</div>
				) : null}
			</SectionCard>

			<RequestSheet
				event={sheetEvent}
				index={sheetIndex}
				onClose={() => setSheetId(null)}
				onSessionFilter={filterSession}
				sessionSummary={sheetEvent ? sessionSummary(sheetEvent, page.sessionCounts) : null}
				onStep={(delta) => {
					const next = events[sheetIndex + delta];
					if (next) setSheetId(next.id);
				}}
				total={events.length}
				{...(source.scope === "admin"
					? {
							onIdentityFilter: (event: UsageEvent) => {
								setSheetId(null);
								changeFilters({ ...filters, downstream: { identityType: "user", identityId: event.userId } });
							},
						}
					: {})}
			/>
		</div>
	);
}

function LogSummaryBlock({
	data,
	loading,
	now,
	rangeLabel,
	onSelectRange,
}: {
	data: UsageEventPage | null;
	loading: boolean;
	now: number;
	rangeLabel: string;
	onSelectRange: (range: { startAt: number; endAt: number }) => void;
}) {
	return (
		<LogSummary
			bucketMs={data?.bucketMs ?? 600_000}
			loading={loading}
			now={now}
			onSelectRange={onSelectRange}
			rangeLabel={rangeLabel}
			rangeTotal={data?.rangeTotal ?? 0}
			series={data?.series ?? []}
			summary={data?.summary ?? null}
		/>
	);
}

function columnsKey(scope: string): string {
	return `codex-router.logs.columns.${scope}`;
}

function loadColumns(scope: string): ReadonlySet<ColumnKey> {
	try {
		const raw = window.localStorage.getItem(columnsKey(scope));
		const parsed: unknown = raw ? JSON.parse(raw) : null;
		if (Array.isArray(parsed)) return new Set(parsed.filter((key): key is ColumnKey => COLUMNS.some((column) => column.key === key)));
	} catch {
		// fall through to defaults
	}
	return new Set(COLUMNS.filter((column) => column.defaultVisible).map((column) => column.key));
}

function storeColumns(scope: string, columns: ReadonlySet<ColumnKey>): void {
	try {
		window.localStorage.setItem(columnsKey(scope), JSON.stringify([...columns]));
	} catch {
		// Column choices then last for the session only.
	}
}


