import { Layers3Icon, UserRoundIcon } from "lucide-react";
import { useState } from "react";

import type { IdentityStats, UpstreamAccountStats, UpstreamGroupStats, UsageAnalytics } from "@/admin-api";
import { Legend, LegendItem, StackedBar } from "@/components/app/bars";
import { Tag } from "@/components/app/badges";
import { SectionCard } from "@/components/app/section-card";
import { Segmented } from "@/components/app/segmented";
import { STATUS_META, TOKEN_PARTS, VIZ, tokenSplit } from "@/components/app/viz";
import { formatCost, formatCount, formatPercent, formatTokens } from "@/lib/format";

import { NoData, RankRow } from "./common";
import { cacheRate, successRate } from "./metrics";

const TOP_IDENTITIES = 5;

function tokenSegments(value: Parameters<typeof tokenSplit>[0]) {
	const split = tokenSplit(value);
	return [
		{ key: "input", value: split.input, color: TOKEN_PARTS.input.color, label: TOKEN_PARTS.input.label },
		{ key: "cached", value: split.cached, color: TOKEN_PARTS.cached.color, label: TOKEN_PARTS.cached.label },
		{ key: "output", value: split.output + split.reasoning, color: TOKEN_PARTS.output.color, label: TOKEN_PARTS.output.label },
	];
}

/** Figma "card · 下游调用量": identities ranked by tokens or cost. */
export function DownstreamCard({ analytics, onSelect }: { analytics: UsageAnalytics; onSelect?: (identity: IdentityStats) => void }) {
	const [metric, setMetric] = useState<"tokens" | "cost">("tokens");
	const value = (row: { totalTokens: number; costUsd: number }) => (metric === "tokens" ? row.totalTokens : row.costUsd);
	const sorted = [...analytics.identities].sort((left, right) => value(right) - value(left));
	const top = sorted.slice(0, TOP_IDENTITIES);
	const rest = sorted.slice(TOP_IDENTITIES);
	const total = metric === "tokens" ? analytics.totals.totalTokens : analytics.totals.costUsd;
	const max = Math.max(0, ...top.map(value));
	const format = metric === "tokens" ? formatTokens : formatCost;
	const restTotals = rest.reduce(
		(sum, row) => ({
			totalTokens: sum.totalTokens + row.totalTokens,
			costUsd: sum.costUsd + row.costUsd,
			requests: sum.requests + row.requests,
			inputTokens: sum.inputTokens + row.inputTokens,
			cachedInputTokens: sum.cachedInputTokens + row.cachedInputTokens,
			outputTokens: sum.outputTokens + row.outputTokens,
			reasoningOutputTokens: sum.reasoningOutputTokens + row.reasoningOutputTokens,
		}),
		{ totalTokens: 0, costUsd: 0, requests: 0, inputTokens: 0, cachedInputTokens: 0, outputTokens: 0, reasoningOutputTokens: 0 },
	);
	return (
		<SectionCard
			actions={
				<Segmented
					label="指标"
					onChange={setMetric}
					options={[
						{ value: "tokens", label: "Token" },
						{ value: "cost", label: "成本" },
					]}
					value={metric}
				/>
			}
			contentClassName="gap-2"
			description={metric === "tokens" ? "按 Token 排序" : "按成本排序"}
			title="下游调用量"
		>
			{metric === "tokens" ? (
				<Legend>
					<LegendItem color={TOKEN_PARTS.input.color} label={TOKEN_PARTS.input.label} />
					<LegendItem color={TOKEN_PARTS.cached.color} label={TOKEN_PARTS.cached.label} />
					<LegendItem color={TOKEN_PARTS.output.color} label={TOKEN_PARTS.output.label} />
				</Legend>
			) : null}
			{top.length === 0 ? <NoData /> : null}
			<div className="flex flex-col">
				{top.map((row, index) => (
					<RankRow
						bar={
							metric === "tokens" ? (
								<StackedBar height={6} segments={tokenSegments(row)} total={max} />
							) : (
								<StackedBar height={6} segments={[{ key: "cost", value: row.costUsd, color: VIZ.teal }]} total={max} />
							)
						}
						detail={
							metric === "tokens"
								? `${formatCost(row.costUsd)} · ${formatCount(row.requests)} 请求 · 成功 ${formatPercent(successRate(row))} · 缓存 ${formatPercent(cacheRate(row))}`
								: `${formatTokens(row.totalTokens)} Token · ${formatCount(row.requests)} 请求 · 每请求 ${formatCost(row.requests ? row.costUsd / row.requests : 0)}`
						}
						key={row.userId}
						name={row.userName || row.userId}
						rank={index + 1}
						share={total > 0 ? formatPercent(value(row) / total) : undefined}
						value={format(value(row))}
						{...(onSelect ? { onClick: () => onSelect(row) } : {})}
					/>
				))}
				{rest.length ? (
					<RankRow
						bar={<StackedBar height={6} segments={metric === "tokens" ? tokenSegments(restTotals) : [{ key: "cost", value: restTotals.costUsd, color: VIZ.teal }]} total={max} />}
						detail={`${formatCost(restTotals.costUsd)} · ${formatCount(restTotals.requests)} 请求`}
						name={`其他 ${rest.length} 个用户`}
						rank="·"
						share={total > 0 ? formatPercent(value(restTotals) / total) : undefined}
						value={format(value(restTotals))}
					/>
				) : null}
			</div>
		</SectionCard>
	);
}

