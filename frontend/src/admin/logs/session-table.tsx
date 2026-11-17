import { Fragment, useState } from "react";
import { ArrowRightIcon, ChevronDownIcon, ChevronRightIcon } from "lucide-react";

import type { OutcomeCounts, SessionChild, SessionGroup, TimelinePoint, UsageEvent, UsageSessionKind } from "@/admin-api";
import { LegendItem, StackedBar } from "@/components/app/bars";
import { LogKind, LogStatus } from "@/components/app/badges";
import { VIZ, sessionKindColor } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { formatClock, formatCost, formatDay, formatDuration, formatSeconds, formatTokens, shortId } from "@/lib/format";
import { cn } from "@/lib/utils";

import { transportText } from "./event-utils";

const KIND_ORDER: UsageSessionKind[] = ["main", "subagent", "fork", "compaction"];
const KIND_SHORT: Record<string, string> = { main: "主", subagent: "子代理", fork: "分叉", compaction: "压缩" };

/** Figma "按会话分组": session trees with child sessions and the root's requests. */
export function SessionTable({
	groups,
	onOpen,
	onSessionFilter,
	showIdentity,
}: {
	groups: SessionGroup[];
	onOpen: (event: UsageEvent) => void;
	/** Shows every request of a session in the request view. */
	onSessionFilter: (sessionId: string) => void;
	showIdentity: boolean;
}) {
	const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set(groups[0] ? [groupKey(groups[0])] : []));
	const toggle = (key: string) =>
		setOpen((current) => {
			const next = new Set(current);
			if (next.has(key)) next.delete(key);
			else next.add(key);
			return next;
		});
	return (
		<Table className="min-w-[960px]">
			<TableHeader>
				<TableRow className="hover:bg-transparent">
					<TableHead className="w-8" />
					<TableHead className="h-10 px-2 text-xs font-normal text-muted-foreground">会话 / 请求</TableHead>
					<TableHead className="h-10 px-2 text-xs font-normal text-muted-foreground">{showIdentity ? "调用身份 · 路由" : "路由"}</TableHead>
					<TableHead className="h-10 px-2 text-xs font-normal text-muted-foreground">构成 · 状态</TableHead>
					<TableHead className="h-10 px-2 text-xs font-normal text-muted-foreground">时间线</TableHead>
					<TableHead className="h-10 px-2 text-right text-xs font-normal text-muted-foreground">请求 · 耗时</TableHead>
					<TableHead className="h-10 px-2 text-right text-xs font-normal text-muted-foreground">Token</TableHead>
					<TableHead className="h-10 px-2 text-right text-xs font-normal text-muted-foreground">成本</TableHead>
				</TableRow>
			</TableHeader>
			<TableBody>
				{groups.map((group) => {
					const key = groupKey(group);
					const expanded = open.has(key);
					const start = group.firstAt;
					const end = Math.max(group.lastAt, start + 1);
					return (
						<Fragment key={key}>
							<TableRow aria-expanded={expanded} className="cursor-pointer bg-surface-subtle/70" onClick={() => toggle(key)}>
								<TableCell className="pl-2">
									{expanded ? <ChevronDownIcon className="size-4 text-muted-foreground" /> : <ChevronRightIcon className="size-4 text-muted-foreground" />}
								</TableCell>
								<TableCell className="px-2 py-3">
									<div className="flex flex-col gap-1">
										<span className="flex items-center gap-2">
											<span className="font-mono text-[12.5px] leading-5 font-bold">{group.sessionId ? shortId(group.sessionId, 13) : "无 session_id"}</span>
											{group.sessionId ? <LogKind kind={rootKind(group)} /> : null}
										</span>
										<span className="text-[11.5px] leading-4 text-faint tabular-nums">
											{group.sessionId
												? `${formatDay(group.firstAt)} ${formatClock(group.firstAt, false)} – ${formatClock(group.lastAt, false)} · ${formatDuration(group.lastAt - group.firstAt)}`
												: `${formatClock(group.firstAt, false)} – ${formatClock(group.lastAt, false)} · 按调用身份聚合`}
										</span>
									</div>
								</TableCell>
								<TableCell className="px-2">
									<div className="flex max-w-44 flex-col gap-1">
										{showIdentity ? <span className="truncate text-[13px] leading-5 font-medium">{group.identityName || "—"}</span> : null}
										<span className={cn("truncate leading-4", showIdentity ? "text-[11.5px] text-faint" : "text-[13px] leading-5")}>{routeText(group)}</span>
									</div>
								</TableCell>
								<TableCell className="px-2">
									<div className="flex w-[152px] flex-col gap-1">
										<StackedBar
											height={6}
											segments={KIND_ORDER.map((kind) => ({
												key: kind,
												value: group.kinds[kind] ?? 0,
												color: sessionKindColor(kind),
											}))}
										/>
										<span className="text-[11.5px] leading-4 text-muted-foreground">{kindText(group.kinds)}</span>
									</div>
								</TableCell>
								<TableCell className="px-2">{group.sessionId ? <Timeline end={end} points={group.timeline} split start={start} /> : <span className="text-faint">—</span>}</TableCell>
								<TableCell className="px-2 text-right">
									<div className="flex flex-col items-end gap-1">
										<span className="text-[13px] leading-5 font-semibold tabular-nums">{group.requests}</span>
										<OutcomeText outcomes={group} />
									</div>
								</TableCell>
								<TableCell className="px-2 text-right text-[13px] font-medium tabular-nums">{formatTokens(group.totalTokens)}</TableCell>
								<TableCell className="px-2 text-right text-[13px] font-medium tabular-nums">{formatCost(group.costUsd)}</TableCell>
							</TableRow>
							{expanded ? (
								<>
									{group.children.map((child) => (
										<ChildRow child={child} end={end} key={child.sessionId} onSessionFilter={onSessionFilter} start={start} />
									))}
									{group.events.map((event) => (
										<EventRow end={end} event={event} key={event.id} onOpen={onOpen} start={start} />
									))}
									{group.eventCount > group.events.length && group.sessionId ? (
										<TableRow className="hover:bg-transparent">
											<TreeLine />
											<TableCell className="px-2 py-2" colSpan={7}>
												<Button onClick={() => onSessionFilter(group.sessionId)} size="xs" type="button" variant="link">
													还有 {group.eventCount - group.events.length} 条请求，查看全部
													<ArrowRightIcon data-icon="inline-end" />
												</Button>
											</TableCell>
										</TableRow>
									) : null}
								</>
							) : null}
						</Fragment>
					);
				})}
			</TableBody>
		</Table>
	);
}

