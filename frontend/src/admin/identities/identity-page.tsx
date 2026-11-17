import { useMemo, useState } from "react";
import {
	ActivityIcon,
	CheckCircle2Icon,
	ChevronDownIcon,
	ChevronUpIcon,
	CopyIcon,
	EyeIcon,
	EyeOffIcon,
	KeyRoundIcon,
	RefreshCwIcon,
	SearchIcon,
	TriangleAlertIcon,
	UsersIcon,
} from "lucide-react";

import type { IdentityStats } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { DeleteConfirmationDialog } from "@/admin/dialogs";
import { useAnalytics } from "@/admin/hooks";
import { RouteTarget } from "@/components/app/badges";
import { Legend, LegendItem, Meter, SparkBars } from "@/components/app/bars";
import { Segmented } from "@/components/app/segmented";
import { StatCard } from "@/components/app/stat-card";
import { VIZ } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Card, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectSeparator, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { DAY_MS, formatCost, formatCount, formatDateTime, formatPercent, formatTokens, maskSecret, ratio } from "@/lib/format";
import { cn } from "@/lib/utils";

import { IdentityDetail } from "./identity-detail";
import { useLastSeen } from "./use-last-seen";
import { apiKeyEntries, clientSummary, proxyEntries, routeInfo, successRate, type IdentityEntry, type IdentityKind, type RouteInfo } from "./model";

type StatusFilter = "all" | "enabled" | "disabled" | "unassigned";
type Period = "24h" | "7d" | "30d";

const WINDOW_LABEL: Record<Period, string> = { "24h": "24 小时", "7d": "7 天", "30d": "30 天" };
const ALL_TARGETS = "__all__";
const UNASSIGNED = "__unassigned__";

const COPY: Record<IdentityKind, { noun: string; title: string; keyLabel: string; search: string; unassignedHint: string; activeUnit: string; empty: string }> = {
	api_key: {
		noun: "API Key",
		title: "全部 API Key",
		keyLabel: "名称 · Key",
		search: "搜索名称、Key 或客户端",
		unassignedHint: "未分配时请求不可用",
		activeUnit: "个 Key",
		empty: "还没有 API Key",
	},
	auth_proxy: {
		noun: "下游账户",
		title: "全部下游账户",
		keyLabel: "名称 · account_id",
		search: "搜索名称、account_id 或客户端",
		unassignedHint: "透传客户端自带的上游凭据",
		activeUnit: "个账户",
		empty: "还没有下游账户",
	},
};

interface Row {
	entry: IdentityEntry;
	route: RouteInfo;
	stats: IdentityStats | null;
	/** Last use across all recorded events. */
	lastSeen: { lastAt: number; userAgent: string } | null;
}

