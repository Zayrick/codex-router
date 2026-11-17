import type { ReactNode } from "react";

import type { SessionKindStats, UsageAnalytics } from "@/admin-api";
import { Legend, LegendItem, Meter, StackedBar } from "@/components/app/bars";
import { LogKind, Tag } from "@/components/app/badges";
import { SectionCard } from "@/components/app/section-card";
import { VIZ, sessionKindColor, sessionKindLabel } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { formatAgo, formatCount, formatDateTime, formatDuration, formatPercent, formatTokens, shortId } from "@/lib/format";

import { MiniStat, NoData, SectionLabel } from "./common";

function kindOrder(kinds: SessionKindStats[]): SessionKindStats[] {
	const order = ["main", "subagent", "fork", "compaction", ""];
	return [...kinds].sort((left, right) => order.indexOf(left.kind) - order.indexOf(right.kind));
}

function KindBreakdown({ analytics }: { analytics: UsageAnalytics }) {
	const kinds = kindOrder(analytics.sessions.kinds);
	const total = kinds.reduce((sum, kind) => sum + kind.requests, 0);
	return (
		<>
			<StackedBar
				gap={2}
				height={10}
				segments={kinds.map((kind) => ({ key: kind.kind || "none", value: kind.requests, color: sessionKindColor(kind.kind), label: sessionKindLabel(kind.kind) }))}
			/>
			<div className="flex flex-col gap-2">
				{kinds.map((kind) => (
					<div className="flex items-center justify-between gap-3 text-[12.5px]" key={kind.kind || "none"}>
						<LegendItem className="text-[12.5px] text-foreground" color={sessionKindColor(kind.kind)} label={sessionKindLabel(kind.kind)} />
						<span className="flex items-baseline gap-3">
							<span className="font-semibold tabular-nums">{formatCount(kind.requests)}</span>
							<span className="w-9 text-right text-[11px] text-faint tabular-nums">{formatPercent(total ? kind.requests / total : 0, 0)}</span>
						</span>
					</div>
				))}
			</div>
		</>
	);
}

/** Figma "card · 会话构成". */
export function SessionMixCard({ analytics }: { analytics: UsageAnalytics }) {
	const sessions = analytics.sessions;
	return (
		<SectionCard description="按会话类型统计请求" title="会话构成">
			<div className="flex gap-6">
				<MiniStat label="会话" value={formatCount(sessions.sessions)} />
				<MiniStat label="请求 / 会话" value={sessions.sessions ? (sessions.requestsWithSession / sessions.sessions).toFixed(1) : "—"} />
				<MiniStat label="最长会话" value={formatCount(sessions.maxRequests)} />
			</div>
			{sessions.kinds.length ? <KindBreakdown analytics={analytics} /> : <NoData />}
		</SectionCard>
	);
}

function ClientList({ analytics, limit = 5 }: { analytics: UsageAnalytics; limit?: number }) {
	const clients = analytics.clients.clients.slice(0, limit);
	const total = analytics.totals.requests;
	if (!clients.length) return <NoData />;
	return (
		<div className="flex flex-col gap-3">
			{clients.map((client, index) => {
				const share = total ? client.requests / total : 0;
				return (
					<div className="flex flex-col gap-1" key={`${client.client}/${client.version}/${client.os}/${index}`}>
						<div className="flex items-baseline justify-between gap-3">
							<span className="truncate font-mono text-[12.5px]">
								{client.client || "未知客户端"} {client.version}
							</span>
							<span className="flex shrink-0 items-baseline gap-3">
								<span className="text-[12.5px] font-semibold tabular-nums">{formatCount(client.requests)}</span>
								<span className="w-9 text-right text-[11px] text-faint tabular-nums">{formatPercent(share, 0)}</span>
							</span>
						</div>
						<span className="text-[11.5px] text-faint">{client.os || "—"}</span>
						<Meter color={VIZ.blue} height={4} ratio={share} />
					</div>
				);
			})}
		</div>
	);
}

/** Figma "card · 客户端". */
export function ClientsCard({ analytics }: { analytics: UsageAnalytics }) {
	const clients = analytics.clients;
	return (
		<SectionCard description="按 User-Agent 解析" title="客户端">
			<div className="flex gap-6">
				<MiniStat label="客户端版本" value={formatCount(clients.distinctClients)} />
				<MiniStat label="客户端 IP" value={formatCount(clients.distinctIps)} />
				<MiniStat label="经代理转发" value={formatPercent(analytics.totals.requests ? clients.forwardedRequests / analytics.totals.requests : null, 0)} />
			</div>
			<ClientList analytics={analytics} />
		</SectionCard>
	);
}

function Split({ label, items }: { label: string; items: { key: string; label: string; value: number; color: string }[] }) {
	return (
		<div className="flex flex-col gap-2">
			<SectionLabel>{label}</SectionLabel>
			<StackedBar gap={2} height={10} segments={items.map((item) => ({ key: item.key, value: item.value, color: item.color, label: item.label }))} />
			<Legend className="justify-between">
				{items.map((item) => (
					<LegendItem color={item.color} key={item.key} label={item.label} value={formatCount(item.value)} />
				))}
			</Legend>
		</div>
	);
}

function transportItems(analytics: UsageAnalytics) {
	const find = (name: string) => analytics.transports.transports.find((row) => row.name === name)?.requests ?? 0;
	return [
		{ key: "websocket", label: "WebSocket", value: find("websocket"), color: VIZ.blue },
		{ key: "http", label: "HTTP", value: find("http"), color: VIZ.blueSoft },
	];
}

