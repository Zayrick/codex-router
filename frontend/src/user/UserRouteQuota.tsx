import { useState } from "react";
import { CheckCircle2Icon, Layers3Icon, TimerIcon, UserRoundIcon } from "lucide-react";

import type { AccountQuota, UsageAnalytics, WindowForecast } from "@/admin-api";
import { formatTtl, strategyLabel } from "@/admin/routing-model";
import { TONE_COLOR, accountTone, forecastCopy, formatResetAt, initial, planText, visibleWindows, windowLabel, windowTone, type RiskTone } from "@/admin/quota/model";
import { QuotaTimeline, TimelineLegend } from "@/admin/quota/QuotaTimeline";
import { Meter } from "@/components/app/bars";
import { Tag } from "@/components/app/badges";
import { SectionCard } from "@/components/app/section-card";
import { Segmented } from "@/components/app/segmented";
import { Card } from "@/components/ui/card";
import { formatCount, formatDateTime, formatTokens } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { AccountGroupStrategy } from "@/admin-api";
import type { UserProfile } from "@/user-api";

const STATUS_COPY: Record<RiskTone, { label: string; tone: "danger" | "warning" | "success" | "neutral" } | null> = {
	danger: { label: "即将限流", tone: "danger" },
	warning: { label: "额度偏紧", tone: "warning" },
	success: { label: "充足", tone: "success" },
	neutral: null,
};

/** Figma "section · 路由额度": quota of the accounts the signed-in identity is routed to. */
export default function UserRouteQuota({ profile, analytics, now }: { profile: UserProfile; analytics: UsageAnalytics | null; now: number }) {
	const [span, setSpan] = useState<"8d" | "24h">("8d");
	if (!profile.visible || !profile.quota) return null;
	const accounts = profile.quota.accounts;
	const group = profile.group ?? null;
	const title = group ? `路由额度 · ${group.name}` : accounts.length === 1 ? `路由额度 · ${accounts[0]?.accountName ?? ""}` : "路由额度";
	const statsById = new Map((analytics?.upstreamAccounts ?? []).map((entry) => [entry.codexAccountId, entry]));

	return (
		<section aria-label="路由额度" className="flex flex-col gap-4 pt-4">
			<div className="flex flex-wrap items-center gap-3">
				<span className="flex size-8 shrink-0 items-center justify-center rounded-[10px] border bg-card">
					{group ? <Layers3Icon className="size-4" /> : <UserRoundIcon className="size-4" />}
				</span>
				<div className="flex min-w-0 flex-1 flex-col gap-0.5">
					<div className="flex flex-wrap items-center gap-2">
						<h2 className="text-lg leading-6 font-bold tracking-[-0.02em]">{title}</h2>
						{group ? <Tag className="font-bold text-foreground">{strategyLabel(group.strategy as AccountGroupStrategy)}</Tag> : null}
						{group && group.strategy !== "fallback" && group.sessionAffinity ? (
							<Tag className="font-bold text-foreground">会话保持 {formatTtl(group.sessionAffinityTtl)}</Tag>
						) : null}
						<Tag className="font-bold text-foreground">{accounts.length} 个账户</Tag>
					</div>
					<p className="text-[12.5px] leading-5 text-muted-foreground">
						{accounts.length > 1
							? `请求在 ${accounts.length} 个账户间分配，单个耗尽时自动切换，全部耗尽后返回 429。`
							: accounts.length === 1
								? "请求固定使用该账户，额度耗尽后返回 429。"
								: "当前未分配可用账户。"}
					</p>
				</div>
				{profile.quota.sampledAt ? <span className="text-[11.5px] leading-4 text-faint">额度同步于 {formatDateTime(profile.quota.sampledAt)}</span> : null}
			</div>

			{accounts.length ? (
				<div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
					{accounts.map((account) => {
						const stats = statsById.get(account.accountId);
						return <QuotaTile account={account} key={account.accountId} now={now} requests={stats?.requests ?? 0} rateLimited={stats?.rateLimited ?? 0} tokens={stats?.totalTokens ?? 0} />;
					})}
				</div>
			) : null}

			{accounts.length ? (
				<SectionCard
					actions={
						<Segmented
							label="时间轴范围"
							onChange={setSpan}
							options={[
								{ value: "8d", label: "8 天" },
								{ value: "24h", label: "24 小时" },
							]}
							value={span}
						/>
					}
					className="pb-4"
					description="红色为预计耗尽至重置的不可用时段"
					title="额度时间轴"
				>
					<QuotaTimeline
						accounts={accounts.map((account) => ({ id: account.accountId, title: account.accountName, plan: account.planType, windows: account.windows }))}
						mode={span === "8d" ? "days" : "hours"}
						now={now}
					/>
					<TimelineLegend note="预测基于该账户最近 3 小时的消耗速度；百分比为已用额度。" />
				</SectionCard>
			) : null}
		</section>
	);
}