/** API Keys / 下游账户 page body: summary, toolbar and the shared identity table. */
export function IdentityPage({ kind }: { kind: IdentityKind }) {
	const { state, now } = useAdmin();
	const copy = COPY[kind];
	const [period, setPeriod] = useState<Period>("7d");
	const [status, setStatus] = useState<StatusFilter>("all");
	const [target, setTarget] = useState(ALL_TARGETS);
	const [search, setSearch] = useState("");
	const [ascending, setAscending] = useState(false);
	const usage = useAnalytics({ range: period });
	const day = useAnalytics({ range: "24h" });
	const lastSeen = useLastSeen();

	const entries = useMemo(
		() => (kind === "api_key" ? apiKeyEntries(state.apiKeys) : proxyEntries(state.authProxyAccounts)),
		[kind, state.apiKeys, state.authProxyAccounts],
	);
	const rows: Row[] = useMemo(
		() =>
			entries.map((entry) => ({
				entry,
				route: routeInfo(state.routes, state.accountGroups, state.codexAccounts, entry),
				stats: usage.data?.identities.find((item) => item.identityType === kind && item.identityId === entry.id) ?? null,
				lastSeen: lastSeen.data?.identities.find((item) => item.identityType === kind && item.identityId === entry.id) ?? null,
			})),
		[entries, kind, lastSeen.data, state.accountGroups, state.codexAccounts, state.routes, usage.data],
	);

	const kindTokens = rows.reduce((sum, row) => sum + (row.stats?.totalTokens ?? 0), 0);
	const kindCost = rows.reduce((sum, row) => sum + (row.stats?.costUsd ?? 0), 0);
	const enabled = rows.filter((row) => row.entry.enabled).length;
	const unassigned = rows.filter((row) => row.route.type === "unassigned");
	const dayRows = (day.data?.identities ?? []).filter((item) => item.identityType === kind && entries.some((entry) => entry.id === item.identityId));
	const activeDay = dayRows.filter((item) => item.lastAt >= now - DAY_MS);
	const dayRequests = dayRows.reduce((sum, item) => sum + item.requests, 0);
	const daySuccess = successRate(
		dayRows.reduce(
			(sum, item) => ({
				requests: 0,
				completed: sum.completed + item.completed,
				incomplete: sum.incomplete + item.incomplete,
				failed: sum.failed + item.failed,
				cancelled: 0,
				interrupted: sum.interrupted + item.interrupted,
				rateLimited: 0,
			}),
			{ requests: 0, completed: 0, incomplete: 0, failed: 0, cancelled: 0, interrupted: 0, rateLimited: 0 },
		),
	);

	const query = search.trim().toLowerCase();
	const visible = rows
		.filter((row) => {
			if (status === "enabled" && !row.entry.enabled) return false;
			if (status === "disabled" && row.entry.enabled) return false;
			if (status === "unassigned" && row.route.type !== "unassigned") return false;
			if (target === UNASSIGNED && row.route.type !== "unassigned") return false;
			if (target !== ALL_TARGETS && target !== UNASSIGNED) {
				const route = state.routes.find((item) => item.consumerType === kind && item.consumerId === row.entry.id);
				if (!route || `${route.targetType}:${route.targetId}` !== target) return false;
			}
			if (!query) return true;
			return [row.entry.name, row.entry.credential, row.stats?.lastUserAgent ?? ""].some((value) => value.toLowerCase().includes(query));
		})
		.sort((left, right) => {
			const difference = (right.stats?.totalTokens ?? -1) - (left.stats?.totalTokens ?? -1);
			const ordered = difference !== 0 ? difference : left.entry.name.localeCompare(right.entry.name);
			return ascending ? -ordered : ordered;
		});

	return (
		<div className="flex flex-col gap-4">
			<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
				<StatCard
					aside={kind === "api_key" ? <KeyRoundIcon className="size-4 text-faint" /> : <UsersIcon className="size-4 text-faint" />}
					className="gap-1 py-4"
					detail={<span className="text-[11.5px] text-faint">{usage.data ? `${WINDOW_LABEL[period]} ${formatTokens(kindTokens)} Token · ${formatCost(kindCost)}` : "—"}</span>}
					label={copy.noun}
					size="sm"
					value={rows.length}
				/>
				<StatCard
					aside={<CheckCircle2Icon className="size-4 text-faint" />}
					className="gap-1 py-4"
					detail={<span className="text-[11.5px] text-faint">{rows.length - enabled} 个已停用</span>}
					label="已启用"
					size="sm"
					unit={`/ ${rows.length}`}
					value={enabled}
				>
					<Meter color={VIZ.success} height={4} ratio={ratio(enabled, rows.length)} />
				</StatCard>
				<StatCard
					aside={<TriangleAlertIcon className="size-4 text-faint" />}
					className="gap-1 py-4"
					detail={
						<span className="text-[11.5px] text-faint">
							{unassigned.length === 0
								? "全部已分配路由"
								: kind === "api_key"
									? `${unassigned.every((row) => !row.entry.enabled) ? "均已停用 · " : ""}${copy.unassignedHint}`
									: `${unassigned
											.slice(0, 2)
											.map((row) => row.entry.name)
											.join("、")} · ${copy.unassignedHint}`}
						</span>
					}
					label="未分配"
					size="sm"
					value={unassigned.length}
					valueClassName={unassigned.length ? "text-warning" : ""}
				/>
				<StatCard
					aside={<ActivityIcon className="size-4 text-faint" />}
					className="gap-1 py-4"
					detail={
						<span className="text-[11.5px] text-faint">
							24 小时 {formatCount(dayRequests)} 次请求 · 成功率 {formatPercent(daySuccess)}
						</span>
					}
					label="24 小时活跃"
					size="sm"
					unit={copy.activeUnit}
					value={day.data ? activeDay.length : "—"}
				/>
			</div>

			<Card className="flex-row flex-wrap items-center justify-between gap-3 p-2">
				<div className="flex flex-wrap items-center gap-3">
					<InputGroup className="h-8 w-full rounded-lg sm:w-70">
						<InputGroupAddon>
							<SearchIcon />
						</InputGroupAddon>
						<InputGroupInput aria-label={copy.search} onChange={(event) => setSearch(event.target.value)} placeholder={copy.search} value={search} />
					</InputGroup>
					<Segmented
						label="状态"
						onChange={setStatus}
						options={[
							{ value: "all", label: "全部", count: rows.length },
							{ value: "enabled", label: "启用", count: enabled },
							{ value: "disabled", label: "停用", count: rows.length - enabled },
							{ value: "unassigned", label: "未分配", count: unassigned.length },
						]}
						size="default"
						value={status}
					/>
				</div>
				<div className="flex flex-wrap items-center gap-2">
					<span className="text-xs text-faint">统计窗口</span>
					<Segmented
						label="统计窗口"
						onChange={setPeriod}
						options={(["24h", "7d", "30d"] as const).map((value) => ({ value, label: WINDOW_LABEL[value] }))}
						size="default"
						value={period}
					/>
					<Select onValueChange={setTarget} value={target}>
						<SelectTrigger aria-label="路由目标" className="min-w-36">
							<SelectValue />
						</SelectTrigger>
						<SelectContent position="popper">
							<SelectGroup>
								<SelectItem value={ALL_TARGETS}>全部路由目标</SelectItem>
								<SelectItem value={UNASSIGNED}>未分配</SelectItem>
							</SelectGroup>
							{state.accountGroups.length ? (
								<>
									<SelectSeparator />
									<SelectGroup>
										<SelectLabel>账户组</SelectLabel>
										{state.accountGroups.map((group) => (
											<SelectItem key={group.id} value={`group:${group.id}`}>
												{group.name}
											</SelectItem>
										))}
									</SelectGroup>
								</>
							) : null}
							{state.codexAccounts.length ? (
								<>
									<SelectSeparator />
									<SelectGroup>
										<SelectLabel>Codex 账户</SelectLabel>
										{state.codexAccounts.map((account) => (
											<SelectItem key={account.id} value={`account:${account.id}`}>
												{account.name}
											</SelectItem>
										))}
									</SelectGroup>
								</>
							) : null}
						</SelectContent>
					</Select>
					<Button
						aria-label="刷新"
						disabled={usage.loading}
						onClick={() => {
							usage.reload();
							day.reload();
						}}
						size="icon"
						type="button"
						variant="outline"
					>
						{usage.loading ? <Spinner /> : <RefreshCwIcon />}
					</Button>
				</div>
			</Card>

			<Card className="gap-0 overflow-hidden py-0">
				<CardHeader className="flex flex-wrap items-start justify-between gap-3 px-6 py-5">
					<div className="flex flex-col gap-1">
						<CardTitle className="text-[15px] leading-5 font-semibold tracking-[-0.01em]">{copy.title}</CardTitle>
						<CardDescription className="text-[12.5px] leading-5">按 Token 降序 · 展开行查看明细</CardDescription>
					</div>
					<Legend className="pt-1">
						<LegendItem color={VIZ.blue} label={period === "7d" ? "每日 Token（7 天柱）" : `Token 趋势（${WINDOW_LABEL[period]}分 7 段）`} />
						<LegendItem color={VIZ.success} label="24 小时内活跃" shape="dot" />
						<LegendItem color={VIZ.warning} label="未分配路由" shape="dot" />
					</Legend>
				</CardHeader>
				{rows.length === 0 ? (
					<Empty className="border-t py-12">
						<EmptyHeader>
							<EmptyMedia variant="icon">{kind === "api_key" ? <KeyRoundIcon /> : <UsersIcon />}</EmptyMedia>
							<EmptyTitle>{copy.empty}</EmptyTitle>
							<EmptyDescription>点击右上角添加{copy.noun}。</EmptyDescription>
						</EmptyHeader>
					</Empty>
				) : (
					<div className="overflow-x-auto border-t">
						<Table className="min-w-[1080px] table-fixed">
							<colgroup>
								<col className="w-10" />
								<col className="w-[184px]" />
								<col className="w-[176px]" />
								<col className="w-[88px]" />
								<col className="w-[152px]" />
								<col className="w-[80px]" />
								<col className="w-[96px]" />
								<col className="w-[184px]" />
								<col />
							</colgroup>
							<TableHeader className="bg-surface-subtle/60">
								<TableRow className="hover:bg-transparent">
									<TableHead className="h-8" />
									<HeadCell>{copy.keyLabel}</HeadCell>
									<HeadCell>路由目标</HeadCell>
									<HeadCell>状态</HeadCell>
									<HeadCell>
										<button
											className="inline-flex items-center gap-0.5 text-muted-foreground hover:text-foreground"
											onClick={() => setAscending((value) => !value)}
											type="button"
										>
											{WINDOW_LABEL[period]} Token
											{ascending ? <ChevronUpIcon className="size-3" /> : <ChevronDownIcon className="size-3" />}
										</button>
									</HeadCell>
									<HeadCell className="text-right">{WINDOW_LABEL[period]}成本</HeadCell>
									<HeadCell className="text-right">请求 · 成功率</HeadCell>
									<HeadCell>最近使用</HeadCell>
									<HeadCell className="text-right">操作</HeadCell>
								</TableRow>
							</TableHeader>
							{visible.map((row) => (
								<IdentityRow kindTokens={kindTokens} key={row.entry.id} loading={!usage.data} row={row} period={period} />
							))}
							{visible.length === 0 ? (
								<TableBody>
									<TableRow className="hover:bg-transparent">
										<TableCell className="py-10 text-center text-sm text-muted-foreground" colSpan={9}>
											没有匹配的{copy.noun}
										</TableCell>
									</TableRow>
								</TableBody>
							) : null}
						</Table>
					</div>
				)}
			</Card>
		</div>
	);
}

