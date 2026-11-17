import { CheckCircle2Icon, CircleAlertIcon, InfoIcon, KeyRoundIcon, Layers3Icon, Link2Icon, RefreshCwIcon, TriangleAlertIcon, UserRoundIcon } from "lucide-react";
import type { ReactNode } from "react";

import type { AccountGroup, AccountQuota, CodexAccount, SubscriptionInfo, UpstreamAccountStats, WindowForecast } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { DeleteConfirmationDialog } from "@/admin/dialogs";
import { strategyLabel } from "@/admin/routing-model";
import { Meter } from "@/components/app/bars";
import { Tag } from "@/components/app/badges";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { formatCost, formatDateTime, formatDay, formatPercent, formatSeconds, formatTokens, formatClock, formatCount } from "@/lib/format";
import { cn } from "@/lib/utils";

import {
	TONE_COLOR,
	accountTitle,
	accountTone,
	cacheRate,
	forecastCopy,
	formatBurn,
	initial,
	maskAccountId,
	planText,
	successRate,
	visibleWindows,
	windowLabel,
	windowTone,
} from "./model";
import { QuotaTimeline } from "./QuotaTimeline";
import { UsagePanel } from "./UsagePanel";

export interface RouteIdentity {
	id: string;
	name: string;
}

export interface RoutingSource {
	/** Null for identities routed directly to the account. */
	group: AccountGroup | null;
	identities: RouteIdentity[];
}