function QuotaTile({
	account,
	now,
	requests,
	tokens,
	rateLimited,
}: {
	account: AccountQuota;
	now: number;
	requests: number;
	tokens: number;
	rateLimited: number;
}) {
	const status = account.enabled ? STATUS_COPY[accountTone(account, now)] : { label: "已停用", tone: "neutral" as const };
	const windows = visibleWindows(account.windows);
	return (
		<Card className="min-w-0 gap-4 p-6">
			<div className="flex items-center gap-2.5">
				<span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-primary text-xs font-bold text-primary-foreground">{initial(account.accountName)}</span>
				<div className="flex min-w-0 flex-1 flex-col">
					<span className="truncate text-[13px] leading-5 font-semibold">{account.accountName}</span>
					{account.planType ? <span className="text-[11.5px] leading-4 text-faint">{planText(account.planType)}</span> : null}
				</div>
				{status ? (
					<Tag className="font-bold" tone={status.tone}>
						<span aria-hidden="true" className={cn("size-1.5 rounded-full", status.tone === "danger" ? "bg-danger" : status.tone === "warning" ? "bg-warning" : status.tone === "success" ? "bg-success" : "bg-faint")} />
						{status.label}
					</Tag>
				) : null}
			</div>
			{windows.length ? windows.map((window) => <WindowRow key={window.id} now={now} window={window} />) : <p className="text-xs text-muted-foreground">额度尚未同步</p>}
			<div className="grid grid-cols-3 gap-4 border-t pt-3">
				<Stat label="你的请求" value={formatCount(requests)} />
				<Stat label="Token" value={formatTokens(tokens)} />
				<Stat danger={rateLimited > 0} label="429 次数" value={formatCount(rateLimited)} />
			</div>
		</Card>
	);
}

function WindowRow({ window, now }: { window: WindowForecast; now: number }) {
	const tone = windowTone(window, now);
	const forecast = forecastCopy(window, now);
	const used = window.usedPercent;
	const valueColor = tone === "danger" ? "text-danger" : tone === "warning" ? "text-warning" : "text-foreground";
	return (
		<div className="flex flex-col gap-1.5">
			<div className="flex items-baseline justify-between gap-2">
				<span className="flex items-baseline gap-2">
					<span className="text-xs font-medium text-muted-foreground">
						{window.name || "Codex"} {windowLabel(window)}
					</span>
					<span className={cn("text-xl leading-7 font-bold tracking-[-0.03em] tabular-nums", valueColor)}>{used === null ? "—" : `${Math.round(used)}%`}</span>
					<span className="text-[11px] text-faint">已用</span>
				</span>
				<span className="text-[11.5px] text-faint">重置 {formatResetAt(window.resetAt, now)}</span>
			</div>
			<Meter color={TONE_COLOR[tone]} label={`${windowLabel(window)}已用`} ratio={(used ?? 0) / 100} />
			<span className={cn("flex items-center gap-1.5 text-[11.5px] leading-4", forecast.exhausts ? (tone === "danger" ? "font-medium text-danger" : "font-medium text-warning") : "text-muted-foreground")}>
				{forecast.exhausts ? <TimerIcon className="size-3" /> : <CheckCircle2Icon className="size-3 text-success" />}
				{forecast.exhausts && forecast.detail ? `${forecast.title}，${forecast.detail}` : forecast.title}
			</span>
		</div>
	);
}

function Stat({ label, value, danger = false }: { label: string; value: string; danger?: boolean }) {
	return (
		<div className="flex min-w-0 flex-col">
			<span className="text-[11px] leading-4 text-faint">{label}</span>
			<span className={cn("text-[13px] leading-5 font-semibold tabular-nums", danger && "text-danger")}>{value}</span>
		</div>
	);
}
