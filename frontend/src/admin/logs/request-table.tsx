import { Fragment, type ReactNode } from "react";
import { ChevronDownIcon, ChevronUpIcon, CopyIcon, ExternalLinkIcon, Link2Icon } from "lucide-react";

import type { UsageEvent } from "@/admin-api";
import { LegendItem, StackedBar } from "@/components/app/bars";
import { CodeChip, LogKind, LogStatus } from "@/components/app/badges";
import { TOKEN_PARTS, VIZ, identityTypeLabel, tokenSplit } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { formatAgo, formatClock, formatCost, formatCount, formatDay, formatPercent, formatSeconds, formatTokens, ratio, startOfLocalDay } from "@/lib/format";
import { cn } from "@/lib/utils";

import { COLUMNS, type ColumnKey } from "./columns";
import { copyText, generationMs, isDowngraded, isSuccess, outputSpeed, sessionText, tierText, transportText, upstreamName } from "./event-utils";

/** Figma "card · 请求" table: one row per request, expandable to a summary. */
export function RequestTable({
	events,
	columns,
	expandedId,
	onToggle,
	onOpen,
	onSessionFilter,
	modelColor,
	slowThresholdMs,
	now,
	sessionSummary,
	renderExpanded,
	columnLabels,
}: {
	columnLabels?: Partial<Record<ColumnKey, string>>;
	events: UsageEvent[];
	/** Size of the event's session tree across the range. */
	sessionSummary: (event: UsageEvent) => { total: number; text: string } | null;
	/** Replaces the default row summary (user inline details). */
	renderExpanded?: (event: UsageEvent) => ReactNode;
	columns: ReadonlySet<ColumnKey>;
	expandedId: number | null;
	onToggle: (id: number) => void;
	onOpen: (event: UsageEvent) => void;
	onSessionFilter: (sessionId: string) => void;
	modelColor: (model: string) => string;
	slowThresholdMs: number | null;
	now: number;
}) {
	const maxLatency = Math.max(1, ...events.map((event) => event.latencyMs));
	const maxTokens = Math.max(1, ...events.map((event) => event.totalTokens));
	const visible = COLUMNS.filter((column) => columns.has(column.key));
	const today = startOfLocalDay(now);
	return (
		<Table className="min-w-[960px]">
			<TableHeader>
				<TableRow className="hover:bg-transparent">
					<TableHead className="h-10 w-20 px-2 text-xs font-normal text-muted-foreground">
						<span className="inline-flex items-center gap-1">
							时间 <ChevronDownIcon className="size-3" />
						</span>
					</TableHead>
					{visible.map((column) => (
						<TableHead className={cn("h-10 px-2 text-xs font-normal text-muted-foreground", column.key === "cost" && "text-right")} key={column.key}>
							{columnLabels?.[column.key] ?? column.label}
						</TableHead>
					))}
					<TableHead className="w-10" />
				</TableRow>
			</TableHeader>
			<TableBody>
				{events.map((event) => {
					const expanded = expandedId === event.id;
					const failedEarly = !isSuccess(event) && event.totalTokens === 0;
					return (
						<Fragment key={event.id}>
							<TableRow
								aria-expanded={expanded}
								className={cn("cursor-pointer", expanded && "border-b-0 bg-surface-subtle/70 hover:bg-surface-subtle/70")}
								onClick={() => onToggle(event.id)}
							>
								<TableCell className="px-2 py-3">
									<div className="flex flex-col gap-1">
										<span className="text-[13px] leading-5 font-medium tabular-nums">
											{event.recordedAt < today ? `${formatDay(event.recordedAt)} ` : ""}
											{formatClock(event.recordedAt)}
										</span>
										<span className="text-[11.5px] leading-4 text-faint">{formatAgo(event.recordedAt, now)}</span>
									</div>
								</TableCell>
								{visible.map((column) => {
									if (failedEarly && column.key === "tokens") {
										return (
											<TableCell className="px-2" colSpan={columns.has("cost") ? 2 : 1} key={column.key}>
												{event.errorCode ? <CodeChip>{event.errorCode}</CodeChip> : <span className="text-faint">—</span>}
											</TableCell>
										);
									}
									if (failedEarly && column.key === "cost" && columns.has("tokens")) return null;
									return (
										<TableCell className={cn("px-2 py-3", column.key === "cost" && "text-right")} key={column.key}>
											<Cell column={column.key} event={event} maxLatency={maxLatency} maxTokens={maxTokens} modelColor={modelColor} slowThresholdMs={slowThresholdMs} />
										</TableCell>
									);
								})}
								<TableCell className="px-2 text-center">
									{expanded ? <ChevronUpIcon className="inline size-4 text-muted-foreground" /> : <ChevronDownIcon className="inline size-4 text-muted-foreground" />}
								</TableCell>
							</TableRow>
							{expanded ? (
								<TableRow className="bg-surface-subtle/70 hover:bg-surface-subtle/70">
									<TableCell className="px-2 pt-0 pb-4 whitespace-normal" colSpan={visible.length + 2}>
										{renderExpanded ? (
											renderExpanded(event)
										) : (
											<RowSummary event={event} onOpen={() => onOpen(event)} onSessionFilter={onSessionFilter} session={sessionSummary(event)} />
										)}
									</TableCell>
								</TableRow>
							) : null}
						</Fragment>
					);
				})}
			</TableBody>
		</Table>
	);
}

