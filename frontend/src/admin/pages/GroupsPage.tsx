import { useMemo, useState } from "react";
import { GitBranchIcon, Layers3Icon, PlusIcon, TriangleAlertIcon, UsersIcon } from "lucide-react";

import type { AccountGroup } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { GroupEditorDialog } from "@/admin/dialogs";
import { STRATEGY_OPTIONS, strategyLabel } from "@/admin/routing-model";
import { openGroupEditor, useGroupEditor } from "@/admin/groups/editor-store";
import { GroupCard } from "@/admin/groups/GroupCard";
import { useAnalytics } from "@/admin/hooks";
import type { RouteIdentity } from "@/admin/quota/AccountCard";
import { accountTitle, successRate } from "@/admin/quota/model";
import { Legend, LegendItem, StackedBar } from "@/components/app/bars";
import { StatCard } from "@/components/app/stat-card";
import { VIZ } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { formatCost, formatCount, formatPercent, formatTokens } from "@/lib/format";
import { cn } from "@/lib/utils";

const STRATEGY_HINTS: Record<string, string> = {
	"round-robin": "依次分配",
	"weighted-round-robin": "按剩余额度 ÷ 剩余时间",
	fallback: "用户粘性，故障时切换",
};

export default function GroupsPage() {
	const { state, quota, saveRouting } = useAdmin();
	const analytics = useAnalytics({ range: "7d" });
	const editing = useGroupEditor();
	const [saving, setSaving] = useState(false);

	const quotaById = useMemo(() => new Map((quota?.accounts ?? []).map((entry) => [entry.accountId, entry])), [quota]);
	const statsById = useMemo(() => new Map((analytics.data?.upstreamAccounts ?? []).map((entry) => [entry.codexAccountId, entry])), [analytics.data]);
	const groupStats = useMemo(() => new Map((analytics.data?.upstreamGroups ?? []).map((entry) => [entry.accountGroupId, entry])), [analytics.data]);

	const routedTo = (targetType: "group" | "account", targetId: string): RouteIdentity[] =>
		state.routes
			.filter((route) => route.targetType === targetType && route.targetId === targetId)
			.map((route) => ({ id: route.userId, name: state.users.find((user) => user.id === route.userId)?.name ?? route.userId }));
	const groupsOf = (accountId: string) => state.accountGroups.filter((group) => group.accountIds.includes(accountId));

	async function saveGroup(group: AccountGroup) {
		setSaving(true);
		const exists = state.accountGroups.some((entry) => entry.id === group.id);
		const groups = exists ? state.accountGroups.map((entry) => (entry.id === group.id ? group : entry)) : [...state.accountGroups, group];
		const ok = await saveRouting(groups, state.routes);
		setSaving(false);
		if (ok) openGroupEditor(null);
	}

	const groupedTokens = (analytics.data?.upstreamGroups ?? []).filter((entry) => entry.accountGroupId).reduce((sum, entry) => sum + entry.totalTokens, 0);

	return (
		<div className="flex flex-col gap-4">
			{renderSummary(analytics.data === null)}
			{state.accountGroups.map((group) => (
				<GroupCard
					directIdentitiesOf={(accountId) => routedTo("account", accountId)}
					group={group}
					groupStats={groupStats.get(group.id)}
					identities={routedTo("group", group.id)}
					key={group.id}
					otherGroupsOf={groupsOf}
					quotaById={quotaById}
					statsById={statsById}
					totalTokens={groupedTokens}
				/>
			))}
			<NewGroupEntry />
			{editing ? (
				<GroupEditorDialog accounts={state.codexAccounts} entry={editing} onCancel={() => openGroupEditor(null)} onSave={saveGroup} saving={saving} />
			) : null}
		</div>
	);

	function renderSummary(loading: boolean) {
		const groups = state.accountGroups;
		const enabledAccounts = state.codexAccounts.filter((account) => account.enabled);
		const membership = state.codexAccounts.map((account) => groupsOf(account.id).length);
		const inGroups = membership.filter((count) => count > 0).length;
		const single = membership.filter((count) => count === 1).length;
		const multi = membership.filter((count) => count > 1).length;
		const outside = state.codexAccounts.filter((account) => groupsOf(account.id).length === 0);
		const upstreamGroups = analytics.data?.upstreamGroups ?? [];
		const grouped = upstreamGroups.filter((entry) => entry.accountGroupId);
		const direct = upstreamGroups.filter((entry) => !entry.accountGroupId);
		const groupedTokenSum = grouped.reduce((sum, entry) => sum + entry.totalTokens, 0);
		const directTokens = direct.reduce((sum, entry) => sum + entry.totalTokens, 0);
		const allTokens = groupedTokenSum + directTokens;
		const groupedRequests = grouped.reduce((sum, entry) => sum + entry.requests, 0);
		const groupedCost = grouped.reduce((sum, entry) => sum + entry.costUsd, 0);
		const groupedRate = successRate({
			requests: groupedRequests,
			completed: grouped.reduce((sum, entry) => sum + entry.completed, 0),
			incomplete: grouped.reduce((sum, entry) => sum + entry.incomplete, 0),
			cancelled: grouped.reduce((sum, entry) => sum + entry.cancelled, 0),
		});
		const unassigned = state.users.filter((user) => !state.routes.some((route) => route.userId === user.id));
		const unassignedEnabled = unassigned.filter((entry) => entry.enabled);
		const unassignedDisabled = unassigned.filter((entry) => !entry.enabled);

		return (
			<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
				<StatCard aside={<Layers3Icon className="size-4 text-faint" />} label="账户组" unit="个" value={groups.length}>
					<div className="flex flex-col gap-1.5">
						{groups.slice(0, 3).map((group, index) => {
							const count = routedTo("group", group.id).length;
							return (
								<div className="flex items-center gap-1.5 text-xs leading-4" key={group.id}>
									<span className="size-1.5 shrink-0 rounded-full" style={{ background: index === 0 ? VIZ.success : VIZ.amber }} />
									<span className="min-w-0 flex-1 truncate text-muted-foreground">
										{group.name} · {strategyLabel(group.strategy)}
									</span>
									<span className={cn("font-semibold whitespace-nowrap", count ? "text-foreground" : "text-warning")}>{count ? `${count} 个用户` : "未分配"}</span>
								</div>
							);
						})}
						{groups.length === 0 ? <span className="text-xs text-muted-foreground">还没有账户组</span> : null}
					</div>
				</StatCard>
				<StatCard
					aside={<UsersIcon className="size-4 text-faint" />}
					detail={
						outside.length
							? `${outside.map((account) => accountTitle(account)).join("、")} 未加入任何组${outside.every((account) => !account.enabled) ? "（已停用）" : ""}`
							: "所有账户都已加入账户组"
					}
					label="组内账户"
					value={`${inGroups} / ${state.codexAccounts.length}`}
				>
					<StackedBar
						height={6}
						segments={[
							{ key: "single", value: single, color: VIZ.blue, label: "仅一个组" },
							{ key: "multi", value: multi, color: VIZ.blueSoft, label: "多个组" },
						]}
						total={Math.max(1, enabledAccounts.length, state.codexAccounts.length)}
					/>
					<Legend className="gap-x-3">
						<LegendItem color={VIZ.blue} label="仅一个组" value={single} />
						<LegendItem color={VIZ.blueSoft} label="多个组" value={multi} />
					</Legend>
				</StatCard>
				{loading ? (
					<Skeleton className="h-[180px] rounded-xl" />
				) : (
					<StatCard
						aside={<GitBranchIcon className="size-4 text-faint" />}
						detail={`${formatCost(groupedCost)} · ${formatCount(groupedRequests)} 请求 · 成功率 ${formatPercent(groupedRate)}`}
						label="经账户组路由"
						unit="Token · 7 天"
						value={formatTokens(groupedTokenSum)}
					>
						<StackedBar
							height={6}
							segments={[
								{ key: "group", value: groupedTokenSum, color: VIZ.blue, label: "账户组" },
								{ key: "direct", value: directTokens, color: VIZ.muted, label: "直连账户" },
							]}
						/>
						<Legend className="gap-x-3">
							<LegendItem color={VIZ.blue} label="账户组" value={formatPercent(allTokens ? groupedTokenSum / allTokens : null)} />
							<LegendItem color={VIZ.muted} label="直连账户" value={formatPercent(allTokens ? directTokens / allTokens : null)} />
						</Legend>
					</StatCard>
				)}
				<StatCard
					aside={<TriangleAlertIcon className={cn("size-4", unassigned.length ? "text-warning" : "text-faint")} />}
					label="未分配用户"
					unit="个"
					value={unassigned.length}
					valueClassName={unassigned.length ? "text-warning" : ""}
				>
					<div className="flex flex-col gap-1.5">
						{unassignedEnabled.length ? (
							<UnassignedRow names={unassignedEnabled.map((entry) => entry.name)} status="启用中" tone="warning" />
						) : null}
						{unassignedDisabled.length ? (
							<UnassignedRow names={unassignedDisabled.map((entry) => entry.name)} status="已停用" tone="neutral" />
						) : null}
						{unassigned.length === 0 ? <span className="text-xs text-muted-foreground">所有用户都已分配</span> : null}
					</div>
				</StatCard>
			</div>
		);
	}
}

