import { useState } from "react";
import { InfoIcon, Layers3Icon, Link2Icon, PencilIcon, PlusIcon, TriangleAlertIcon } from "lucide-react";

import type { AccountGroup, AccountQuota, CodexAccount, UpstreamAccountStats, UpstreamGroupStats, WindowForecast } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { DeleteConfirmationDialog } from "@/admin/dialogs";
import { formatTtl, strategyLabel } from "@/admin/routing-model";
import { IdentityChip, type RouteIdentity } from "@/admin/quota/AccountCard";
import { TONE_COLOR, accountTitle, accountTone, cacheRate, planText, shortAccountName, successRate, visibleWindows, windowLabel, windowTone } from "@/admin/quota/model";
import { UsagePanel } from "@/admin/quota/UsagePanel";
import { Legend, LegendItem, Meter, StackedBar } from "@/components/app/bars";
import { Tag } from "@/components/app/badges";
import { seriesColor } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { formatClock, formatCost, formatCount, formatPercent, formatTokens } from "@/lib/format";
import { cn } from "@/lib/utils";

import { AssignIdentitiesDialog } from "./AssignIdentitiesDialog";
import { openGroupEditor } from "./editor-store";

const STRATEGY_COPY: Record<string, (ttl: string, affinity: boolean) => string> = {
	"weighted-round-robin": (ttl, affinity) => `按剩余额度 ÷ 剩余时间分配${affinity ? `；同一会话 ${formatTtl(ttl)} 内固定到同一账户` : ""}。`,
	"round-robin": (ttl, affinity) => `按账户顺序轮流分配${affinity ? `；同一会话 ${formatTtl(ttl)} 内固定到同一账户` : ""}。`,
	fallback: () => "每个用户固定使用首选账户，不可用时按顺序切换。",
};

