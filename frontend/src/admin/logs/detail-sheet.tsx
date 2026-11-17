import type { ReactNode } from "react";
import {
	ChevronDownIcon,
	ChevronUpIcon,
	CoinsIcon,
	CopyIcon,
	FileJsonIcon,
	GitBranchIcon,
	Globe2Icon,
	Link2Icon,
	ListFilterIcon,
	MessageSquareMoreIcon,
	ScrollTextIcon,
	TimerIcon,
	XIcon,
} from "lucide-react";

import type { UsageEvent } from "@/admin-api";
import { StackedBar } from "@/components/app/bars";
import { LogKind, LogStatus, Tag } from "@/components/app/badges";
import { TOKEN_PARTS, VIZ, identityTypeLabel, tierLabel } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Sheet, SheetClose, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { formatCost, formatCount, formatDuration, formatPercent, formatSeconds, formatTimestamp, ratio } from "@/lib/format";
import { cn } from "@/lib/utils";

import { copyText, generationMs, isDowngraded, outputSpeed, transportText, upstreamName } from "./event-utils";

/** Figma "request-sheet": every recorded field of one request, with prev/next. */
export function RequestSheet({
	event,
	index,
	total,
	onClose,
	onStep,
	onSessionFilter,
	onIdentityFilter,
	sessionSummary,
}: {
	event: UsageEvent | null;
	sessionSummary: { total: number; text: string } | null;
	/** Position among the loaded requests, 0-based. */
	index: number;
	total: number;
	onClose: () => void;
	onStep: (delta: -1 | 1) => void;
	onSessionFilter: (sessionId: string) => void;
	/** Omitted in the user scope. */
	onIdentityFilter?: (event: UsageEvent) => void;
}) {
	return (
		<Sheet onOpenChange={(open) => (open ? undefined : onClose())} open={event !== null}>
			<SheetContent className="w-full gap-0 p-0 data-[side=right]:sm:max-w-[720px]" showCloseButton={false} side="right">
				{event ? (
					<SheetBody
						event={event}
						index={index}
						onIdentityFilter={onIdentityFilter}
						onSessionFilter={onSessionFilter}
						onStep={onStep}
						session={sessionSummary}
						total={total}
					/>
				) : null}
			</SheetContent>
		</Sheet>
	);
}