type UpstreamMetric = "tokens" | "requests" | "failed";

export interface UpstreamMeta {
	/** Strategy label by group id. */
	groupStrategy?: (groupId: string) => string | null;
	/** Member count by group id. */
	groupSize?: (groupId: string) => number | null;
	/** Plan label by Codex account id. */
	accountPlan?: (accountId: string) => string | null;
}

/** Figma "card · 上游路由": groups with their member accounts, plus direct accounts. */
export function UpstreamCard({ analytics, meta = {}, onSelectAccount }: { analytics: UsageAnalytics; meta?: UpstreamMeta; onSelectAccount?: (account: UpstreamAccountStats) => void }) {
	const [metric, setMetric] = useState<UpstreamMetric>("tokens");
	const value = (row: { totalTokens: number; requests: number; failed: number; interrupted: number }) =>
		metric === "tokens" ? row.totalTokens : metric === "requests" ? row.requests : row.failed + row.interrupted;
	const format = metric === "tokens" ? formatTokens : formatCount;
	const accounts = analytics.upstreamAccounts;
	const groups = [...analytics.upstreamGroups].sort((left, right) => value(right) - value(left));
	const total = groups.reduce((sum, row) => sum + value(row), 0);
	const max = Math.max(0, ...groups.map(value), ...accounts.map(value));
	const bar = (row: Parameters<typeof tokenSplit>[0] & { requests: number; failed: number; interrupted: number; completed: number; incomplete: number }) =>
		metric === "tokens" ? (
			<StackedBar height={6} segments={tokenSegments(row)} total={max} />
		) : metric === "requests" ? (
			<StackedBar
				height={6}
				segments={[
					{ key: "ok", value: row.completed + row.incomplete, color: VIZ.blue },
					{ key: "bad", value: row.requests - row.completed - row.incomplete, color: STATUS_META.failed.color },
				]}
				total={max}
			/>
		) : (
			<StackedBar
				height={6}
				segments={[
					{ key: "failed", value: row.failed, color: STATUS_META.failed.color },
					{ key: "interrupted", value: row.interrupted, color: STATUS_META.interrupted.color },
				]}
				total={max}
			/>
		);
	type Member = UpstreamGroupStats["accounts"][number];
	/** Tokens and requests are this group's share; failures are the account's overall count. */
	const memberValue = (entry: Member, overall: UpstreamAccountStats | undefined) =>
		metric === "tokens" ? entry.totalTokens : metric === "requests" ? entry.requests : overall ? overall.failed + overall.interrupted : 0;
	const memberRow = (entry: Member, overall: UpstreamAccountStats | undefined, indent: boolean, groupId: string) => {
		const plan = meta.accountPlan?.(entry.codexAccountId);
		return (
			<RankRow
				bar={
					metric === "failed" && overall ? (
						bar(overall)
					) : (
						<StackedBar height={6} segments={[{ key: "value", value: memberValue(entry, overall), color: metric === "tokens" ? VIZ.blueSoft : VIZ.blue }]} total={max} />
					)
				}
				detail={[
					`${formatCount(entry.requests)} 请求`,
					overall ? `账户合计 ${formatCost(overall.costUsd)} · 成功 ${formatPercent(successRate(overall))}` : null,
				]
					.filter(Boolean)
					.join(" · ")}
				indent={indent}
				key={`${groupId}:${entry.codexAccountId}`}
				name={
					<span className="flex items-center gap-1.5">
						<UserRoundIcon className="size-3.5 shrink-0 text-muted-foreground" />
						<span className="truncate">{entry.codexAccountName || entry.codexAccountId}</span>
					</span>
				}
				share={total > 0 ? formatPercent(memberValue(entry, overall) / total) : undefined}
				tag={
					<span className="flex items-center gap-1">
						{plan ? <Tag>{indent ? plan : `单账户 · ${plan}`}</Tag> : null}
						{entry.rateLimited ? <Tag tone="danger">429 × {formatCount(entry.rateLimited)}</Tag> : null}
					</span>
				}
				value={format(memberValue(entry, overall))}
				{...(onSelectAccount && overall ? { onClick: () => onSelectAccount(overall) } : {})}
			/>
		);
	};
	return (
		<SectionCard
			actions={
				<Segmented
					label="指标"
					onChange={setMetric}
					options={[
						{ value: "tokens", label: "Token" },
						{ value: "requests", label: "请求" },
						{ value: "failed", label: "失败" },
					]}
					value={metric}
				/>
			}
			contentClassName="gap-0"
			description="账户组与 Codex 账户承担的用量及 429 次数"
			title="上游路由"
		>
			{groups.length === 0 ? <NoData /> : null}
			{groups.map((group) => {
				const members = [...group.accounts]
					.map((entry) => ({ entry, overall: accounts.find((row) => row.codexAccountId === entry.codexAccountId) }))
					.sort((left, right) => memberValue(right.entry, right.overall) - memberValue(left.entry, left.overall));
				if (!group.accountGroupId) return members.map(({ entry, overall }) => memberRow(entry, overall, false, ""));
				const size = meta.groupSize?.(group.accountGroupId);
				const strategy = meta.groupStrategy?.(group.accountGroupId);
				return (
					<div key={group.accountGroupId}>
						<RankRow
							bar={bar(group)}
							detail={[
								formatCost(group.costUsd),
								`${formatCount(group.requests)} 请求`,
								`成功 ${formatPercent(successRate(group))}`,
								strategy,
							]
								.filter(Boolean)
								.join(" · ")}
							name={
								<span className="flex items-center gap-1.5">
									<Layers3Icon className="size-3.5 shrink-0 text-muted-foreground" />
									<span className="truncate">{group.accountGroupName || "账户组"}</span>
								</span>
							}
							share={total > 0 ? formatPercent(value(group) / total) : undefined}
							tag={<Tag>账户组{size ? ` · ${size} 个账户` : ""}</Tag>}
							value={format(value(group))}
						/>
						{members.map(({ entry, overall }) => memberRow(entry, overall, true, group.accountGroupId))}
					</div>
				);
			})}
		</SectionCard>
	);
}