export function SessionLegend() {
	return (
		<div className="flex flex-wrap items-center gap-4">
			{KIND_ORDER.map((kind) => (
				<LegendItem color={sessionKindColor(kind)} key={kind} label={KIND_SHORT[kind] === "主" ? "主会话" : (KIND_SHORT[kind] ?? "")} />
			))}
			<LegendItem color={VIZ.danger} label="失败 / 中断" />
		</div>
	);
}

function ChildRow({ child, start, end, onSessionFilter }: { child: SessionChild; start: number; end: number; onSessionFilter: (id: string) => void }) {
	return (
		<TableRow className="cursor-pointer" onClick={() => onSessionFilter(child.sessionId)}>
			<TreeLine />
			<TableCell className="px-2 py-2">
				<div className="flex flex-col gap-1">
					<span className="flex items-center gap-1.5">
						<ChevronRightIcon className="size-3.5 text-muted-foreground" />
						<LogKind kind={child.sessionKind} />
						<span className="font-mono text-xs font-medium">{shortId(child.sessionId)}</span>
					</span>
					<span className="text-[11.5px] leading-4 text-faint tabular-nums">
						{formatClock(child.firstAt)} – {formatClock(child.lastAt)} · {child.requests} 请求
					</span>
				</div>
			</TableCell>
			<TableCell className="max-w-44 truncate px-2 text-xs text-muted-foreground">{shortAccounts(child.accounts)}</TableCell>
			<TableCell className="px-2 text-xs text-muted-foreground">
				{child.completed} 已完成
				{child.fastDowngraded ? ` · ${child.fastDowngraded} 档位降级` : ""}
				{child.failed + child.interrupted ? ` · ${child.failed + child.interrupted} 失败` : ""}
			</TableCell>
			<TableCell className="px-2">
				<Timeline compact end={end} points={child.timeline} start={start} kind={child.sessionKind} />
			</TableCell>
			<TableCell className="px-2 text-right text-[12.5px] font-semibold tabular-nums">{child.requests}</TableCell>
			<TableCell className="px-2 text-right text-[12.5px] font-medium tabular-nums">{formatTokens(child.totalTokens)}</TableCell>
			<TableCell className="px-2 text-right text-[12.5px] font-medium tabular-nums">{formatCost(child.costUsd)}</TableCell>
		</TableRow>
	);
}