function HeadCell({ children, className }: { children?: React.ReactNode; className?: string }) {
	return <TableHead className={cn("h-8 px-2 text-[11.5px] font-medium text-faint", className)}>{children}</TableHead>;
}

function IdentityRow({ row, period, kindTokens, loading }: { row: Row; period: Period; kindTokens: number; loading: boolean }) {
	const { busy, now, notify, openApiKeyEditor, openProxyEditor, toggleApiKey, toggleProxy, deleteApiKey, deleteProxy } = useAdmin();
	const [open, setOpen] = useState(false);
	const [revealed, setRevealed] = useState(false);
	const { entry, route, stats } = row;
	const lastAt = stats?.lastAt ?? row.lastSeen?.lastAt ?? null;
	const lastAgent = stats?.lastUserAgent || row.lastSeen?.userAgent || "";
	const pending = busy.has(`${entry.kind}:${entry.id}`);
	const dim = entry.enabled ? undefined : "opacity-55";
	const active = lastAt !== null && lastAt >= now - DAY_MS;
	const noun = entry.kind === "api_key" ? "API Key" : "下游账户";

	function edit() {
		if (entry.raw && entry.kind === "api_key") openApiKeyEditor(entry.raw as Parameters<typeof openApiKeyEditor>[0]);
		else openProxyEditor(entry.raw as Parameters<typeof openProxyEditor>[0]);
	}
	function toggle() {
		if (entry.kind === "api_key") void toggleApiKey(entry.raw as Parameters<typeof toggleApiKey>[0]);
		else void toggleProxy(entry.raw as Parameters<typeof toggleProxy>[0]);
	}
	function remove() {
		if (entry.kind === "api_key") void deleteApiKey(entry.raw as Parameters<typeof deleteApiKey>[0]);
		else void deleteProxy(entry.raw as Parameters<typeof deleteProxy>[0]);
	}
	async function copyCredential() {
		try {
			await navigator.clipboard.writeText(entry.credential);
			notify(entry.kind === "api_key" ? "Key 已复制。" : "account_id 已复制。");
		} catch {
			notify("复制失败，请手动复制。", "error");
		}
	}

	const credential = entry.kind === "api_key" && !revealed ? maskSecret(entry.credential) : entry.credential;
	const success = stats ? successRate(stats) : null;

	return (
		<Collapsible asChild onOpenChange={setOpen} open={open}>
			<TableBody className="[&_tr:last-child]:border-b">
				<TableRow className={cn("h-[60px] bg-card hover:bg-surface-subtle/50", open && "border-b-0")}>
					<TableCell className="px-1">
						<CollapsibleTrigger asChild>
							<Button aria-label={open ? "收起明细" : "展开明细"} size="icon-xs" type="button" variant="ghost">
								{open ? <ChevronUpIcon /> : <ChevronDownIcon />}
							</Button>
						</CollapsibleTrigger>
					</TableCell>
					<TableCell className={cn("px-2", dim)}>
						<div className="truncate text-[13.5px] leading-5 font-semibold" title={entry.name}>
							{entry.name}
						</div>
						<div className="flex min-w-0 items-center gap-0.5">
							<span className="truncate font-mono text-[11.5px] leading-4 text-muted-foreground" title={entry.kind === "api_key" ? undefined : entry.credential}>
								{credential}
							</span>
							{entry.kind === "api_key" ? (
								<Button aria-label={revealed ? "隐藏 Key" : "显示 Key"} className="text-muted-foreground" onClick={() => setRevealed((value) => !value)} size="icon-xs" type="button" variant="ghost">
									{revealed ? <EyeOffIcon /> : <EyeIcon />}
								</Button>
							) : null}
							<Button aria-label="复制" className="text-muted-foreground" onClick={() => void copyCredential()} size="icon-xs" type="button" variant="ghost">
								<CopyIcon />
							</Button>
						</div>
					</TableCell>
					<TableCell className={cn("px-2", dim)}>
						<RouteTarget label={route.label} type={route.type} />
					</TableCell>
					<TableCell className="px-2">
						<label className="inline-flex items-center gap-2">
							<Switch aria-label={`${entry.enabled ? "停用" : "启用"} ${entry.name}`} checked={entry.enabled} disabled={pending} onCheckedChange={toggle} />
							<span className={cn("text-xs", entry.enabled ? "text-muted-foreground" : "text-faint")}>{entry.enabled ? "启用" : "停用"}</span>
						</label>
					</TableCell>
					<TableCell className={cn("px-2", dim)}>
						{loading ? (
							<Skeleton className="h-8 w-32" />
						) : (
							<div className="flex items-center gap-3">
								<div className="flex w-16 flex-col">
									<span className="text-[13px] leading-5 font-semibold tabular-nums">{stats ? formatTokens(stats.totalTokens) : "—"}</span>
									<span className="truncate text-[11px] leading-4 text-faint">
										{stats ? `占 ${formatPercent(ratio(stats.totalTokens, kindTokens))}` : `${WINDOW_LABEL[period]}无请求`}
									</span>
								</div>
								<SparkBars values={stats?.trend ?? [0, 0, 0, 0, 0, 0, 0]} />
							</div>
						)}
					</TableCell>
					<TableCell className={cn("px-2 text-right text-[13px] font-medium tabular-nums", dim)}>{stats ? formatCost(stats.costUsd) : "—"}</TableCell>
					<TableCell className={cn("px-2 text-right", dim)}>
						<div className="text-[13px] leading-5 font-medium tabular-nums">{formatCount(stats?.requests ?? 0)}</div>
						<div className={cn("text-[11px] leading-4", success !== null && success < 0.97 ? "text-warning" : "text-faint")}>
							{success === null ? "—" : `成功 ${formatPercent(success)}`}
						</div>
					</TableCell>
					<TableCell className={cn("px-2", dim)}>
						<div className="flex items-center gap-1.5">
							<span aria-hidden="true" className={cn("size-1.5 shrink-0 rounded-full", active ? "bg-success" : "bg-faint")} />
							<span className="text-[12.5px] leading-5 font-medium tabular-nums">{lastAt ? formatDateTime(lastAt) : "—"}</span>
						</div>
						<div className="truncate text-[11px] leading-4 text-faint">{lastAt ? clientSummary(lastAgent) || "—" : "从未使用"}</div>
					</TableCell>
					<TableCell className="px-2">
						<div className="flex justify-end gap-2">
							<Button disabled={pending} onClick={edit} size="sm" type="button" variant="outline">
								编辑
							</Button>
							<DeleteConfirmationDialog
								description={`删除后使用「${entry.name}」的请求将立即失效，路由分配也会移除。`}
								onConfirm={remove}
								title={`删除${noun}？`}
								trigger={
									<Button disabled={pending} size="sm" type="button" variant="destructive">
										删除
									</Button>
								}
							/>
						</div>
					</TableCell>
				</TableRow>
				<CollapsibleContent asChild>
					<tr>
						<td className="p-0" colSpan={9}>
							{open ? <IdentityDetail entry={entry} range={period} route={route} /> : null}
						</td>
					</tr>
				</CollapsibleContent>
			</TableBody>
		</Collapsible>
	);
}