/** Figma "account · …": header, risk insight, quota windows + timeline and 7-day usage. */
export function AccountCard({
	account,
	quota,
	stats,
	totalTokens,
	sources,
	share,
	subscription,
	refreshing,
	onRename,
	onRefresh,
}: {
	account: CodexAccount;
	quota: AccountQuota | undefined;
	stats: UpstreamAccountStats | undefined;
	/** Tokens of all accounts in the same period, for the share. */
	totalTokens: number;
	sources: RoutingSource[];
	/** Live routing weight in its weighted group, when any. */
	share: { group: string; value: number } | null;
	subscription: SubscriptionInfo | null;
	refreshing: boolean;
	onRename: () => void;
	onRefresh: () => void;
}) {
	const { now, busy, updateCodexAccount, deleteCodexAccount } = useAdmin();
	const title = accountTitle(account);
	const plan = quota?.planType ?? account.subscription?.planType ?? null;
	const tokenExpired = account.oauth ? account.oauth.expiresAt > 0 && account.oauth.expiresAt <= now : true;
	const tone = accountTone(quota, now);
	const windows = visibleWindows(quota?.windows ?? []);
	const exhausting = windows.filter((window) => window.exhaustsBeforeReset || window.limitReached);
	const isBusy = busy.has(`codex:${account.id}`);
	const meta = [
		account.oauth?.email && account.name !== account.oauth.email ? account.name : null,
		maskAccountId(account.oauth?.accountId),
		account.oauth?.expiresAt ? `Token 至 ${formatDay(account.oauth.expiresAt)}` : null,
		quota?.sampledAt ? `同步于 ${formatSyncTime(quota.sampledAt, now)}` : null,
	].filter(Boolean);

	return (
		<Card className="gap-0 overflow-hidden py-0">
			<div className={cn("flex flex-wrap items-center gap-4 border-b px-6 py-4", !account.enabled && "opacity-70")}>
				<span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary-soft text-[15px] font-semibold">{initial(title)}</span>
				<div className="flex min-w-0 flex-1 flex-col gap-1">
					<div className="flex flex-wrap items-center gap-2">
						<span className="truncate text-[15px] leading-5 font-semibold tracking-[-0.01em]">{title}</span>
						{plan ? <Tag>{planText(plan)}</Tag> : null}
						{tokenExpired ? (
							<Tag tone="danger">
								<Dot className="bg-danger" />
								Token 已过期
							</Tag>
						) : (
							<Tag tone="success">
								<Dot className="bg-success" />
								凭据有效
							</Tag>
						)}
						{!account.enabled ? <Tag>已停用</Tag> : null}
						{account.enabled && quota?.quarantined ? <Tag tone="danger">已暂停调度</Tag> : null}
						{account.enabled && tone === "danger" ? <Tag tone="danger">额度即将耗尽</Tag> : null}
						{account.enabled && tone === "warning" ? <Tag tone="warning">额度偏紧</Tag> : null}
					</div>
					<div className="flex items-center gap-1.5 text-xs leading-4 text-muted-foreground">
						<span className="truncate">{meta.join(" · ")}</span>
						<AccountDetails account={account} quota={quota} subscription={subscription} />
					</div>
				</div>
				<div className="flex flex-wrap items-center gap-2">
					<label className="flex items-center gap-2 text-[12.5px] text-muted-foreground">
						{account.enabled ? "启用" : "已停用"}
						<Switch
							aria-label={account.enabled ? `停用 ${title}` : `启用 ${title}`}
							checked={account.enabled}
							disabled={isBusy}
							onCheckedChange={(enabled) => void updateCodexAccount(account, { name: account.name, enabled })}
						/>
					</label>
					<Button aria-label="刷新额度" disabled={!account.enabled || refreshing} onClick={onRefresh} size="icon-sm" title="刷新额度" type="button" variant="outline">
						{refreshing ? <Spinner /> : <RefreshCwIcon />}
					</Button>
					<Button disabled={isBusy} onClick={onRename} size="sm" type="button" variant="outline">
						重命名
					</Button>
					<DeleteConfirmationDialog
						description="相关路由也会被移除。此操作无法撤销。"
						onConfirm={() => void deleteCodexAccount(account)}
						title={`删除 Codex 账户“${title}”？`}
						trigger={
							<Button disabled={isBusy} size="sm" type="button" variant="destructive">
								删除
							</Button>
						}
					/>
				</div>
			</div>

			{account.enabled && exhausting.length ? (
				<div className="flex items-center gap-2 border-b bg-warning-soft px-6 py-2 text-xs leading-4 text-warning">
					<TriangleAlertIcon className="size-3.5 shrink-0" />
					<span>{insightText(exhausting, windows.length, share)}</span>
				</div>
			) : null}

			<div className={cn("flex flex-col gap-6 p-6 lg:flex-row", !account.enabled && "opacity-70")}>
				{account.enabled ? (
					<div className="flex min-w-0 flex-1 flex-col justify-between gap-4">
						{windows.length ? (
							<QuotaWindowTable now={now} windows={windows} />
						) : (
							<p className="py-6 text-center text-xs text-muted-foreground">{quota?.sampledAt ? "暂无额度窗口" : "额度尚未同步，点击刷新获取。"}</p>
						)}
						{windows.length ? (
							<div className="flex flex-col gap-2">
								<QuotaTimeline accounts={[{ id: account.id, title, plan, windows }]} daysBefore={3} days={11} now={now} showAccountHeaders={false} title="额度时间轴" />
							</div>
						) : null}
					</div>
				) : (
					<DisabledState account={account} tokenExpired={tokenExpired} />
				)}
				<UsagePanel
					aside={
						!account.enabled && !stats
							? "停用期间无请求"
							: stats && totalTokens > 0
								? `占全部 ${formatPercent(stats.totalTokens / totalTokens)}`
								: null
					}
					className="lg:w-[300px] lg:shrink-0"
					stats={[
						{ label: "成本", value: formatCost(stats?.costUsd ?? 0) },
						{ label: "请求", value: formatCount(stats?.requests ?? 0) },
						{ label: "成功率", value: stats ? formatPercent(successRate(stats)) : "—" },
						{ label: "429", value: formatCount(stats?.rateLimited ?? 0), tone: stats?.rateLimited ? "danger" : "default" },
						{ label: "缓存率", value: stats ? formatPercent(cacheRate(stats)) : "—" },
						{ label: "首字 P50", value: formatSeconds(stats?.ttft.p50 ?? null) },
					]}
					tokens={formatTokens(stats?.totalTokens ?? 0)}
					trend={stats?.trend ?? null}
				>
					<RoutingSources sources={sources} />
				</UsagePanel>
			</div>
		</Card>
	);
}

function Dot({ className }: { className: string }) {
	return <span aria-hidden="true" className={cn("size-1.5 rounded-full", className)} />;
}

function insightText(exhausting: WindowForecast[], total: number, share: { group: string; value: number } | null): string {
	const head =
		exhausting.length === total && total > 1 ? "所有窗口都将在重置前耗尽" : `${exhausting.map((window) => windowLabel(window)).join("、")}窗口将在重置前耗尽`;
	const weight = share ? `，${share.group}当前分配权重 ${formatPercent(share.value, 0)}` : "";
	return `${head}${weight}；如需保留额度，可临时停用。`;
}

function formatSyncTime(at: number, now: number): string {
	return now - at < 20 * 3_600_000 ? formatClock(at, false) : formatDateTime(at);
}

const WINDOW_COLUMNS = "grid grid-cols-[64px_152px_120px_92px_108px_72px_minmax(150px,1fr)] items-center";

