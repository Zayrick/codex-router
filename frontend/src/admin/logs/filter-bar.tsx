import { useEffect, useRef, useState, type ReactNode } from "react";
import { ListFilterIcon, PlusIcon, RefreshCwIcon, SaveIcon, SearchIcon, XIcon } from "lucide-react";

import type { EventView, EventViewCounts, UsageEventStatus, UsageIdentityFilter } from "@/admin-api";
import { FilterMenu, type FilterOption } from "@/components/app/filter-menu";
import { Segmented } from "@/components/app/segmented";
import { STATUS_META, STATUS_ORDER, SESSION_KIND_META, identityTypeLabel, tierLabel } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { formatClock, formatDateTime, formatDay, startOfLocalDay } from "@/lib/format";
import { cn } from "@/lib/utils";

import { RANGE_OPTIONS, type LogFilters, type ResolvedRange, type SavedView } from "./state";

export interface IdentityOption {
	filter: UsageIdentityFilter;
	label: string;
	hint?: string;
}

export interface FilterOptions {
	/** Downstream identities; omitted in the user scope. */
	identities?: IdentityOption[];
	/** Account groups and Codex accounts; omitted in the user scope. */
	upstreams?: IdentityOption[];
	models: string[];
}

const EFFORTS = ["minimal", "low", "medium", "high", "xhigh"];
const TIERS = ["priority", "default", "auto", "flex"];
const TRANSPORTS: FilterOption[] = [
	{ value: "http", label: "HTTP" },
	{ value: "websocket", label: "WebSocket" },
];

const VIEW_TABS: { value: EventView; label: string; tone: "neutral" | "danger" | "warning" }[] = [
	{ value: "all", label: "全部", tone: "neutral" },
	{ value: "failed", label: "失败", tone: "danger" },
	{ value: "slow", label: "慢请求 > P95", tone: "warning" },
	{ value: "downgraded", label: "档位降级", tone: "warning" },
	{ value: "subagent", label: "子代理", tone: "neutral" },
];

function identityKey(filter: UsageIdentityFilter | null): string {
	return filter ? `${filter.identityType}:${filter.identityId}` : "";
}

function parseIdentity(options: IdentityOption[], key: string | undefined): UsageIdentityFilter | null {
	return options.find((option) => identityKey(option.filter) === key)?.filter ?? null;
}