/** Figma "group · …": header, members with live weights, 7-day usage and assigned identities. */
export function GroupCard({
	group,
	identities,
	quotaById,
	statsById,
	groupStats,
	totalTokens,
	otherGroupsOf,
	directIdentitiesOf,
}: {
	group: AccountGroup;
	identities: RouteIdentity[];
	quotaById: Map<string, AccountQuota>;
	statsById: Map<string, UpstreamAccountStats>;
	groupStats: UpstreamGroupStats | undefined;
	/** Tokens routed through all groups, for the share. */
	totalTokens: number;
	otherGroupsOf: (accountId: string) => AccountGroup[];
	directIdentitiesOf: (accountId: string) => RouteIdentity[];
}) {
	const { state, quota, saveRouting } = useAdmin();
	const [assigning, setAssigning] = useState(false);
	const members = group.accountIds
		.map((id) => state.codexAccounts.find((account) => account.id === id))
		.filter((account): account is CodexAccount => Boolean(account));
	const shares = quota?.groups.find((entry) => entry.groupId === group.id)?.members ?? [];
	const fallback = group.strategy === "fallback";
	const unassigned = identities.length === 0;

	async function remove() {
		await saveRouting(
			state.accountGroups.filter((entry) => entry.id !== group.id),
			state.routes.filter((route) => route.targetType !== "group" || route.targetId !== group.id),
		);
	}

	return (
		<Card className="gap-0 overflow-hidden py-0">
			<div className="flex flex-wrap items-center gap-4 border-b px-6 py-4">
				<span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary-soft">
					<Layers3Icon className="size-[18px]" />
				</span>
				<div className="flex min-w-0 flex-1 flex-col gap-1">
					<div className="flex flex-wrap items-center gap-2">
						<span className="truncate text-[15px] leading-5 font-semibold tracking-[-0.01em]">{group.name}</span>
						<Tag>{strategyLabel(group.strategy)}</Tag>
						<Tag>{fallback ? "用户保持" : group.sessionAffinity ? `会话保持 ${formatTtl(group.sessionAffinityTtl)}` : "不保持会话"}</Tag>
						<Tag>{members.length} 个账户</Tag>
						{unassigned ? <Tag tone="warning">未分配</Tag> : <Tag tone="success">{identities.length} 个用户</Tag>}
					</div>
					<p className="text-[12.5px] leading-5 text-muted-foreground">
						{(STRATEGY_COPY[group.strategy] ?? STRATEGY_COPY["round-robin"])?.(group.sessionAffinityTtl, group.sessionAffinity)}
					</p>
				</div>
				<div className="flex flex-wrap items-center gap-2">
					<Button onClick={() => setAssigning(true)} size="sm" type="button" variant="outline">
						<Link2Icon data-icon="inline-start" />
						分配给用户
					</Button>
					<Button onClick={() => openGroupEditor(group)} size="sm" type="button" variant="outline">
						<PencilIcon data-icon="inline-start" />
						编辑
					</Button>
					<DeleteConfirmationDialog
						description="分配到此组的用户将变为未分配。此操作无法撤销。"
						onConfirm={() => void remove()}
						title={`删除账户组“${group.name}”？`}
						trigger={
							<Button size="sm" type="button" variant="destructive">
								删除
							</Button>
						}
					/>
				</div>
			</div>

			{unassigned && fallback ? (
				<div className="flex flex-wrap items-center gap-3 border-b bg-warning-soft px-6 py-3">
					<TriangleAlertIcon className="size-4 shrink-0 text-warning" />
					<div className="flex min-w-0 flex-1 flex-col">
						<span className="text-[12.5px] leading-5 font-bold text-warning">尚未分配用户</span>
						<span className="text-xs leading-5 text-warning">分配后，首选账户不可用时将按顺序自动切换。</span>
					</div>
					<Button onClick={() => setAssigning(true)} size="sm" type="button">
						<Link2Icon data-icon="inline-start" />
						分配给用户
					</Button>
				</div>
			) : null}

			<div className="flex flex-col gap-6 p-6 lg:flex-row">
				<div className="flex min-w-0 flex-1 flex-col gap-4">
					{members.length === 0 ? (
						<p className="rounded-[10px] border border-dashed p-6 text-center text-xs text-muted-foreground">组内还没有账户，点击「编辑」添加 Codex 账户。</p>
					) : fallback ? (
						<FallbackOrder directIdentitiesOf={directIdentitiesOf} group={group} members={members} otherGroupsOf={otherGroupsOf} quotaById={quotaById} />
					) : (
						<WeightedMembers
							groupStats={groupStats}
							members={members}
							quotaById={quotaById}
							sampledAt={quota?.sampledAt ?? null}
							shares={shares}
							statsById={statsById}
							weighted={group.strategy === "weighted-round-robin"}
						/>
					)}
				</div>
				<UsagePanel
					aside={groupStats && totalTokens > 0 ? `占全部 ${formatPercent(groupStats.totalTokens / totalTokens)}` : "暂无流量"}
					className="lg:w-[300px] lg:shrink-0"
					stats={
						groupStats
							? [
									{ label: "成本", value: formatCost(groupStats.costUsd) },
									{ label: "请求", value: formatCount(groupStats.requests) },
									{ label: "成功率", value: formatPercent(successRate(groupStats)) },
									{ label: "429", value: formatCount(groupStats.rateLimited), tone: groupStats.rateLimited ? "danger" : "default" },
									{ label: "会话", value: formatCount(groupStats.sessions) },
									{ label: "缓存率", value: formatPercent(cacheRate(groupStats)) },
								]
							: []
					}
					tokens={formatTokens(groupStats?.totalTokens ?? 0)}
					trend={groupStats?.trend ?? null}
					unit={groupStats ? "Token" : "Token · 0 请求"}
				>
					{groupStats?.consumers.length ? (
						<div className="flex flex-col gap-2">
							<span className="text-[11.5px] leading-4 font-medium text-faint">按用户（7 天请求）</span>
							{groupStats.consumers.slice(0, 6).map((consumer) => (
								<div className="flex items-center gap-2 text-[12.5px] leading-5" key={consumer.userId}>
									<IdentityChip identity={{ id: consumer.userId, name: consumer.userName }} />
									<span className="ml-auto font-semibold tabular-nums">{formatCount(consumer.requests)}</span>
								</div>
							))}
						</div>
					) : (
						<p className="text-xs text-muted-foreground">{unassigned ? "分配用户后显示用量" : "近 7 天无请求"}</p>
					)}
				</UsagePanel>
			</div>

			<div className="flex flex-wrap items-center gap-2 border-t bg-surface-subtle/60 px-6 py-3 text-xs text-muted-foreground">
				<Link2Icon className="size-3.5" />
				<span>已分配</span>
				{identities.map((identity) => (
					<IdentityChip identity={identity} key={identity.id} />
				))}
				{identities.length === 0 ? <span className="text-faint">暂无</span> : null}
				<Button className="ml-auto" onClick={() => setAssigning(true)} size="sm" type="button" variant="ghost">
					<PlusIcon data-icon="inline-start" />
					添加用户
				</Button>
			</div>

			{assigning ? <AssignIdentitiesDialog group={group} onClose={() => setAssigning(false)} /> : null}
		</Card>
	);
}