function SheetBody({
	event,
	index,
	total,
	onStep,
	onSessionFilter,
	onIdentityFilter,
	session,
}: {
	event: UsageEvent;
	session: { total: number; text: string } | null;
	index: number;
	total: number;
	onStep: (delta: -1 | 1) => void;
	onSessionFilter: (sessionId: string) => void;
	onIdentityFilter?: ((event: UsageEvent) => void) | undefined;
}) {
	const downgraded = isDowngraded(event);
	const generation = generationMs(event);
	const speed = outputSpeed(event);
	const uncached = Math.max(0, event.inputTokens - event.cachedInputTokens);
	const sessionFilter = event.parentSessionId || event.sessionId;
	const summary = [
		formatTimestamp(event.recordedAt).slice(5, 19).replace("-", "/"),
		event.userName ? `${event.userName} → ${upstreamName(event)}${event.codexAccountName ? ` / ${event.codexAccountName}` : ""}` : null,
		event.model || null,
		formatSeconds(event.latencyMs, 1),
		event.totalTokens ? formatCost(event.costUsd) : null,
	]
		.filter(Boolean)
		.join(" · ");
	return (
		<>
			<div className="flex flex-col gap-3 border-b px-6 pt-5 pb-4">
				<div className="flex flex-wrap items-center gap-2">
					<SheetTitle className="mr-1 text-lg leading-6 font-bold tracking-[-0.015em]">请求详情</SheetTitle>
					<LogStatus status={event.status} />
					<span className="font-mono text-[11.5px] text-faint">{event.statusCode ? `HTTP ${event.statusCode}` : "无响应码"}</span>
					<LogKind kind={event.sessionKind} />
					{downgraded ? <Tag tone="warning">档位降级</Tag> : null}
					<span className="flex-1" />
					<span className="text-[11.5px] text-faint tabular-nums">
						第 {index + 1} / {total} 条
					</span>
					<Button aria-label="上一条" disabled={index <= 0} onClick={() => onStep(-1)} size="icon-sm" type="button" variant="outline">
						<ChevronUpIcon />
					</Button>
					<Button aria-label="下一条" disabled={index >= total - 1} onClick={() => onStep(1)} size="icon-sm" type="button" variant="outline">
						<ChevronDownIcon />
					</Button>
					<SheetClose asChild>
						<Button aria-label="关闭" size="icon-sm" type="button" variant="ghost">
							<XIcon />
						</Button>
					</SheetClose>
				</div>
				<SheetDescription className="text-[12.5px] leading-5 text-muted-foreground">{summary}</SheetDescription>
				<div className="flex flex-wrap gap-2">
					{sessionFilter ? (
						<Button onClick={() => onSessionFilter(sessionFilter)} size="sm" type="button">
							<Link2Icon data-icon="inline-start" />
							查看同会话请求{session ? ` · ${session.total}` : ""}
						</Button>
					) : null}
					<Button disabled={!event.requestId} onClick={() => copyText(event.requestId, "request_id")} size="sm" type="button" variant="outline">
						<CopyIcon data-icon="inline-start" />
						复制 request_id
					</Button>
					<Button onClick={() => copyText(JSON.stringify(event, null, 2), "JSON")} size="sm" type="button" variant="outline">
						<FileJsonIcon data-icon="inline-start" />
						复制 JSON
					</Button>
					{onIdentityFilter && event.userId ? (
						<Button onClick={() => onIdentityFilter(event)} size="sm" type="button" variant="ghost">
							<ListFilterIcon data-icon="inline-start" />
							按此用户筛选
						</Button>
					) : null}
				</div>
			</div>
			<ScrollArea className="min-h-0 flex-1">
				<div className="flex flex-col gap-4 px-6 pt-4 pb-6">
					<Section icon={<ScrollTextIcon />} title="请求">
						<Field label="请求 ID" mono value={event.requestId} />
						<Field label="响应 ID" mono value={event.responseId} />
						<Field label="记录时间" value={formatTimestamp(event.recordedAt)} />
						<Field label="端点" mono value={event.endpoint} />
						<Field label="状态" value={`${event.status}${event.statusCode ? ` · ${event.statusCode}` : ""}`} />
						<Field label="错误码" mono value={event.errorCode} valueClassName={event.errorCode ? "text-danger" : undefined} />
					</Section>
					<Section icon={<GitBranchIcon />} title="用户与路由">
						<Field label="用户" value={event.userName} />
						<Field label={`凭据 · ${identityTypeLabel(event.credentialType)}`} value={event.credentialName} />
						<Field label="账户组" value={event.accountGroupName || (event.codexAccountName ? "直连" : "")} />
						<Field label="Codex 账户" value={event.codexAccountName} />
					</Section>
					<Section icon={<ScrollTextIcon />} title="模型与档位">
						<Field
							label="模型（请求 → 实际）"
							mono
							value={event.requestedModel && event.requestedModel !== event.model ? `${event.requestedModel} → ${event.model}` : event.model}
						/>
						<Field label="推理强度" value={event.reasoningEffort} />
						<Field
							label="速度档位（请求 → 实际）"
							value={
								event.responseServiceTier && event.responseServiceTier !== event.serviceTier
									? `${tierLabel(event.serviceTier)} → ${tierLabel(event.responseServiceTier)}`
									: tierLabel(event.serviceTier)
							}
							valueClassName={downgraded ? "font-semibold text-warning" : undefined}
						/>
					</Section>
					<Section icon={<MessageSquareMoreIcon />} title="会话">
						<Field label="会话 ID" mono value={event.sessionId} />
						<div className="flex min-w-0 flex-col gap-1">
							<span className="text-[11.5px] leading-4 text-faint">会话类型</span>
							{event.sessionKind ? <LogKind className="w-fit" kind={event.sessionKind} /> : <span className="text-[12.5px] text-faint">—</span>}
						</div>
						<Field label="父会话" mono value={event.parentSessionId} />
						{session?.text ? <Field className="col-span-full" label="同会话请求" value={`${session.total} 条 · ${session.text}`} /> : null}
					</Section>
					<Section icon={<Globe2Icon />} title="客户端与网络">
						<Field label="传输" value={transportText(event)} />
						<Field label="客户端 IP" mono value={event.clientIp} />
						<Field label="X-Forwarded-For" mono value={event.xForwardedFor} />
						<Field className="col-span-full" label="User-Agent" mono value={event.userAgent} />
					</Section>
					<div className="grid gap-4 sm:grid-cols-2">
						<Panel icon={<TimerIcon />} title="耗时">
							<StackedBar
								segments={[
									{ key: "ttft", value: event.ttftMs ?? 0, color: VIZ.blue },
									{ key: "gen", value: generation ?? event.latencyMs, color: VIZ.blueSoft },
								]}
							/>
							<Row color={VIZ.blue} label="首字" value={formatSeconds(event.ttftMs, 1)} />
							<Row color={VIZ.blueSoft} label="生成" value={formatSeconds(generation, 1)} />
							<Row bold label="总延迟" value={formatDuration(event.latencyMs)} />
							<Row label="输出速度" value={speed === null ? "—" : `${speed.toFixed(1)} t/s`} />
						</Panel>
						<Panel icon={<CoinsIcon />} title="Token 明细">
							<StackedBar
								segments={[
									{ key: "input", value: uncached, color: TOKEN_PARTS.input.color },
									{ key: "cached", value: event.cachedInputTokens, color: TOKEN_PARTS.cached.color },
									{ key: "output", value: event.outputTokens, color: TOKEN_PARTS.output.color },
								]}
							/>
							<Row color={TOKEN_PARTS.input.color} label="未缓存输入" value={formatCount(uncached)} />
							<Row
								color={TOKEN_PARTS.cached.color}
								label={`缓存读取 ${formatPercent(ratio(event.cachedInputTokens, event.inputTokens))} · 写入 ${formatCount(event.cacheCreationInputTokens)}`}
								value={formatCount(event.cachedInputTokens)}
							/>
							<Row color={TOKEN_PARTS.output.color} label={`输出（推理 ${formatCount(event.reasoningOutputTokens)}）`} value={formatCount(event.outputTokens)} />
							<Row bold label={`总计 · ${event.totalTokens ? formatCost(event.costUsd) : "未计费"}`} value={formatCount(event.totalTokens)} />
						</Panel>
					</div>
				</div>
			</ScrollArea>
		</>
	);
}