function EventRow({ event, start, end, onOpen }: { event: UsageEvent; start: number; end: number; onOpen: (event: UsageEvent) => void }) {
	const position = ((event.recordedAt - start) / (end - start)) * 100;
	return (
		<TableRow className="cursor-pointer" onClick={() => onOpen(event)}>
			<TreeLine />
			<TableCell className="px-2 py-2 pl-8">
				<div className="flex flex-col gap-1">
					<span className="text-[13px] leading-5 font-medium tabular-nums">{formatClock(event.recordedAt)}</span>
					<span className="flex items-center gap-1.5 text-[11.5px] leading-4 text-faint">
						<span className="font-mono">{event.model || "—"}</span>
						{event.reasoningEffort ? `· ${event.reasoningEffort}` : null}
					</span>
				</div>
			</TableCell>
			<TableCell className="px-2">
				<div className="flex max-w-44 flex-col gap-1">
					<span className="truncate text-xs">{event.codexAccountName || "—"}</span>
					<span className="text-[11.5px] leading-4 text-faint">
						{transportText(event)}
						{event.endpoint.endsWith("/compact") ? " · /compact" : ""}
					</span>
				</div>
			</TableCell>
			<TableCell className="px-2">
				<LogStatus status={event.status} />
			</TableCell>
			<TableCell className="px-2">
				<div className="relative h-3 w-[232px]">
					<div className="absolute inset-x-0 top-1/2 h-px bg-border" />
					<div className="absolute top-0.5 h-2 w-[3px] rounded-[1px]" style={{ left: `${Math.min(99, position)}%`, background: sessionKindColor(event.sessionKind || "main") }} />
				</div>
			</TableCell>
			<TableCell className="px-2 text-right text-[13px] tabular-nums">{formatSeconds(event.latencyMs, 1)}</TableCell>
			<TableCell className="px-2 text-right text-[13px] font-medium tabular-nums">{formatTokens(event.totalTokens)}</TableCell>
			<TableCell className="px-2 text-right text-[13px] font-medium tabular-nums">{event.totalTokens ? formatCost(event.costUsd) : "—"}</TableCell>
		</TableRow>
	);
}

function TreeLine() {
	return (
		<TableCell className="relative w-8 p-0">
			<span aria-hidden="true" className="absolute inset-y-0 left-[15px] w-px bg-border-strong" />
		</TableCell>
	);
}

/**
 * Request ticks across the session's span. With `split`, main-session ticks sit
 * on the upper lane and child requests on the lower one.
 */
function Timeline({
	points,
	start,
	end,
	split = false,
	compact = false,
	kind,
}: {
	points: TimelinePoint[];
	start: number;
	end: number;
	split?: boolean;
	compact?: boolean;
	kind?: UsageSessionKind;
}) {
	const span = end - start;
	if (compact && kind) {
		const first = points[0]?.[0] ?? start;
		const last = points[points.length - 1]?.[0] ?? first;
		const left = ((first - start) / span) * 100;
		const width = Math.max(1, ((last - first) / span) * 100);
		const failed = points.some((point) => point[2] === "failed" || point[2] === "interrupted");
		return (
			<div className="relative h-3 w-[232px]">
				<div className="absolute inset-x-0 top-1/2 h-px bg-border" />
				<div className="absolute top-0.5 h-2 rounded-[1px] opacity-35" style={{ left: `${left}%`, width: `${width}%`, background: failed ? VIZ.danger : sessionKindColor(kind) }} />
			</div>
		);
	}
	return (
		<div className={cn("relative w-[232px]", split ? "h-5" : "h-3")}>
			<div className="absolute inset-x-0 top-1/2 h-px bg-border" />
			{points.slice(-400).map(([at, , status], index) => {
				const failed = status === "failed" || status === "interrupted";
				return (
					<span
						className="absolute h-[7px] w-[3px] rounded-[1px]"
						key={index}
						style={{
							left: `${Math.min(98.5, ((at - start) / span) * 100)}%`,
							top: split ? (index % 2 ? 12 : 1) : 2,
							background: failed ? VIZ.danger : VIZ.blue,
						}}
					/>
				);
			})}
		</div>
	);
}

function OutcomeText({ outcomes }: { outcomes: OutcomeCounts }) {
	const parts = [
		outcomes.failed ? `${outcomes.failed} 失败` : "",
		outcomes.interrupted ? `${outcomes.interrupted} 中断` : "",
		outcomes.cancelled ? `${outcomes.cancelled} 取消` : "",
		outcomes.incomplete ? `${outcomes.incomplete} 未完成` : "",
	].filter(Boolean);
	if (!parts.length) return <span className="text-[11.5px] leading-4 text-success">全部成功</span>;
	return <span className={cn("text-[11.5px] leading-4", outcomes.failed || outcomes.interrupted ? "text-danger" : "text-warning")}>{parts.join(" · ")}</span>;
}

function groupKey(group: SessionGroup): string {
	return group.sessionId || `identity:${group.identityType}:${group.identityId}`;
}

function rootKind(group: SessionGroup): UsageSessionKind {
	return group.events[0]?.sessionKind || (group.kinds.main ? "main" : (KIND_ORDER.find((kind) => group.kinds[kind]) ?? ""));
}

function kindText(kinds: SessionGroup["kinds"]): string {
	const parts = KIND_ORDER.filter((kind) => kinds[kind]).map((kind) => `${KIND_SHORT[kind]} ${kinds[kind]}`);
	return parts.length ? parts.join(" · ") : "未关联会话";
}

function routeText(group: SessionGroup): string {
	const accounts = group.accounts.length === 1 ? (group.accounts[0] ?? "") : group.accounts.length > 1 ? `${group.accounts.length} 个账户` : "—";
	return group.accountGroupName ? `${group.accountGroupName} · ${accounts}` : accounts;
}

function shortAccounts(accounts: string[]): string {
	return accounts.map((account) => account.split("@")[0]).join(" / ") || "—";
}