/** Two shortest windows of a member, short first ("5 小时", "7 天"). */
function pairWindows(quota: AccountQuota | undefined): [WindowForecast | undefined, WindowForecast | undefined] {
	const windows = visibleWindows(quota?.windows ?? []).toSorted((left, right) => (left.limitWindowSeconds ?? 0) - (right.limitWindowSeconds ?? 0));
	return [windows[0], windows[1]];
}

function windowHeaders(members: CodexAccount[], quotaById: Map<string, AccountQuota>): [string, string] {
	for (const member of members) {
		const [short, long] = pairWindows(quotaById.get(member.id));
		if (short && long) return [`${windowLabel(short)}已用`, `${windowLabel(long)}已用`];
	}
	return ["5 小时已用", "7 天已用"];
}

function UsedCell({ window, now }: { window: WindowForecast | undefined; now: number }) {
	if (!window || window.usedPercent === null) return <span className="text-faint">—</span>;
	return (
		<span className="flex items-center gap-2">
			<Meter className="w-14" color={TONE_COLOR[windowTone(window, now)]} ratio={window.usedPercent / 100} />
			<span className="font-semibold tabular-nums">{Math.round(window.usedPercent)}%</span>
		</span>
	);
}

function memberStatus(account: CodexAccount, quota: AccountQuota | undefined, now: number) {
	if (!account.enabled) return <Tag>已停用</Tag>;
	if (quota?.quarantined) return <Tag tone="danger">已暂停</Tag>;
	const tone = accountTone(quota, now);
	if (tone === "danger") return <Tag tone="danger">即将耗尽</Tag>;
	if (tone === "warning") {
		const tight = visibleWindows(quota?.windows ?? []).find((window) => windowTone(window, now) === "warning");
		return <Tag tone="warning">{tight ? `${windowLabel(tight)}偏紧` : "偏紧"}</Tag>;
	}
	return <Tag tone="success">正常</Tag>;
}

function memberMeta(account: CodexAccount, quota: AccountQuota | undefined, rateLimited: number | undefined): string {
	return [
		planText(quota?.planType ?? account.subscription?.planType),
		account.enabled ? "启用中" : "已停用",
		rateLimited ? `429 × ${rateLimited}` : null,
	]
		.filter(Boolean)
		.join(" · ");
}

