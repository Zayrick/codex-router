import { CopyIcon, EyeIcon, EyeOffIcon, ScrollTextIcon, ServerIcon } from "lucide-react";
import { useState, type ReactNode } from "react";

import type { LastSeen, UsageAnalytics, UsageEventStatus, User, UserCredential } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { strategyLabel, formatTtl } from "@/admin/routing-model";
import { useAnalytics } from "@/admin/hooks";
import { CodeChip } from "@/components/app/badges";
import { Legend, LegendItem, Meter, StackedBar } from "@/components/app/bars";
import { SESSION_KIND_META, STATUS_META, seriesColor, sessionKindColor, sessionKindLabel } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { formatCount, formatDateTime, formatPercent, formatSeconds, formatTokens, maskSecret, ratio } from "@/lib/format";
import { cn } from "@/lib/utils";

import { useLastSeen } from "./use-last-seen";
import { CREDENTIAL_LABEL, clientSummary, type RouteInfo } from "./model";
import { copyToClipboard } from "@/lib/clipboard";

const RANGE_TITLE: Record<string, string> = { "24h": "近 24 小时", "7d": "近 7 天", "30d": "近 30 天" };

/** Expanded "近 7 天明细" panel of a user row. */
export function IdentityDetail({ user, range, route }: { user: User; range: "24h" | "7d" | "30d"; route: RouteInfo }) {
	const { navigate } = useAdmin();
	const { data, error, loading } = useAnalytics({ range, downstream: { identityType: "user", identityId: user.id } });
	const lastSeen = useLastSeen();

	return (
		<div className="flex flex-col gap-4 border-b bg-surface-subtle py-4 pr-6 pl-6 sm:pl-12">
			<div className="flex flex-wrap items-center justify-between gap-3">
				<div className="flex flex-wrap items-center gap-2">
					<span className="text-[13px] leading-5 font-semibold">
						{user.name} · {RANGE_TITLE[range]}明细
					</span>
					{data ? <HeadChips analytics={data} /> : null}
				</div>
				<Button
					onClick={() => navigate("logs", { downstreamType: "user", downstreamId: user.id })}
					size="sm"
					type="button"
					variant="outline"
				>
					<ScrollTextIcon data-icon="inline-start" />
					在调用日志中查看
				</Button>
			</div>
			<CredentialsPanel lastSeen={lastSeen.data} user={user} />
			{error ? <p className="text-xs text-danger">{error}</p> : null}
			{!data && loading ? (
				<div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
					{[0, 1, 2, 3].map((index) => (
						<Skeleton className="h-56 rounded-lg" key={index} />
					))}
				</div>
			) : null}
			{data && data.totals.requests === 0 ? (
				<Empty className="rounded-lg border bg-card py-8">
					<EmptyHeader>
						<EmptyTitle className="text-sm">{RANGE_TITLE[range]}没有请求</EmptyTitle>
						<EmptyDescription className="text-xs">切换统计窗口或稍后再看。</EmptyDescription>
					</EmptyHeader>
				</Empty>
			) : null}
			{data && data.totals.requests > 0 ? (
				<>
					<div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
						<ModelsPanel analytics={data} />
						<SessionsPanel analytics={data} />
						<ClientsPanel analytics={data} />
						<FailuresPanel analytics={data} />
					</div>
					<UpstreamLine analytics={data} route={route} />
				</>
			) : null}
		</div>
	);
}

function CredentialsPanel({ user, lastSeen }: { user: User; lastSeen: LastSeen | null }) {
	return (
		<Card className="gap-0 rounded-lg py-1 shadow-none ring-border">
			{user.credentials.map((credential, index) => (
				<div key={credential.id}>
					{index > 0 ? <Separator /> : null}
					<CredentialLine credential={credential} seen={lastSeen?.credentials.find((item) => item.credentialId === credential.id) ?? null} />
				</div>
			))}
		</Card>
	);
}

function CredentialLine({ credential, seen }: { credential: UserCredential; seen: LastSeen["credentials"][number] | null }) {
	const { notify } = useAdmin();
	const [revealed, setRevealed] = useState(false);
	const secret = credential.type === "api_key";
	async function copy() {
		try {
			await copyToClipboard(credential.value);
			notify(`${CREDENTIAL_LABEL[credential.type]}已复制。`);
		} catch {
			notify("复制失败，请手动复制。", "error");
		}
	}
	return (
		<div className={cn("flex flex-wrap items-center gap-x-4 gap-y-1 px-4 py-2", !credential.enabled && "opacity-55")}>
			<span className="w-16 shrink-0 text-[11.5px] leading-4 font-medium text-muted-foreground">{CREDENTIAL_LABEL[credential.type]}</span>
			<span className="flex min-w-0 flex-1 items-center gap-0.5">
				<span className="truncate font-mono text-[11.5px] leading-4">{secret && !revealed ? maskSecret(credential.value) : credential.value}</span>
				{secret ? (
					<Button aria-label={revealed ? "隐藏" : "显示"} className="text-muted-foreground" onClick={() => setRevealed((value) => !value)} size="icon-xs" type="button" variant="ghost">
						{revealed ? <EyeOffIcon /> : <EyeIcon />}
					</Button>
				) : null}
				<Button aria-label="复制" className="text-muted-foreground" onClick={() => void copy()} size="icon-xs" type="button" variant="ghost">
					<CopyIcon />
				</Button>
				{credential.name ? <span className="truncate text-[11.5px] leading-4 text-muted-foreground">{credential.name}</span> : null}
			</span>
			<span className="text-[11px] leading-4 text-faint">
				{credential.enabled ? "" : "已停用 · "}
				{seen ? [`最近 ${formatDateTime(seen.lastAt)}`, clientSummary(seen.userAgent)].filter(Boolean).join(" · ") : "从未使用"}
			</span>
		</div>
	);
}