function UnassignedRow({ names, status, tone }: { names: string[]; status: string; tone: "warning" | "neutral" }) {
	const label = names.length > 1 ? `${names[0]} 等 ${names.length} 个` : (names[0] ?? "");
	return (
		<div className="flex items-center gap-1.5 text-xs leading-4">
			<span className="size-1.5 shrink-0 rounded-full" style={{ background: tone === "warning" ? VIZ.amber : VIZ.muted }} />
			<span className="min-w-0 flex-1 truncate text-muted-foreground">{label}</span>
			<span className={cn("font-semibold whitespace-nowrap", tone === "warning" ? "text-warning" : "text-muted-foreground")}>{status}</span>
		</div>
	);
}

/** Figma "new-group-entry": dashed call to action listing the strategies. */
function NewGroupEntry() {
	return (
		<Card className="flex-row flex-wrap items-center gap-4 border border-dashed border-border-strong bg-transparent px-6 py-6 ring-0">
			<Button aria-label="新建账户组" onClick={() => openGroupEditor("new")} size="icon-lg" type="button" variant="outline">
				<PlusIcon />
			</Button>
			<div className="flex min-w-0 flex-1 flex-col gap-1">
				<span className="text-[15px] leading-5 font-semibold">新建账户组</span>
				<span className="text-[12.5px] leading-5 text-muted-foreground">选择分配策略，再添加 Codex 账户。</span>
				<div className="mt-1 flex flex-wrap gap-2">
					{STRATEGY_OPTIONS.map((option) => (
						<span className="inline-flex items-center gap-1.5 rounded-full border bg-card px-2 py-[3px] text-xs" key={option.value}>
							<b className="font-semibold">{strategyLabel(option.value)}</b>
							<span className="text-faint">{STRATEGY_HINTS[option.value]}</span>
						</span>
					))}
				</div>
			</div>
			<Button onClick={() => openGroupEditor("new")} type="button" variant="outline">
				<PlusIcon data-icon="inline-start" />
				新建账户组
			</Button>
		</Card>
	);
}

export function PageAction() {
	return (
		<Button onClick={() => openGroupEditor("new")} type="button">
			<PlusIcon data-icon="inline-start" />
			新建账户组
		</Button>
	);
}
