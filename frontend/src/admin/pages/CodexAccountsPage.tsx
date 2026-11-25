import { useMemo, useState } from "react";
import { PlusIcon, RefreshCwIcon, TriangleAlertIcon, UploadIcon, UsersIcon } from "lucide-react";

import type { AccountQuota, CodexAccount, SubscriptionInfo } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { AccountEditDialog } from "@/admin/dialogs";
import { useAnalytics } from "@/admin/hooks";
import { AccountCard, type RouteIdentity, type RoutingSource } from "@/admin/quota/AccountCard";
import {
	accountTitle,
	accountTone,
	forecastCopy,
	planText,
	riskScore,
	shortAccountName,
	visibleWindows,
	windowLabel,
	windowTone,
} from "@/admin/quota/model";
import { Legend, LegendItem, StackedBar } from "@/components/app/bars";
import { Segmented } from "@/components/app/segmented";
import { StatCard } from "@/components/app/stat-card";
import { VIZ, seriesColor } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { formatClock, formatCost, formatCount, formatDateTime, formatPercent, formatSpan, formatTokens } from "@/lib/format";
import { cn } from "@/lib/utils";

type Filter = "all" | "enabled" | "attention" | "disabled";
type Sort = "risk" | "usage" | "name";

const PLAN_COLORS = [VIZ.blue, VIZ.teal, VIZ.muted, VIZ.violet, VIZ.amber];