/** Figma "log-filters": saved views, search + time, field filters, applied conditions. */
export function LogFilterBar({
	filters,
	onChange,
	view,
	onViewChange,
	counts,
	savedViews,
	activeSavedView,
	onApplySavedView,
	onSaveView,
	onDeleteSavedView,
	options,
	range,
	live,
	pushed = false,
	onLiveChange,
	onRefresh,
	loading,
	trailing,
}: {
	filters: LogFilters;
	onChange: (filters: LogFilters) => void;
	view: EventView;
	onViewChange: (view: EventView) => void;
	counts: EventViewCounts | null;
	savedViews: SavedView[];
	activeSavedView: string | null;
	onApplySavedView: (view: SavedView) => void;
	onSaveView: (name: string) => void;
	onDeleteSavedView: (id: string) => void;
	options: FilterOptions;
	range: ResolvedRange;
	live: boolean;
	/** New requests are pushed by the server instead of polled. */
	pushed?: boolean;
	onLiveChange: (live: boolean) => void;
	onRefresh: () => void;
	loading: boolean;
	/** Extra controls at the end of the search row (e.g. export). */
	trailing?: ReactNode;
}) {
	const searchRef = useRef<HTMLInputElement>(null);
	const [search, setSearch] = useState(filters.search);
	const [syncedSearch, setSyncedSearch] = useState(filters.search);
	if (syncedSearch !== filters.search) {
		setSyncedSearch(filters.search);
		setSearch(filters.search);
	}
	const [extraOpen, setExtraOpen] = useState(() => filters.transports.length > 0 || filters.sessionKinds.length > 0);

	useEffect(() => {
		const timer = window.setTimeout(() => {
			if (search !== filters.search) onChange({ ...filters, search });
		}, 350);
		return () => window.clearTimeout(timer);
	}, [filters, onChange, search]);

	useEffect(() => {
		function onKey(event: KeyboardEvent) {
			const target = event.target as HTMLElement | null;
			if (event.key !== "/" || target?.closest("input, textarea, [contenteditable=true]")) return;
			event.preventDefault();
			searchRef.current?.focus();
		}
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, []);

	const set = (patch: Partial<LogFilters>) => onChange({ ...filters, ...patch });
	const conditions = appliedConditions(filters, options);
	const showExtra = extraOpen || filters.transports.length > 0 || filters.sessionKinds.length > 0;

	return (
		<Card className="gap-0 overflow-hidden p-0">
			<div className="flex items-center gap-2 border-b px-4">
				<div className="-mb-px flex min-w-0 flex-1 gap-1 overflow-x-auto" role="tablist" aria-label="快捷视图">
					{VIEW_TABS.map((tab) => (
						<ViewTab
							active={activeSavedView === null && view === tab.value}
							count={counts?.[tab.value === "rate_limited" ? "rateLimited" : tab.value]}
							key={tab.value}
							label={tab.label}
							onClick={() => onViewChange(tab.value)}
							tone={tab.tone}
						/>
					))}
					{savedViews.map((saved) => (
						<ViewTab
							active={activeSavedView === saved.id}
							key={saved.id}
							label={saved.name}
							onClick={() => onApplySavedView(saved)}
							onRemove={() => onDeleteSavedView(saved.id)}
							tone="neutral"
						/>
					))}
				</div>
				<SaveViewButton label="新建视图" onSave={onSaveView} variant="ghost" />
			</div>

			<div className="flex flex-wrap items-center gap-2 px-4 pt-4">
				<InputGroup className="min-w-56 flex-1">
					<InputGroupAddon>
						<SearchIcon />
					</InputGroupAddon>
					<InputGroupInput
						aria-label="搜索请求"
						onChange={(event) => setSearch(event.target.value)}
						placeholder="搜索 request / response / session ID"
						ref={searchRef}
						value={search}
					/>
					<InputGroupAddon align="inline-end">
						<kbd className="rounded border bg-surface-subtle px-1.5 font-mono text-[11px] text-muted-foreground">/</kbd>
					</InputGroupAddon>
				</InputGroup>
				<div className="flex items-center gap-2">
					<Segmented
						label="时间范围"
						onChange={(value) => {
							if (value !== "custom") set({ range: value, custom: null });
						}}
						options={RANGE_OPTIONS.filter((option) => option.value !== "custom")}
						size="default"
						value={filters.range === "custom" ? ("" as never) : filters.range}
					/>
					<CustomRange filters={filters} onApply={(custom) => set({ range: "custom", custom })} range={range} />
				</div>
				<div className="flex flex-col">
					<span className="text-[12.5px] leading-5 font-medium tabular-nums">{rangeCaption(range)}</span>
					<span className="text-[11px] leading-4 text-faint">
						{Intl.DateTimeFormat().resolvedOptions().timeZone}
						{live ? (pushed ? " · 实时推送" : " · 每 10 秒刷新") : ""}
					</span>
				</div>
				<label className="flex items-center gap-2 px-2 text-[12.5px] font-medium">
					<Switch checked={live} onCheckedChange={onLiveChange} />
					实时
				</label>
				<Button aria-label="刷新" disabled={loading} onClick={onRefresh} size="icon" type="button" variant="outline">
					{loading ? <Spinner /> : <RefreshCwIcon />}
				</Button>
				{trailing}
			</div>

			<div className="flex flex-wrap items-center gap-2 px-4 pt-2 pb-4">
				{options.identities ? (
					<FilterMenu
						label="用户"
						multiple={false}
						onChange={(values) => set({ downstream: parseIdentity(options.identities ?? [], values[0]) })}
						options={options.identities.map((option) => ({
							value: identityKey(option.filter),
							label: option.label,
							text: option.label,
							hint: option.hint,
						}))}
						selected={filters.downstream ? [identityKey(filters.downstream)] : []}
					/>
				) : null}
				{options.upstreams ? (
					<FilterMenu
						label="上游路由"
						multiple={false}
						onChange={(values) => set({ upstream: parseIdentity(options.upstreams ?? [], values[0]) })}
						options={options.upstreams.map((option) => ({
							value: identityKey(option.filter),
							label: option.label,
							text: option.label,
							hint: option.hint,
						}))}
						selected={filters.upstream ? [identityKey(filters.upstream)] : []}
					/>
				) : null}
				<FilterMenu
					label="模型"
					onChange={(models) => set({ models })}
					options={options.models.map((model) => ({ value: model, label: <span className="font-mono text-xs">{model}</span>, text: model }))}
					selected={filters.models}
				/>
				<FilterMenu
					label="状态"
					onChange={(values) => set({ statuses: values as UsageEventStatus[] })}
					options={STATUS_ORDER.map((status) => ({ value: status, label: STATUS_META[status].label, text: STATUS_META[status].label }))}
					selected={filters.statuses}
				/>
				<FilterMenu
					label="推理强度"
					onChange={(efforts) => set({ efforts })}
					options={EFFORTS.map((effort) => ({ value: effort, label: effort, text: effort }))}
					selected={filters.efforts}
				/>
				<FilterMenu
					label="速度档位"
					onChange={(tiers) => set({ tiers })}
					options={TIERS.map((tier) => ({ value: tier, label: tierLabel(tier), text: tierLabel(tier) }))}
					selected={filters.tiers}
				/>
				{showExtra ? (
					<>
						<FilterMenu label="传输" onChange={(transports) => set({ transports })} options={TRANSPORTS} selected={filters.transports} />
						<FilterMenu
							label="会话类型"
							onChange={(sessionKinds) => set({ sessionKinds })}
							options={Object.entries(SESSION_KIND_META).map(([value, meta]) => ({ value, label: meta.label, text: meta.label }))}
							selected={filters.sessionKinds}
						/>
					</>
				) : (
					<Button onClick={() => setExtraOpen(true)} size="sm" type="button" variant="ghost">
						<PlusIcon data-icon="inline-start" />
						更多条件
					</Button>
				)}
			</div>

			{conditions.length ? (
				<div className="flex flex-wrap items-center gap-2 border-t bg-surface-subtle/60 px-4 py-2">
					<ListFilterIcon className="size-3.5 text-muted-foreground" />
					<span className="text-xs text-muted-foreground">已应用 {conditions.length} 个条件</span>
					{conditions.map((condition) => (
						<span className="inline-flex items-center gap-1 rounded-full border bg-card py-0.5 pr-1 pl-2 text-xs" key={condition.key}>
							<span className="text-muted-foreground">{condition.label}</span>
							<span className="max-w-72 truncate font-semibold">{condition.value}</span>
							<Button
								aria-label={`移除${condition.label}`}
								className="size-4 rounded-full"
								onClick={() => onChange(condition.clear(filters))}
								size="icon-xs"
								type="button"
								variant="ghost"
							>
								<XIcon className="size-3" />
							</Button>
						</span>
					))}
					<Button
						onClick={() => onChange({ ...filters, ...clearedConditions })}
						size="xs"
						type="button"
						variant="link"
					>
						清除全部
					</Button>
					<span className="flex-1" />
					<SaveViewButton label="保存为视图" onSave={onSaveView} variant="outline" />
				</div>
			) : null}
		</Card>
	);
}

const clearedConditions: Partial<LogFilters> = {
	downstream: null,
	upstream: null,
	models: [],
	statuses: [],
	efforts: [],
	tiers: [],
	transports: [],
	sessionKinds: [],
	sessionId: "",
};

function ViewTab({
	active,
	label,
	count,
	tone,
	onClick,
	onRemove,
}: {
	active: boolean;
	label: string;
	count?: number | undefined;
	tone: "neutral" | "danger" | "warning";
	onClick: () => void;
	onRemove?: () => void;
}) {
	return (
		<div
			className={cn(
				"group/view flex shrink-0 items-center gap-1.5 border-b-2 border-transparent px-3 pt-3 pb-2.5",
				active && "border-primary",
			)}
		>
			<button
				aria-selected={active}
				className={cn("flex items-center gap-1.5 text-[13px] leading-5", active ? "font-bold text-foreground" : "font-medium text-muted-foreground hover:text-foreground")}
				onClick={onClick}
				role="tab"
				type="button"
			>
				{label}
				{count !== undefined ? (
					<span
						className={cn(
							"rounded-full px-1.5 text-[11px] leading-4 font-semibold tabular-nums",
							active ? "bg-primary-soft text-foreground" : tone === "danger" ? "bg-danger-soft text-danger" : tone === "warning" ? "bg-warning-soft text-warning" : "bg-surface-subtle text-muted-foreground",
						)}
					>
						{count.toLocaleString("en-US")}
					</span>
				) : null}
			</button>
			{onRemove ? (
				<Button aria-label={`删除视图 ${label}`} className="size-4 opacity-0 group-hover/view:opacity-100 focus-visible:opacity-100" onClick={onRemove} size="icon-xs" type="button" variant="ghost">
					<XIcon className="size-3" />
				</Button>
			) : null}
		</div>
	);
}

function SaveViewButton({ label, onSave, variant }: { label: string; onSave: (name: string) => void; variant: "ghost" | "outline" }) {
	const [open, setOpen] = useState(false);
	const [name, setName] = useState("");
	return (
		<Popover onOpenChange={setOpen} open={open}>
			<PopoverTrigger asChild>
				<Button size="xs" type="button" variant={variant}>
					{variant === "ghost" ? <PlusIcon data-icon="inline-start" /> : <SaveIcon data-icon="inline-start" />}
					{label}
				</Button>
			</PopoverTrigger>
			<PopoverContent align="end" className="w-64">
				<form
					className="flex flex-col gap-2"
					onSubmit={(event) => {
						event.preventDefault();
						if (!name.trim()) return;
						onSave(name.trim());
						setName("");
						setOpen(false);
					}}
				>
					<span className="text-xs text-muted-foreground">将当前筛选条件保存为视图</span>
					<Input autoFocus maxLength={24} onChange={(event) => setName(event.target.value)} placeholder="视图名称" value={name} />
					<Button disabled={!name.trim()} size="sm" type="submit">
						保存
					</Button>
				</form>
			</PopoverContent>
		</Popover>
	);
}

function toLocalInput(at: number): string {
	const date = new Date(at - new Date(at).getTimezoneOffset() * 60_000);
	return date.toISOString().slice(0, 16);
}

function CustomRange({ filters, range, onApply }: { filters: LogFilters; range: ResolvedRange; onApply: (custom: { startAt: number; endAt: number }) => void }) {
	const [open, setOpen] = useState(false);
	const [start, setStart] = useState("");
	const [end, setEnd] = useState("");
	const startAt = start ? new Date(start).getTime() : NaN;
	const endAt = end ? new Date(end).getTime() : NaN;
	const valid = Number.isFinite(startAt) && Number.isFinite(endAt) && endAt > startAt;
	return (
		<Popover
			onOpenChange={(next) => {
				if (next) {
					setStart(toLocalInput(range.startAt));
					setEnd(toLocalInput(Math.min(range.endAt, Date.now())));
				}
				setOpen(next);
			}}
			open={open}
		>
			<PopoverTrigger asChild>
				<Button
					className={cn("h-8 px-3 text-[13px]", filters.range === "custom" ? "bg-surface-subtle text-foreground" : "text-muted-foreground")}
					type="button"
					variant="outline"
				>
					自定义
				</Button>
			</PopoverTrigger>
			<PopoverContent align="start" className="w-72">
				<form
					className="flex flex-col gap-2"
					onSubmit={(event) => {
						event.preventDefault();
						if (!valid) return;
						onApply({ startAt, endAt });
						setOpen(false);
					}}
				>
					<label className="flex flex-col gap-1 text-xs text-muted-foreground">
						开始
						<Input onChange={(event) => setStart(event.target.value)} type="datetime-local" value={start} />
					</label>
					<label className="flex flex-col gap-1 text-xs text-muted-foreground">
						结束
						<Input onChange={(event) => setEnd(event.target.value)} type="datetime-local" value={end} />
					</label>
					<Button disabled={!valid} size="sm" type="submit">
						应用
					</Button>
				</form>
			</PopoverContent>
		</Popover>
	);
}

function rangeCaption(range: ResolvedRange): string {
	const end = Math.min(range.endAt, Date.now());
	const sameDay = startOfLocalDay(range.startAt) === startOfLocalDay(end);
	return sameDay
		? `${formatDay(range.startAt)} ${formatClock(range.startAt, false)} — ${formatClock(end, false)}`
		: `${formatDateTime(range.startAt)} — ${formatDateTime(end)}`;
}

interface AppliedCondition {
	key: string;
	label: string;
	value: string;
	clear: (filters: LogFilters) => LogFilters;
}

function appliedConditions(filters: LogFilters, options: FilterOptions): AppliedCondition[] {
	const items: AppliedCondition[] = [];
	const name = (list: IdentityOption[] | undefined, filter: UsageIdentityFilter) =>
		list?.find((option) => identityKey(option.filter) === identityKey(filter))?.label ?? filter.identityId;
	if (filters.downstream) {
		items.push({ key: "downstream", label: identityTypeLabel(filters.downstream.identityType), value: name(options.identities, filters.downstream), clear: (value) => ({ ...value, downstream: null }) });
	}
	if (filters.upstream) {
		items.push({ key: "upstream", label: "上游路由", value: name(options.upstreams, filters.upstream), clear: (value) => ({ ...value, upstream: null }) });
	}
	const lists: [keyof LogFilters, string, (value: string) => string][] = [
		["models", "模型", (value) => value],
		["statuses", "状态", (value) => STATUS_META[value as UsageEventStatus]?.label ?? value],
		["efforts", "推理强度", (value) => value],
		["tiers", "速度档位", tierLabel],
		["transports", "传输", (value) => (value === "websocket" ? "WebSocket" : "HTTP")],
		["sessionKinds", "会话类型", (value) => SESSION_KIND_META[value as keyof typeof SESSION_KIND_META]?.label ?? value],
	];
	for (const [key, label, format] of lists) {
		const values = filters[key] as string[];
		if (values.length) items.push({ key, label, value: values.map(format).join("、"), clear: (value) => ({ ...value, [key]: [] }) });
	}
	if (filters.sessionId) items.push({ key: "session", label: "会话", value: filters.sessionId, clear: (value) => ({ ...value, sessionId: "" }) });
	return items;
}