function WeightedMembers({
	members,
	quotaById,
	statsById,
	shares,
	groupStats,
	sampledAt,
	weighted,
}: {
	members: CodexAccount[];
	quotaById: Map<string, AccountQuota>;
	statsById: Map<string, UpstreamAccountStats>;
	shares: { accountId: string; share: number }[];
	groupStats: UpstreamGroupStats | undefined;
	sampledAt: number | null;
	weighted: boolean;
}) {
	const { now } = useAdmin();
	const [shortHeader, longHeader] = windowHeaders(members, quotaById);
	// Usage through this group only; account totals when the group has no stats yet.
	const inGroup = (id: string) => groupStats?.accounts.find((entry) => entry.codexAccountId === id);
	const memberTokens = members.map((member) => (groupStats ? (inGroup(member.id)?.totalTokens ?? 0) : 0));
	const tokenTotal = memberTokens.reduce((sum, value) => sum + value, 0);
	const shareOf = (id: string) => shares.find((entry) => entry.accountId === id)?.share ?? 0;
	const exhausting = members.filter((member) => member.enabled && accountTone(quotaById.get(member.id), now) === "danger");
	return (
		<>
			<div className="flex items-baseline gap-2">
				<h3 className="text-[13px] leading-5 font-bold">成员与实时权重</h3>
				{sampledAt ? <span className="text-[11.5px] text-faint">按 {formatClock(sampledAt, false)} 额度快照计算</span> : null}
			</div>
			<Table className="min-w-[620px]">
				<TableHeader>
					<TableRow className="hover:bg-transparent">
						<TableHead className="h-8 px-0 text-[11.5px] font-medium text-faint">账户</TableHead>
						<TableHead className="h-8 text-[11.5px] font-medium text-faint">{shortHeader}</TableHead>
						<TableHead className="h-8 text-[11.5px] font-medium text-faint">{longHeader}</TableHead>
						<TableHead className="h-8 text-[11.5px] font-medium text-faint">{weighted ? "当前权重" : "当前分配"}</TableHead>
						<TableHead className="h-8 text-[11.5px] font-medium text-faint">7 天实际占比</TableHead>
						<TableHead className="h-8 text-[11.5px] font-medium text-faint">状态</TableHead>
					</TableRow>
				</TableHeader>
				<TableBody>
					{members.map((member, index) => {
						const quota = quotaById.get(member.id);
						const [short, long] = pairWindows(quota);
						const stats = statsById.get(member.id);
						const share = shareOf(member.id);
						return (
							<TableRow className="text-[13px] hover:bg-transparent" key={member.id}>
								<TableCell className="px-0 py-3">
									<span className="flex items-center gap-2">
										<span className="size-2 shrink-0 rounded-[2px]" style={{ background: seriesColor(index) }} />
										<span className="truncate">{accountTitle(member)}</span>
									</span>
									<span className="block pl-4 text-[11.5px] text-faint">{memberMeta(member, quota, inGroup(member.id)?.rateLimited ?? (groupStats ? 0 : stats?.rateLimited))}</span>
								</TableCell>
								<TableCell>
									<UsedCell now={now} window={short} />
								</TableCell>
								<TableCell>
									<UsedCell now={now} window={long} />
								</TableCell>
								<TableCell>
									<span className="flex items-center gap-2">
										<Meter className="w-20" color={seriesColor(0)} ratio={share} />
										<span className="font-semibold tabular-nums">{formatPercent(share, 0)}</span>
									</span>
								</TableCell>
								<TableCell className="tabular-nums text-muted-foreground">
									{formatTokens(memberTokens[index] ?? 0)} · {formatPercent(tokenTotal ? (memberTokens[index] ?? 0) / tokenTotal : null)}
								</TableCell>
								<TableCell>{memberStatus(member, quota, now)}</TableCell>
							</TableRow>
						);
					})}
				</TableBody>
			</Table>
			<div className="flex flex-col gap-3 rounded-[10px] bg-surface-subtle p-4">
				<div className="flex flex-wrap items-center justify-between gap-2">
					<span className="text-[12.5px] leading-5 font-bold">流量分配：当前权重 vs 7 天实际</span>
					{groupStats ? (
						<span className="text-[11.5px] text-faint">
							活跃会话 {formatCount(groupStats.sessions)}
							{groupStats.sessionAffinityRate !== null ? ` · 会话保持命中 ${formatPercent(groupStats.sessionAffinityRate, 0)}` : ""}
						</span>
					) : null}
				</div>
				<SplitRow label={weighted ? "当前权重" : "当前分配"} values={members.map((member) => shareOf(member.id))} />
				<SplitRow label="7 天实际" values={memberTokens} />
				<Legend className="gap-x-4">
					{members.map((member, index) => (
						<LegendItem color={seriesColor(index)} key={member.id} label={shortAccountName(accountTitle(member))} />
					))}
				</Legend>
				{exhausting.length ? (
					<p className="flex items-center gap-1.5 text-xs text-warning">
						<InfoIcon className="size-3.5" />
						{exhausting.map((member) => shortAccountName(accountTitle(member))).join("、")} 将在重置前耗尽
						{weighted ? `，当前权重 ${exhausting.map((member) => formatPercent(shareOf(member.id), 0)).join("、")}` : ""}。
					</p>
				) : null}
			</div>
		</>
	);
}