export default function CodexAccountsPage() {
	const { state, quota, now, api, refreshQuota, quotaRefreshing, reportError, startDeviceLogin, deviceLoginActive } = useAdmin();
	const analytics = useAnalytics({ range: "7d" });
	const [filter, setFilter] = useState<Filter>("all");
	const [sort, setSort] = useState<Sort>("risk");
	const [renaming, setRenaming] = useState<CodexAccount | null>(null);
	const [refreshing, setRefreshing] = useState<ReadonlySet<string>>(() => new Set());
	const [subscriptions, setSubscriptions] = useState<Record<string, SubscriptionInfo>>({});
	const { updateCodexAccount, busy } = useAdmin();

	const quotaById = useMemo(() => new Map((quota?.accounts ?? []).map((entry) => [entry.accountId, entry])), [quota]);
	const statsById = useMemo(() => new Map((analytics.data?.upstreamAccounts ?? []).map((entry) => [entry.codexAccountId, entry])), [analytics.data]);
	const totalTokens = useMemo(() => (analytics.data?.upstreamAccounts ?? []).reduce((sum, entry) => sum + entry.totalTokens, 0), [analytics.data]);

	const attention = (account: CodexAccount) => {
		const tone = accountTone(quotaById.get(account.id), now);
		return account.enabled && (tone === "danger" || tone === "warning");
	};
	const counts = {
		all: state.codexAccounts.length,
		enabled: state.codexAccounts.filter((account) => account.enabled).length,
		attention: state.codexAccounts.filter(attention).length,
		disabled: state.codexAccounts.filter((account) => !account.enabled).length,
	};

	const accounts = state.codexAccounts
		.filter((account) =>
			filter === "enabled" ? account.enabled : filter === "disabled" ? !account.enabled : filter === "attention" ? attention(account) : true,
		)
		.toSorted((left, right) => {
			if (left.enabled !== right.enabled) return left.enabled ? -1 : 1;
			if (sort === "name") return accountTitle(left).localeCompare(accountTitle(right));
			if (sort === "usage") return (statsById.get(right.id)?.totalTokens ?? 0) - (statsById.get(left.id)?.totalTokens ?? 0);
			return riskScore(quotaById.get(left.id), now) - riskScore(quotaById.get(right.id), now);
		});

	function sourcesFor(account: CodexAccount): RoutingSource[] {
		const identitiesFor = (targetType: "group" | "account", targetId: string): RouteIdentity[] =>
			state.routes
				.filter((route) => route.targetType === targetType && route.targetId === targetId)
				.map((route) => ({ id: route.userId, name: state.users.find((user) => user.id === route.userId)?.name ?? route.userId }));
		const sources: RoutingSource[] = state.accountGroups
			.filter((group) => group.accountIds.includes(account.id))
			.map((group) => ({ group, identities: identitiesFor("group", group.id) }));
		const direct = identitiesFor("account", account.id);
		if (direct.length) sources.push({ group: null, identities: direct });
		return sources;
	}

	function shareFor(account: CodexAccount): { group: string; value: number } | null {
		for (const group of state.accountGroups) {
			if (group.strategy !== "weighted-round-robin" || !group.accountIds.includes(account.id)) continue;
			const member = quota?.groups.find((entry) => entry.groupId === group.id)?.members.find((entry) => entry.accountId === account.id);
			if (member) return { group: group.name, value: member.share };
		}
		return null;
	}

	async function refreshAccount(account: CodexAccount) {
		setRefreshing((current) => new Set(current).add(account.id));
		try {
			const subscription = await api.getCodexAccountSubscription(account.id);
			setSubscriptions((current) => ({ ...current, [account.id]: subscription }));
			await refreshQuota(account.id);
		} catch (error) {
			reportError(error, "额度同步失败。");
		} finally {
			setRefreshing((current) => {
				const next = new Set(current);
				next.delete(account.id);
				return next;
			});
		}
	}

	if (state.codexAccounts.length === 0) {
		return (
			<Empty className="rounded-xl border border-dashed py-16">
				<EmptyHeader>
					<EmptyMedia variant="icon">
						<UsersIcon />
					</EmptyMedia>
					<EmptyTitle>还没有 Codex 账户</EmptyTitle>
					<EmptyDescription>登录或导入凭据后，即可在这里查看额度与耗尽预测。</EmptyDescription>
				</EmptyHeader>
				<EmptyContent>
					<Button disabled={deviceLoginActive} onClick={startDeviceLogin} type="button">
						<PlusIcon data-icon="inline-start" />
						登录新账户
					</Button>
				</EmptyContent>
			</Empty>
		);
	}

	return (
		<div className="flex flex-col gap-4">
			<div className="flex flex-wrap items-center gap-2">
				<Segmented
					label="筛选账户"
					onChange={setFilter}
					options={[
						{ value: "all", label: "全部", count: counts.all },
						{ value: "enabled", label: "启用", count: counts.enabled },
						{ value: "attention", label: "需关注", count: counts.attention },
						{ value: "disabled", label: "停用", count: counts.disabled },
					]}
					size="default"
					value={filter}
				/>
				<div className="flex-1" />
				<Select onValueChange={(value) => setSort(value as Sort)} value={sort}>
					<SelectTrigger aria-label="排序" size="sm">
						<SelectValue />
					</SelectTrigger>
					<SelectContent position="popper">
						<SelectItem value="risk">按额度风险排序</SelectItem>
						<SelectItem value="usage">按 7 天用量排序</SelectItem>
						<SelectItem value="name">按名称排序</SelectItem>
					</SelectContent>
				</Select>
				{quota?.sampledAt ? (
					<span className="rounded-full bg-surface-subtle px-2 py-[3px] text-[11.5px] leading-4 text-muted-foreground">
						额度同步于 <b className="font-semibold text-foreground">{formatClock(quota.sampledAt, false)}</b>
					</span>
				) : null}
				<Button disabled={quotaRefreshing} onClick={() => void refreshQuota(true)} size="sm" type="button" variant="outline">
					{quotaRefreshing ? <Spinner data-icon="inline-start" /> : <RefreshCwIcon data-icon="inline-start" />}
					全部刷新
				</Button>
			</div>

			{renderSummary(analytics.data === null)}

			{analytics.error ? <p className="text-xs text-danger">7 天用量读取失败：{analytics.error}</p> : null}

			{accounts.length === 0 ? (
				<Empty className="rounded-xl border border-dashed py-12">
					<EmptyHeader>
						<EmptyTitle>没有符合条件的账户</EmptyTitle>
					</EmptyHeader>
				</Empty>
			) : null}

			{accounts.map((account) => (
				<AccountCard
					account={account}
					key={account.id}
					onRefresh={() => void refreshAccount(account)}
					onRename={() => setRenaming(account)}
					quota={quotaById.get(account.id)}
					refreshing={refreshing.has(account.id)}
					share={shareFor(account)}
					sources={sourcesFor(account)}
					stats={statsById.get(account.id)}
					subscription={subscriptions[account.id] ?? null}
					totalTokens={totalTokens}
				/>
			))}

			{renaming ? (
				<AccountEditDialog
					account={renaming}
					busy={busy.has(`codex:${renaming.id}`)}
					onCancel={() => setRenaming(null)}
					onSave={({ name, proxy }) => {
						void updateCodexAccount(renaming, { name, enabled: renaming.enabled, proxy }).then((saved) => {
							if (saved) setRenaming(null);
						});
					}}
					onTestProxy={(proxy) => api.testCodexAccountProxy(renaming.id, proxy)}
				/>
			) : null}
		</div>
	);

	function renderSummary(loading: boolean) {
		const plans = new Map<string, number>();
		for (const account of state.codexAccounts) {
			const plan = planText(quotaById.get(account.id)?.planType ?? account.subscription?.planType) || "未知";
			plans.set(plan, (plans.get(plan) ?? 0) + 1);
		}
		const planEntries = [...plans.entries()];
		const risks = state.codexAccounts
			.filter((account) => account.enabled)
			.flatMap((account) => {
				const entry: AccountQuota | undefined = quotaById.get(account.id);
				return visibleWindows(entry?.windows ?? [])
					.filter((window) => window.exhaustsBeforeReset || window.limitReached)
					.map((window) => ({ account, window }));
			})
			.toSorted((left, right) => (left.window.exhaustAt ?? now) - (right.window.exhaustAt ?? now));
		const riskAccounts = new Set(risks.map((risk) => risk.account.id)).size;
		const upstream = (analytics.data?.upstreamAccounts ?? []).toSorted((left, right) => right.totalTokens - left.totalTokens);
		const totals = analytics.data?.totals;
		const rateLimited = upstream.filter((entry) => entry.rateLimited > 0).toSorted((left, right) => right.rateLimited - left.rateLimited);
		const rateLimitedTotal = rateLimited.reduce((sum, entry) => sum + entry.rateLimited, 0);
		const decided = totals ? totals.requests - totals.cancelled : 0;

		return (
			<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
				<StatCard
					detail={`启用 ${counts.enabled} · 停用 ${counts.disabled}`}
					label="Codex 账户"
					unit="个"
					value={state.codexAccounts.length}
				>
					<StackedBar
						height={6}
						segments={planEntries.map(([plan, count], index) => ({ key: plan, value: count, color: PLAN_COLORS[index % PLAN_COLORS.length] ?? VIZ.gray, label: plan }))}
					/>
					<Legend className="gap-x-3">
						{planEntries.map(([plan, count], index) => (
							<LegendItem color={PLAN_COLORS[index % PLAN_COLORS.length] ?? VIZ.gray} key={plan} label={plan} value={count} />
						))}
					</Legend>
				</StatCard>
				<StatCard
					aside={<TriangleAlertIcon className={cn("size-4", riskAccounts ? "text-danger" : "text-faint")} />}
					detail={risks.length ? "将在重置前耗尽" : "所有窗口都能撑到重置"}
					label="需关注"
					unit="个账户"
					value={riskAccounts}
					valueClassName={riskAccounts ? "text-danger" : ""}
				>
					<div className="flex flex-col gap-1.5">
						{risks.slice(0, 3).map(({ account, window }) => {
							const tone = windowTone(window, now);
							const at = window.exhaustAt;
							const soon = at !== null && at - now < 3 * 3_600_000;
							return (
								<div className="flex items-center gap-1.5 text-xs leading-4" key={`${account.id}:${window.id}`}>
									<span className="size-1.5 shrink-0 rounded-full" style={{ background: tone === "danger" ? VIZ.danger : VIZ.amber }} />
									<span className="min-w-0 flex-1 truncate text-muted-foreground">
										{shortAccountName(accountTitle(account))} · {windowLabel(window)}
									</span>
									<span className={cn("font-semibold whitespace-nowrap", tone === "danger" ? "text-danger" : "text-warning")}>
										{window.limitReached ? "已耗尽" : at === null ? forecastCopy(window, now).title : soon ? `约 ${formatSpan(at - now)}后` : formatDateTime(at)}
									</span>
								</div>
							);
						})}
					</div>
				</StatCard>
				{loading ? (
					<SummarySkeleton />
				) : (
					<StatCard
						detail={totals ? `${formatCost(totals.costUsd)} · ${formatCount(totals.requests)} 请求 · 成功率 ${formatPercent(decided > 0 ? (totals.completed + totals.incomplete) / decided : null)}` : null}
						label="7 天用量"
						unit="Token"
						value={formatTokens(upstream.reduce((sum, entry) => sum + entry.totalTokens, 0))}
					>
						<StackedBar
							height={6}
							segments={upstream.map((entry, index) => ({ key: entry.codexAccountId, value: entry.totalTokens, color: seriesColor(index), label: entry.codexAccountName }))}
						/>
						<Legend className="gap-x-2">
							{upstream.slice(0, 5).map((entry, index) => (
								<LegendItem color={seriesColor(index)} key={entry.codexAccountId} label={shortAccountName(entry.codexAccountName)} />
							))}
						</Legend>
					</StatCard>
				)}
				{loading ? (
					<SummarySkeleton />
				) : (
					<StatCard
						detail={totals ? `7 天 · 占全部请求 ${formatPercent(totals.requests ? rateLimitedTotal / totals.requests : null)}` : null}
						label="429 限流"
						unit="次"
						value={formatCount(rateLimitedTotal)}
					>
						<div className="flex flex-col gap-1.5">
							{rateLimited.slice(0, 3).map((entry, index) => (
								<div className="flex items-center gap-1.5 text-xs leading-4" key={entry.codexAccountId}>
									<span className="size-1.5 shrink-0 rounded-full" style={{ background: index === 0 ? VIZ.danger : VIZ.amber }} />
									<span className="min-w-0 flex-1 truncate text-muted-foreground">{entry.codexAccountName}</span>
									<span className="font-semibold tabular-nums">{entry.rateLimited}</span>
								</div>
							))}
							{rateLimited.length === 0 ? <span className="text-xs text-muted-foreground">7 天内无限流</span> : null}
						</div>
					</StatCard>
				)}
			</div>
		);
	}
}

function SummarySkeleton() {
	return <Skeleton className="h-[188px] rounded-xl" />;
}

export function PageAction() {
	const { openCredentialImport, startDeviceLogin, deviceLoginActive } = useAdmin();
	return (
		<div className="flex flex-wrap gap-2">
			<Button disabled={deviceLoginActive} onClick={openCredentialImport} type="button" variant="outline">
				<UploadIcon data-icon="inline-start" />
				导入凭据
			</Button>
			<Button disabled={deviceLoginActive} onClick={startDeviceLogin} type="button">
				<PlusIcon data-icon="inline-start" />
				登录新账户
			</Button>
		</div>
	);
}