function Section({ icon, title, children }: { icon: ReactNode; title: string; children: ReactNode }) {
	return (
		<section className="flex flex-col gap-2">
			<h3 className="flex items-center gap-1.5 text-[12.5px] leading-5 font-bold [&_svg]:size-3.5 [&_svg]:text-muted-foreground">
				{icon}
				{title}
			</h3>
			<div className="grid grid-cols-1 gap-x-4 gap-y-3 border-t pt-2 sm:grid-cols-3">{children}</div>
		</section>
	);
}

function Field({ label, value, mono = false, className, valueClassName }: { label: string; value: string; mono?: boolean; className?: string; valueClassName?: string | undefined }) {
	return (
		<div className={cn("flex min-w-0 flex-col gap-1", className)}>
			<span className="text-[11.5px] leading-4 text-faint">{label}</span>
			<span className={cn("break-all", mono ? "font-mono text-xs leading-4" : "text-[12.5px] leading-5", !value && "text-faint", valueClassName)}>{value || "—"}</span>
		</div>
	);
}

function Panel({ icon, title, children }: { icon: ReactNode; title: string; children: ReactNode }) {
	return (
		<div className="flex flex-col gap-2 rounded-lg bg-surface-subtle p-4">
			<h3 className="flex items-center gap-1.5 text-[12.5px] leading-5 font-bold [&_svg]:size-3.5 [&_svg]:text-muted-foreground">
				{icon}
				{title}
			</h3>
			{children}
		</div>
	);
}

function Row({ label, value, color, bold = false }: { label: string; value: string; color?: string; bold?: boolean }) {
	return (
		<div className="flex items-center gap-2 text-xs leading-4">
			{color ? <span aria-hidden="true" className="size-2 shrink-0 rounded-[2px]" style={{ background: color }} /> : null}
			<span className="min-w-0 flex-1 text-muted-foreground">{label}</span>
			<span className={cn("font-mono tabular-nums", bold ? "font-bold" : "font-medium")}>{value}</span>
		</div>
	);
}