function Cell({
	column,
	event,
	maxLatency,
	maxTokens,
	modelColor,
	slowThresholdMs,
}: {
	column: ColumnKey;
	event: UsageEvent;
	maxLatency: number;
	maxTokens: number;
	modelColor: (model: string) => string;
	slowThresholdMs: number | null;
}) {
	switch (column) {
		case "identity":
			return (
				<TwoLine primary={event.userName || "—"} secondary={event.credentialType ? `${identityTypeLabel(event.credentialType)} · ${event.credentialName}` : ""} />
			);
		case "model":
			return (
				<div className="flex flex-col gap-1">
					<span className="flex items-center gap-1.5">
						<span aria-hidden="true" className="size-1.5 shrink-0 rounded-[2px]" style={{ background: event.model ? modelColor(event.model) : VIZ.gray }} />
						<span className="font-mono text-[12.5px] leading-5">{event.model || "—"}</span>
					</span>
					<span className="flex items-center gap-1 pl-3 text-[11.5px] leading-4 text-faint">
						{event.reasoningEffort || "—"} ·
						{isDowngraded(event) ? (
							<span className="rounded bg-warning-soft px-1 font-semibold text-warning">{tierText(event)}</span>
						) : (
							<span className={cn(event.serviceTier === "priority" && "font-semibold text-muted-foreground")}>{tierText(event)}</span>
						)}
					</span>
				</div>
			);
		case "upstream":
			return <TwoLine primary={upstreamName(event)} secondary={event.codexAccountName || "—"} />;
		case "session":
			return (
				<div className="flex flex-col items-start gap-1">
					{event.sessionKind ? <LogKind kind={event.sessionKind} /> : <span className="text-[13px] leading-5 text-faint">—</span>}
					<span className="text-[11.5px] leading-4 text-faint">{transportText(event)}</span>
				</div>
			);
		case "status":
			return (
				<div className="flex flex-col gap-1">
					<LogStatus status={event.status} />
					<span className="font-mono text-[11.5px] leading-4 text-faint">{event.statusCode ?? "—"}</span>
				</div>
			);
		case "latency": {
			const slow = slowThresholdMs !== null && event.latencyMs > slowThresholdMs && isSuccess(event);
			const ttft = event.ttftMs ?? 0;
			return (
				<div className="flex w-[112px] flex-col gap-1">
					<StackedBar
						height={6}
						segments={[
							{ key: "ttft", value: ttft, color: slow ? VIZ.amber : VIZ.blue, label: "首字" },
							{ key: "gen", value: event.latencyMs - ttft, color: slow ? VIZ.amber : VIZ.blueSoft, label: "生成" },
						]}
						total={maxLatency}
					/>
					<span className={cn("text-xs leading-4 tabular-nums", slow ? "font-semibold text-warning" : "text-muted-foreground")}>
						{event.ttftMs === null ? "—" : formatSeconds(event.ttftMs, 1)} · {formatSeconds(event.latencyMs, 1)}
						{slow ? <span className="ml-1 rounded bg-warning-soft px-1">&gt; P95</span> : null}
					</span>
				</div>
			);
		}
		case "tokens": {
			const split = tokenSplit(event);
			return (
				<div className="flex flex-col gap-1">
					<div style={{ width: `${Math.max(4, (event.totalTokens / maxTokens) * 104)}px` }}>
						<StackedBar
							className="bg-transparent"
							height={6}
							segments={[
								{ key: "input", value: split.input, color: TOKEN_PARTS.input.color },
								{ key: "cached", value: split.cached, color: TOKEN_PARTS.cached.color },
								{ key: "output", value: split.output + split.reasoning, color: TOKEN_PARTS.output.color },
							]}
						/>
					</div>
					<span className="text-[13px] leading-5 font-medium tabular-nums">{formatTokens(event.totalTokens)}</span>
				</div>
			);
		}
		case "cost":
			return <span className="text-[13px] leading-5 font-medium tabular-nums">{event.totalTokens ? formatCost(event.costUsd) : "—"}</span>;
		case "requestId":
			return <span className="font-mono text-xs">{event.requestId || "—"}</span>;
		case "client":
			return <span className="block max-w-48 truncate text-xs text-muted-foreground" title={event.userAgent}>{event.userAgent || "—"}</span>;
		case "ip":
			return <TwoLine mono primary={event.clientIp || "—"} secondary={event.xForwardedFor || ""} />;
		case "endpoint":
			return <span className="font-mono text-xs text-muted-foreground">{event.endpoint || "—"}</span>;
	}
}