/** Figma "quota-windows" table. Scrolls horizontally on narrow screens. */
function QuotaWindowTable({ windows, now }: { windows: WindowForecast[]; now: number }) {
	return (
		<div className="overflow-x-auto">
			<div className="min-w-[700px]">
				<div className={cn(WINDOW_COLUMNS, "border-b pb-2 text-[11.5px] leading-4 font-medium text-faint")}>
					<span>窗口</span>
					<span>已用</span>
					<span>重置</span>
					<span>本周期用量</span>
					<span>每 1% 额度</span>
					<span>消耗速度</span>
					<span>预测</span>
				</div>
				{windows.map((window) => {
					const tone = windowTone(window, now);
					const forecast = forecastCopy(window, now);
					return (
						<div className={cn(WINDOW_COLUMNS, "border-b py-4 last:border-b-0")} key={window.id}>
							<span className="text-[13px] leading-5 font-medium">{windowLabel(window)}</span>
							<span className="flex items-center gap-2">
								<Meter className="w-24" color={TONE_COLOR[tone]} label={`${windowLabel(window)}已用`} ratio={(window.usedPercent ?? 0) / 100} />
								<span className="text-[13px] leading-5 font-semibold tabular-nums">{window.usedPercent === null ? "—" : `${Math.round(window.usedPercent)}%`}</span>
							</span>
							<Cell sub={window.resetAt ? `${formatUntil2(window.resetAt, now)}` : null}>{window.resetAt ? formatDateTime(window.resetAt) : "—"}</Cell>
							<Cell sub={formatCost(window.costUsd)}>{formatTokens(window.totalTokens)}</Cell>
							<Cell sub={window.costPerPercent !== null ? `${formatCost(window.costPerPercent)} / 1%` : null}>
								<span className="font-semibold">{window.tokensPerPercent !== null ? `${formatTokens(window.tokensPerPercent)} Token` : "—"}</span>
							</Cell>
							<span className="text-[13px] leading-5 tabular-nums">{formatBurn(window.burnPercentPerHour)}</span>
							<ForecastCell forecast={forecast} />
						</div>
					);
				})}
			</div>
		</div>
	);
}

function formatUntil2(at: number, now: number): string {
	const diff = at - now;
	if (diff <= 0) return "已重置";
	const hours = Math.floor(diff / 3_600_000);
	const minutes = Math.round((diff % 3_600_000) / 60_000);
	if (hours < 24) return `${hours}h ${minutes}m 后`;
	return `${Math.floor(hours / 24)} 天 ${hours % 24} 小时后`;
}

function Cell({ children, sub }: { children: ReactNode; sub?: string | null }) {
	return (
		<span className="flex min-w-0 flex-col gap-0.5">
			<span className="text-[13px] leading-5 tabular-nums">{children}</span>
			{sub ? <span className="text-[11.5px] leading-4 text-faint tabular-nums">{sub}</span> : null}
		</span>
	);
}

export function ForecastCell({ forecast, compact = false }: { forecast: ReturnType<typeof forecastCopy>; compact?: boolean }) {
	const color = forecast.tone === "danger" ? "text-danger" : forecast.tone === "warning" ? "text-warning" : "text-success";
	return (
		<span className="flex min-w-0 flex-col gap-0.5">
			<span className={cn("flex items-center gap-1 text-[12.5px] leading-5 font-semibold", color, compact && "text-[11.5px] leading-4 font-medium")}>
				{forecast.exhausts ? <TriangleAlertIcon className="size-3.5 shrink-0" /> : <CheckCircle2Icon className="size-3.5 shrink-0" />}
				{forecast.title}
			</span>
			{forecast.detail && !compact ? <span className="text-[11.5px] leading-4 text-faint">{forecast.detail}</span> : null}
		</span>
	);
}

function RoutingSources({ sources }: { sources: RoutingSource[] }) {
	return (
		<div className="flex flex-col gap-2">
			<span className="text-[11.5px] leading-4 font-medium text-faint">路由来源</span>
			{sources.length === 0 ? <span className="text-xs text-muted-foreground">未被任何路由使用</span> : null}
			{sources.map((source) => (
				<div className="flex flex-col gap-2" key={source.group?.id ?? "direct"}>
					<span className="flex items-center gap-1.5 text-[12.5px] leading-5">
						{source.group ? <Layers3Icon className="size-3.5" /> : <Link2Icon className="size-3.5" />}
						<span className="font-bold">{source.group ? source.group.name : "直连"}</span>
						<span className="text-[11.5px] text-faint">
							{source.group ? `${strategyLabel(source.group.strategy)}${source.identities.length ? "" : " · 未分配"}` : "用户直接路由到此账户"}
						</span>
					</span>
					{source.identities.length ? (
						<div className="flex flex-wrap gap-1.5">
							{source.identities.map((identity) => (
								<IdentityChip identity={identity} key={identity.id} />
							))}
						</div>
					) : null}
				</div>
			))}
		</div>
	);
}