function SplitRow({ label, values }: { label: string; values: number[] }) {
	return (
		<div className="flex items-center gap-3">
			<span className="w-[60px] shrink-0 text-xs text-muted-foreground">{label}</span>
			<StackedBar className="flex-1" height={8} segments={values.map((value, index) => ({ key: String(index), value, color: seriesColor(index) }))} />
		</div>
	);
}

function FallbackOrder({
	group,
	members,
	quotaById,
	otherGroupsOf,
	directIdentitiesOf,
}: {
	group: AccountGroup;
	members: CodexAccount[];
	quotaById: Map<string, AccountQuota>;
	otherGroupsOf: (accountId: string) => AccountGroup[];
	directIdentitiesOf: (accountId: string) => RouteIdentity[];
}) {
	const { now } = useAdmin();
	const [shortHeader, longHeader] = windowHeaders(members, quotaById);
	const shared = members
		.map((member) => ({ member, groups: otherGroupsOf(member.id).filter((entry) => entry.id !== group.id) }))
		.filter((entry) => entry.groups.length);
	return (
		<>
			<h3 className="text-[13px] leading-5 font-bold">切换顺序</h3>
			<Table className="min-w-[560px]">
				<TableHeader>
					<TableRow className="hover:bg-transparent">
						<TableHead className="h-8 w-12 px-0 text-[11.5px] font-medium text-faint">顺序</TableHead>
						<TableHead className="h-8 text-[11.5px] font-medium text-faint">账户</TableHead>
						<TableHead className="h-8 text-[11.5px] font-medium text-faint">{shortHeader}</TableHead>
						<TableHead className="h-8 text-[11.5px] font-medium text-faint">{longHeader}</TableHead>
						<TableHead className="h-8 text-[11.5px] font-medium text-faint">角色</TableHead>
						<TableHead className="h-8 text-[11.5px] font-medium text-faint">状态</TableHead>
					</TableRow>
				</TableHeader>
				<TableBody>
					{members.map((member, index) => {
						const quota = quotaById.get(member.id);
						const [short, long] = pairWindows(quota);
						const direct = directIdentitiesOf(member.id);
						const available = member.enabled && !quota?.quarantined && !(quota?.windows.some((window) => window.limitReached) ?? false);
						return (
							<TableRow className="text-[13px] hover:bg-transparent" key={member.id}>
								<TableCell className="px-0 py-3">
									<span className="flex size-6 items-center justify-center rounded-full border text-xs font-semibold">{index + 1}</span>
								</TableCell>
								<TableCell>
									<span className="block truncate">{accountTitle(member)}</span>
									<span className="block text-[11.5px] text-faint">
										{[planText(quota?.planType ?? member.subscription?.planType), direct.length ? `也被 ${direct.map((entry) => entry.name).join("、")} 直连` : null]
											.filter(Boolean)
											.join(" · ")}
									</span>
								</TableCell>
								<TableCell>
									<UsedCell now={now} window={short} />
								</TableCell>
								<TableCell>
									<UsedCell now={now} window={long} />
								</TableCell>
								<TableCell className="text-muted-foreground">{index === 0 ? "首选账户" : "首选不可用时切换"}</TableCell>
								<TableCell>{available ? <Tag tone="success">可用</Tag> : <Tag tone="danger">不可用</Tag>}</TableCell>
							</TableRow>
						);
					})}
				</TableBody>
			</Table>
			{shared.length ? (
				<p className={cn("flex items-center gap-1.5 text-xs text-muted-foreground")}>
					<InfoIcon className="size-3.5" />
					{shared.map((entry) => `${shortAccountName(accountTitle(entry.member))} 同时属于${entry.groups.map((other) => other.name).join("、")}`).join("；")}，两组将共享其额度。
				</p>
			) : null}
		</>
	);
}