function Chip({ label, value }: { label: ReactNode; value: ReactNode }) {
	return (
		<span className="inline-flex items-center gap-1.5 rounded-full bg-card px-2 py-0.5 text-[11.5px] leading-4">
			<span className="text-muted-foreground">{label}</span>
			<span className="font-semibold tabular-nums">{value}</span>
		</span>
	);
}

function HeadChips({ analytics }: { analytics: UsageAnalytics }) {
	const { totals, transports } = analytics;
	const websocket = transports.transports.find((item) => item.name === "websocket")?.requests ?? 0;
	return (
		<>
			<Chip label="缓存命中" value={formatPercent(ratio(totals.cachedInputTokens, totals.inputTokens))} />
			<Chip label="首字 P50" value={formatSeconds(totals.ttft.p50)} />
			<Chip label="延迟 P50" value={formatSeconds(totals.latency.p50, 1)} />
			<Chip label="WebSocket" value={formatPercent(ratio(websocket, totals.requests), 0)} />
		</>
	);
}

function Panel({ title, aside, children, className }: { title: ReactNode; aside?: ReactNode; children: ReactNode; className?: string }) {
	return (
		<Card className={cn("min-w-0 gap-3 rounded-lg px-4 py-4 shadow-none ring-border", className)}>
			<div className="flex items-center justify-between gap-2">
				<span className="text-[12.5px] leading-5 font-bold">{title}</span>
				{aside ? <span className="text-[11px] leading-4 text-faint">{aside}</span> : null}
			</div>
			{children}
		</Card>
	);
}

function KeyValue({ label, value, share, sub, mono = false, color }: { label: ReactNode; value: ReactNode; share?: string; sub?: ReactNode; mono?: boolean; color?: string }) {
	return (
		<div className="flex min-w-0 flex-col gap-0.5">
			<div className="flex items-center justify-between gap-2">
				<span className="flex min-w-0 items-center gap-1.5">
					{color ? <span aria-hidden="true" className="size-2 shrink-0 rounded-[2px]" style={{ background: color }} /> : null}
					<span className={cn("truncate text-[11.5px] leading-4", mono ? "font-mono text-foreground" : "text-muted-foreground")}>{label}</span>
				</span>
				<span className="flex shrink-0 items-baseline gap-1.5 tabular-nums">
					<span className="text-[11.5px] leading-4 font-semibold">{value}</span>
					{share ? <span className="text-[10.5px] leading-4 text-faint">{share}</span> : null}
				</span>
			</div>
			{sub ? <span className="truncate text-[11px] leading-4 text-faint">{sub}</span> : null}
		</div>
	);
}

function ModelsPanel({ analytics }: { analytics: UsageAnalytics }) {
	const total = analytics.totals.totalTokens;
	const models = analytics.models.slice(0, 5);
	const unpriced = analytics.models.filter((model) => !model.priced && model.totalTokens > 0);
	return (
		<Panel aside={`${analytics.models.length} 个模型 · Token`} title="模型">
			{models.map((model, index) => (
				<div className="flex flex-col gap-1" key={model.model}>
					<KeyValue color={seriesColor(index)} label={model.model} mono share={formatPercent(ratio(model.totalTokens, total))} value={formatTokens(model.totalTokens)} />
					<Meter color={seriesColor(index)} height={4} ratio={ratio(model.totalTokens, total)} />
				</div>
			))}
			{unpriced.length ? (
				<p className="text-[11px] leading-4 text-warning">{unpriced.map((model) => model.model).join("、")} 未定价，不计入成本</p>
			) : null}
		</Panel>
	);
}

function SessionsPanel({ analytics }: { analytics: UsageAnalytics }) {
	const { sessions, totals } = analytics;
	const kinds = sessions.kinds;
	const average = sessions.sessions > 0 ? sessions.requestsWithSession / sessions.sessions : null;
	return (
		<Panel title="会话">
			<div className="flex items-baseline gap-1.5">
				<span className="text-xl leading-7 font-bold tracking-[-0.03em] tabular-nums">{formatCount(sessions.sessions)}</span>
				<span className="text-xs text-muted-foreground">个会话</span>
			</div>
			<span className="text-[11px] leading-4 text-faint">
				平均 {average === null ? "—" : average.toFixed(1)} 请求 / 会话 · 最长 {formatCount(sessions.maxRequests)} 请求
			</span>
			<StackedBar
				height={6}
				segments={kinds.map((kind) => ({ key: kind.kind || "none", value: kind.requests, color: sessionKindColor(kind.kind), label: sessionKindLabel(kind.kind) }))}
			/>
			{(["main", "subagent", "fork", "compaction", ""] as const).map((kind) => {
				const row = kinds.find((item) => item.kind === kind);
				if (!row) return null;
				return (
					<KeyValue
						color={sessionKindColor(kind)}
						key={kind || "none"}
						label={kind ? SESSION_KIND_META[kind].label : "未标识"}
						share={formatPercent(ratio(row.requests, totals.requests), 0)}
						value={formatCount(row.requests)}
					/>
				);
			})}
		</Panel>
	);
}