function TwoLine({ primary, secondary, mono = false }: { primary: string; secondary: string; mono?: boolean }) {
	return (
		<div className="flex max-w-44 flex-col gap-1">
			<span className={cn("truncate text-[13px] leading-5", mono ? "font-mono text-xs" : "font-medium")}>{primary}</span>
			{secondary ? <span className="truncate text-[11.5px] leading-4 text-faint">{secondary}</span> : null}
		</div>
	);
}

/** Figma "row-summary": timing bar, key fields and actions. */
function RowSummary({
	event,
	session,
	onOpen,
	onSessionFilter,
}: {
	event: UsageEvent;
	session: { total: number; text: string } | null;
	onOpen: () => void;
	onSessionFilter: (sessionId: string) => void;
}) {
	const generation = generationMs(event);
	const speed = outputSpeed(event);
	const cacheRate = ratio(event.cachedInputTokens, event.inputTokens);
	const sessionFilter = event.parentSessionId || event.sessionId;
	return (
		<div className="flex flex-col gap-4 rounded-lg border bg-card p-4">
			<div className="flex flex-col gap-2">
				<div className="flex flex-wrap items-center gap-x-4 gap-y-1">
					<span className="text-xs font-bold">耗时</span>
					<LegendItem color={VIZ.blue} label="首字" value={formatSeconds(event.ttftMs, 1)} />
					<LegendItem color={VIZ.blueSoft} label="生成" value={formatSeconds(generation, 1)} />
					<span className="flex-1" />
					<span className="text-[11.5px] text-faint tabular-nums">
						总计 {formatSeconds(event.latencyMs, 1)}
						{speed !== null ? ` · 输出 ${speed.toFixed(1)} t/s` : ""} · {formatClock(event.recordedAt - event.latencyMs)} → {formatClock(event.recordedAt)}
					</span>
				</div>
				<StackedBar
					segments={[
						{ key: "ttft", value: event.ttftMs ?? 0, color: VIZ.blue },
						{ key: "gen", value: generation ?? event.latencyMs, color: VIZ.blueSoft },
					]}
				/>
			</div>
			<div className="grid grid-cols-2 gap-4 md:grid-cols-5">
				<Field label="请求 ID" mono value={event.requestId || "—"} />
				<Field label="会话" mono value={sessionText(event.sessionId, event.sessionKind)} />
				<Field label="父会话" mono value={sessionText(event.parentSessionId, event.parentSessionId ? "main" : "")} />
				<Field
					label="速度档位"
					value={isDowngraded(event) ? `请求 ${tierText(event).replace("→", " → 实际 ")}` : tierText(event)}
					valueClassName={isDowngraded(event) ? "font-semibold text-warning" : undefined}
				/>
				<Field
					label="Token"
					value={`${formatTokens(event.inputTokens)} 入 · ${formatCount(event.outputTokens)} 出 · 缓存 ${formatPercent(cacheRate, 0)}`}
				/>
			</div>
			<div className="flex flex-wrap items-center gap-2 border-t pt-4">
				<Button onClick={onOpen} size="xs" type="button">
					<ExternalLinkIcon data-icon="inline-start" />
					打开完整详情
				</Button>
				{sessionFilter ? (
					<Button onClick={() => onSessionFilter(sessionFilter)} size="xs" type="button" variant="outline">
						<Link2Icon data-icon="inline-start" />
						查看同会话请求{session ? ` · ${session.total}` : ""}
					</Button>
				) : null}
				<Button disabled={!event.requestId} onClick={() => copyText(event.requestId, "request_id")} size="xs" type="button" variant="outline">
					<CopyIcon data-icon="inline-start" />
					复制 request_id
				</Button>
				{session?.text ? <span className="ml-auto text-[11.5px] text-faint">同会话：{session.text}</span> : null}
			</div>
		</div>
	);
}

function Field({ label, value, mono = false, valueClassName }: { label: string; value: string; mono?: boolean; valueClassName?: string | undefined }) {
	return (
		<div className="flex min-w-0 flex-col gap-1">
			<span className="text-[11.5px] leading-4 text-faint">{label}</span>
			<span className={cn("text-xs leading-4 break-all", mono && "font-mono", valueClassName)}>{value}</span>
		</div>
	);
}