function streamItems(analytics: UsageAnalytics) {
	const find = (name: string) => analytics.transports.streaming.find((row) => row.name === name)?.requests ?? 0;
	return [
		{ key: "stream", label: "流式", value: find("stream"), color: VIZ.teal },
		{ key: "non_stream", label: "非流式", value: find("non_stream"), color: VIZ.tealSoft },
	];
}

/** Figma "card · 传输与端点". */
export function TransportEndpointsCard({ analytics }: { analytics: UsageAnalytics }) {
	const endpoints = analytics.transports.endpoints.filter((row) => row.name);
	return (
		<SectionCard title="传输与端点">
			<Split items={transportItems(analytics)} label="传输方式" />
			<Split items={streamItems(analytics)} label="响应方式" />
			<div className="flex flex-col gap-2">
				<SectionLabel>端点</SectionLabel>
				{endpoints.length ? (
					endpoints.slice(0, 6).map((row) => (
						<div className="flex items-baseline justify-between gap-3 text-xs" key={row.name}>
							<span className="truncate font-mono">{row.name}</span>
							<span className="font-semibold tabular-nums">{formatCount(row.requests)}</span>
						</div>
					))
				) : (
					<span className="text-xs text-faint">—</span>
				)}
			</div>
		</SectionCard>
	);
}

/** User page "card · 会话与客户端": sessions, kinds, clients and sources in one card. */
export function SessionsClientsCard({ analytics, now, onOpenSession, onOpenLogs }: { analytics: UsageAnalytics; now: number; onOpenSession?: (sessionId: string) => void; onOpenLogs?: () => void }) {
	const sessions = analytics.sessions;
	const recent = [...sessions.recent].sort((left, right) => right.totalTokens - left.totalTokens).slice(0, 3);
	const total = analytics.totals.requests;
	const transports = transportItems(analytics);
	const streams = streamItems(analytics);
	const chip = (label: string, value: ReactNode) => (
		<Tag className="h-6 rounded-md">
			{label} <span className="font-semibold text-foreground">{value}</span>
		</Tag>
	);
	return (
		<SectionCard title="会话与客户端">
			<div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
				<MiniStat label="会话" value={formatCount(sessions.sessions)} />
				<MiniStat label="请求 / 会话" value={sessions.sessions ? (sessions.requestsWithSession / sessions.sessions).toFixed(1) : "—"} />
				<MiniStat label="最长会话（请求）" value={formatCount(sessions.maxRequests)} />
				<MiniStat label="会话中位时长" value={formatDuration(sessions.medianDurationMs)} />
			</div>
			<div className="flex flex-col gap-2">
				<SectionLabel
					aside={
						onOpenLogs ? (
							<Button className="h-auto p-0 text-xs" onClick={onOpenLogs} type="button" variant="link">
								在调用日志中查看 →
							</Button>
						) : undefined
					}
				>
					最活跃会话
				</SectionLabel>
				{recent.length ? (
					recent.map((session) => (
						<button
							className="flex items-center justify-between gap-3 rounded-md py-1 text-left hover:bg-muted/40 disabled:hover:bg-transparent"
							disabled={!onOpenSession}
							key={session.sessionId}
							onClick={() => onOpenSession?.(session.sessionId)}
							type="button"
						>
							<span className="flex min-w-0 flex-col">
								<span className="flex items-center gap-2 font-mono text-[12.5px]">
									{shortId(session.sessionId)}
									<LogKind kind={session.sessionKind} />
								</span>
								<span className="text-[11px] text-faint">
									{session.childSessions ? `含 ${session.childSessions} 个子会话 · ` : ""}最近 {formatDateTime(session.lastAt)}（{formatAgo(session.lastAt, now)}）
								</span>
							</span>
							<span className="flex shrink-0 items-baseline gap-3 text-xs tabular-nums">
								<span className="text-muted-foreground">{formatCount(session.requests)} 次</span>
								<span className="w-14 text-right font-semibold">{formatTokens(session.totalTokens)}</span>
							</span>
						</button>
					))
				) : (
					<span className="text-xs text-faint">暂无会话</span>
				)}
			</div>
			<div className="flex flex-col gap-2">
				<SectionLabel>会话类型</SectionLabel>
				{sessions.kinds.length ? <KindBreakdown analytics={analytics} /> : <span className="text-xs text-faint">—</span>}
			</div>
			<div className="flex flex-col gap-2">
				<SectionLabel aside="user_agent">客户端版本</SectionLabel>
				<ClientList analytics={analytics} limit={3} />
			</div>
			<div className="flex flex-col gap-2">
				<SectionLabel>来源 IP 与传输</SectionLabel>
				{analytics.clients.ips.slice(0, 3).map((ip) => (
					<div className="flex items-baseline justify-between gap-3" key={ip.ip}>
						<span className="flex min-w-0 flex-col">
							<span className="truncate font-mono text-[12.5px]">{ip.ip}</span>
							<span className="text-[11px] text-faint">最近 {formatDateTime(ip.lastAt)}</span>
						</span>
						<span className="flex shrink-0 items-baseline gap-3 text-xs tabular-nums">
							<span className="font-semibold">{formatCount(ip.requests)}</span>
							<span className="w-9 text-right text-faint">{formatPercent(total ? ip.requests / total : 0, 0)}</span>
						</span>
					</div>
				))}
				<div className="flex flex-wrap gap-1.5">
					{transports.map((item) => (item.value ? <span key={item.key}>{chip(item.label, formatPercent(total ? item.value / total : 0, 0))}</span> : null))}
					{streams[0] && streams[0].value ? chip("流式", formatPercent(total ? streams[0].value / total : 0, 0)) : null}
					{chip("X-Forwarded-For", analytics.clients.forwardedRequests ? "有" : "无")}
				</div>
			</div>
		</SectionCard>
	);
}