function ClientsPanel({ analytics }: { analytics: UsageAnalytics }) {
	const { clients, totals } = analytics;
	return (
		<Panel title="客户端与 IP">
			{clients.clients.slice(0, 3).map((client) => (
				<KeyValue
					key={`${client.client}/${client.version}/${client.os}`}
					label={[client.client || "未知客户端", client.version].filter(Boolean).join(" ")}
					mono
					share={formatPercent(ratio(client.requests, totals.requests))}
					sub={client.os || undefined}
					value={formatCount(client.requests)}
				/>
			))}
			{clients.ips.length ? <Separator /> : null}
			{clients.ips.slice(0, 3).map((ip) => (
				<KeyValue
					key={ip.ip}
					label={ip.ip}
					mono
					sub={ip.forwardedFor ? `X-Forwarded-For ${ip.forwardedFor}` : `最近 ${formatDateTime(ip.lastAt)}`}
					value={formatCount(ip.requests)}
				/>
			))}
		</Panel>
	);
}

const FAILURE_STATUSES: { status: UsageEventStatus; label: string }[] = [
	{ status: "failed", label: "失败" },
	{ status: "cancelled", label: "取消" },
	{ status: "interrupted", label: "中断" },
	{ status: "incomplete", label: "未完成" },
];

function FailuresPanel({ analytics }: { analytics: UsageAnalytics }) {
	const { totals } = analytics;
	const counts = { failed: totals.failed, cancelled: totals.cancelled, interrupted: totals.interrupted, incomplete: totals.incomplete, completed: 0 };
	const unsuccessful = totals.failed + totals.cancelled + totals.interrupted + totals.incomplete;
	return (
		<Panel aside={`${formatCount(unsuccessful)} 次 · ${formatPercent(ratio(unsuccessful, totals.requests))}`} title="最近失败">
			<StackedBar
				height={6}
				segments={FAILURE_STATUSES.map((item) => ({ key: item.status, value: counts[item.status], color: STATUS_META[item.status].color, label: item.label }))}
			/>
			<Legend className="gap-x-3">
				{FAILURE_STATUSES.filter((item) => counts[item.status] > 0).map((item) => (
					<LegendItem className="text-[11px]" color={STATUS_META[item.status].color} key={item.status} label={item.label} value={counts[item.status]} />
				))}
			</Legend>
			{analytics.recentFailures.length === 0 ? <span className="text-xs text-faint">没有未成功的请求</span> : null}
			{analytics.recentFailures.slice(0, 3).map((event) => (
				<div className="flex flex-col gap-0.5 border-t pt-2" key={event.id}>
					<div className="flex min-w-0 items-center gap-1.5">
						<CodeChip tone={event.status === "failed" ? "danger" : event.status === "cancelled" ? "neutral" : "warning"}>{event.status}</CodeChip>
						<span className="truncate font-mono text-[11px] leading-4">
							{[event.statusCode, event.errorCode].filter((value) => value !== null && value !== "").join(" ") || STATUS_META[event.status].label}
						</span>
					</div>
					<span className="truncate text-[11px] leading-4 text-faint">
						{[formatDateTime(event.recordedAt), event.model, event.codexAccountName].filter(Boolean).join(" · ")}
					</span>
				</div>
			))}
		</Panel>
	);
}

function UpstreamLine({ analytics, route }: { analytics: UsageAnalytics; route: RouteInfo }) {
	const total = analytics.upstreamAccounts.reduce((sum, account) => sum + account.requests, 0);
	const group = route.group;
	return (
		<div className="flex flex-wrap items-center gap-2">
			<ServerIcon aria-hidden="true" className="size-3.5 text-muted-foreground" />
			<span className="text-[11.5px] leading-4 font-medium text-muted-foreground">上游承载</span>
			{analytics.upstreamAccounts.slice(0, 5).map((account) => (
				<Chip key={account.codexAccountId} label={account.codexAccountName} value={formatPercent(ratio(account.requests, total), 0)} />
			))}
			{group ? (
				<span className="text-[11px] leading-4 text-faint">
					{[group.name, strategyLabel(group.strategy), group.strategy !== "fallback" && group.sessionAffinity ? `会话保持 ${formatTtl(group.sessionAffinityTtl)}` : null]
						.filter(Boolean)
						.join(" · ")}
				</span>
			) : route.type === "account" ? (
				<span className="text-[11px] leading-4 text-faint">直连 {route.label}</span>
			) : null}
		</div>
	);
}