export function IdentityChip({ identity, aside }: { identity: RouteIdentity; aside?: ReactNode }) {
	return (
		<span className="inline-flex max-w-full items-center gap-1 rounded-full border bg-card px-2 py-[3px] text-[11.5px] leading-4">
			<UserRoundIcon className="size-3 shrink-0" />
			<span className="truncate">{identity.name}</span>
			{aside}
		</span>
	);
}

function DisabledState({ account, tokenExpired }: { account: CodexAccount; tokenExpired: boolean }) {
	const { startDeviceLogin, updateCodexAccount, deviceLoginActive } = useAdmin();
	return (
		<div className="flex min-w-0 flex-1 gap-4 rounded-[10px] border border-dashed border-border-strong p-6">
			<CircleAlertIcon className="size-5 shrink-0 text-muted-foreground" />
			<div className="flex flex-col gap-2">
				<p className="text-[13px] leading-5 font-bold">已停用，不参与调度与额度同步</p>
				<p className="text-xs leading-4 text-muted-foreground">
					{tokenExpired && account.oauth?.expiresAt
						? `Token 已于 ${formatDateTime(account.oauth.expiresAt)} 过期，重新登录后即可启用。`
						: tokenExpired
							? "尚无有效凭据，重新登录后即可启用。"
							: "启用后将重新参与路由并同步额度。"}
				</p>
				<div>
					{tokenExpired ? (
						<Button disabled={deviceLoginActive} onClick={startDeviceLogin} size="sm" type="button" variant="outline">
							<KeyRoundIcon data-icon="inline-start" />
							重新登录
						</Button>
					) : (
						<Button onClick={() => void updateCodexAccount(account, { name: account.name, enabled: true })} size="sm" type="button" variant="outline">
							启用
						</Button>
					)}
				</div>
			</div>
		</div>
	);
}

/** Figma "tooltip · 账户详情". */
function AccountDetails({ account, quota, subscription }: { account: CodexAccount; quota: AccountQuota | undefined; subscription: SubscriptionInfo | null }) {
	const { now } = useAdmin();
	const metadata = subscription ?? account.subscription;
	const credits = subscription?.rateLimitResetCredits.availableCount ?? null;
	const rows: { label: string; value: string; danger?: boolean }[] = [
		{ label: "Account ID", value: maskAccountId(account.oauth?.accountId) },
		{ label: "开始时间", value: formatLong(metadata?.subscriptionActiveStart) },
		{ label: "到期时间", value: formatLong(metadata?.subscriptionActiveUntil), danger: isPast(metadata?.subscriptionActiveUntil, now) },
		{ label: "Token 到期时间", value: formatLong(account.oauth?.expiresAt), danger: isPast(account.oauth?.expiresAt, now) },
		{ label: "重置积分", value: credits === null ? "—" : String(Math.max(0, credits)) },
		{ label: "用量更新时间", value: formatLong(subscription?.fetchedAt ?? quota?.sampledAt) },
	];
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button aria-label="查看账户详情" size="icon-xs" type="button" variant="ghost">
					<InfoIcon />
				</Button>
			</TooltipTrigger>
			<TooltipContent align="start" className="flex w-[304px] max-w-none flex-col items-stretch gap-2 px-3 py-2.5" side="bottom" sideOffset={6}>
				<strong className="font-bold">账户详情</strong>
				{rows.map((row) => (
					<span className="grid grid-cols-[96px_minmax(0,1fr)] gap-3 leading-4" key={row.label}>
						<span className="opacity-70">{row.label}</span>
						<b className={cn("text-right font-semibold", row.danger && "text-[color-mix(in_srgb,var(--danger)_55%,white)]")}>{row.value}</b>
					</span>
				))}
			</TooltipContent>
		</Tooltip>
	);
}

function isPast(value: number | null | undefined, now: number): boolean {
	return typeof value === "number" && value > 0 && value <= now;
}

function formatLong(value: number | null | undefined): string {
	if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return "—";
	const date = new Date(value);
	return `${date.getFullYear()}年${date.getMonth() + 1}月${date.getDate()}日 ${formatClock(value, false)}`;
}

