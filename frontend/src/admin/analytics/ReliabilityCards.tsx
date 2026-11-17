import { InfoIcon, Layers3Icon, ScrollTextIcon, TriangleAlertIcon, UserRoundIcon } from "lucide-react";
import { useState } from "react";

import type { FastBreakdown, ModelStats, QuotaOverview, UpstreamAccountStats, UsageAnalytics } from "@/admin-api";
import { Legend, LegendItem, Meter, RangeBar, StackedBar } from "@/components/app/bars";
import { Tag } from "@/components/app/badges";
import { SectionCard } from "@/components/app/section-card";
import { Segmented } from "@/components/app/segmented";
import { DeltaBadge } from "@/components/app/stat-card";
import { VIZ, tierLabel } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { formatCount, formatDay, formatPercent, formatSeconds, formatUntil, pointDelta } from "@/lib/format";
import { cn } from "@/lib/utils";

import type { LatencyTargets } from "./PerformanceCards";
import { NoData, SectionLabel, SignedDelta } from "./common";
import { bucketTick, modelColors, successRate } from "./metrics";

type ModelSort = "samples" | "ttft" | "failure";

function failureRate(model: ModelStats): number | null {
	return model.requests ? (model.failed + model.interrupted + model.incomplete) / model.requests : null;
}

/** Figma "card · 模型性能": P50–P95 ranges per model on a shared log scale. */
export function ModelPerformanceCard({ analytics, targets }: { analytics: UsageAnalytics; targets: LatencyTargets }) {
	const [sort, setSort] = useState<ModelSort>("samples");
	const colors = modelColors(analytics);
	const models = [...analytics.models].sort((left, right) =>
		sort === "ttft" ? (right.ttft.p95 ?? 0) - (left.ttft.p95 ?? 0) : sort === "failure" ? (failureRate(right) ?? 0) - (failureRate(left) ?? 0) : right.ttft.samples - left.ttft.samples,
	);
	const ttftMax = Math.max(targets.ttftP95Ms * 1.2, ...models.map((model) => model.ttft.p95 ?? 0));
	const latencyMax = Math.max(targets.latencyP95Ms * 1.2, ...models.map((model) => model.latency.p95 ?? 0));
	const logRatio = (value: number, max: number) => Math.log1p(value) / Math.log1p(max);
	const slow = models.filter((model) => (model.ttft.p95 ?? 0) > targets.ttftP95Ms && (model.latency.p95 ?? 0) > targets.latencyP95Ms);
	return (
		<SectionCard
			actions={
				<Segmented
					label="排序"
					onChange={setSort}
					options={[
						{ value: "samples", label: "按样本" },
						{ value: "ttft", label: "按 P95 首字" },
						{ value: "failure", label: "按失败率" },
					]}
					value={sort}
				/>
			}
			contentClassName="gap-2"
			description="条形为 P50–P95（对数刻度），红线为 P95 目标"
			title="模型性能"
		>
			{models.length === 0 ? (
				<NoData />
			) : (
				<div className="overflow-x-auto">
					<Table className="min-w-[900px] text-[13px]">
						<TableHeader>
							<TableRow className="hover:bg-transparent [&>th]:h-8 [&>th]:text-[11.5px] [&>th]:font-medium [&>th]:text-faint">
								<TableHead className="w-52">模型</TableHead>
								<TableHead className="text-right">样本</TableHead>
								<TableHead className="w-48">首字延迟 P50 / P95</TableHead>
								<TableHead className="w-48">总延迟 P50 / P95</TableHead>
								<TableHead>输出速度 前半 → 后半 t/s</TableHead>
								<TableHead className="text-right">失败率</TableHead>
								<TableHead className="text-right">取消率</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{models.map((model) => {
								const color = colors.get(model.model) ?? VIZ.gray;
								const failure = failureRate(model);
								const cancel = model.requests ? model.cancelled / model.requests : null;
								const ttftOver = (model.ttft.p95 ?? 0) > targets.ttftP95Ms;
								const latencyOver = (model.latency.p95 ?? 0) > targets.latencyP95Ms;
								const first = model.outputSpeedFirstHalf;
								const second = model.outputSpeedSecondHalf;
								return (
									<TableRow className="hover:bg-transparent [&>td]:py-3" key={model.model}>
										<TableCell>
											<span className="flex items-center gap-2 font-mono font-bold">
												<span aria-hidden="true" className="size-2 rounded-[2px]" style={{ background: color }} />
												{model.model}
											</span>
										</TableCell>
										<TableCell className="text-right">
											<div className="flex flex-col items-end">
												<span className="font-medium tabular-nums">{formatCount(model.ttft.samples)}</span>
												<span className="text-[11px] text-faint tabular-nums">/ {formatCount(model.requests)} 请求</span>
											</div>
										</TableCell>
										{([
											["ttft", model.ttft, ttftMax, targets.ttftP95Ms, ttftOver],
											["latency", model.latency, latencyMax, targets.latencyP95Ms, latencyOver],
										] as const).map(([key, value, max, target, over]) => (
											<TableCell key={key}>
												<div className="flex flex-col gap-1">
													<span className="text-xs tabular-nums">
														<span className="font-semibold">{formatSeconds(value.p50, 1)}</span>
														<span className="text-faint"> / </span>
														<span className={cn(over ? "font-semibold text-danger" : "text-muted-foreground")}>{formatSeconds(value.p95, 1)}</span>
													</span>
													<RangeBar
														color={color}
														from={value.p50 === null ? null : logRatio(value.p50, max)}
														marker={value.p50 === null ? null : logRatio(value.p50, max)}
														max={1}
														targetRatio={logRatio(target, max)}
														to={value.p95 === null ? null : logRatio(value.p95, max)}
													/>
												</div>
											</TableCell>
										))}
										<TableCell>
											<span className="flex items-baseline gap-1.5 text-xs tabular-nums">
												<span className="text-muted-foreground">{first?.toFixed(1) ?? "—"}</span>
												<span className="text-faint">→</span>
												<span className="font-semibold">{second?.toFixed(1) ?? "—"}</span>
												<SignedDelta value={first !== null && second !== null ? second - first : null} />
											</span>
										</TableCell>
										<TableCell className={cn("text-right tabular-nums", (failure ?? 0) > 0.03 && "font-semibold text-danger")}>{formatPercent(failure)}</TableCell>
										<TableCell className={cn("text-right tabular-nums", (cancel ?? 0) > 0.01 && "text-warning")}>{formatPercent(cancel)}</TableCell>
									</TableRow>
								);
							})}
						</TableBody>
					</Table>
				</div>
			)}
			<div className="flex flex-wrap items-center gap-x-4 gap-y-1 pt-2 text-[11.5px] text-muted-foreground">
				<LegendItem color={VIZ.blue} label="P50 – P95" shape="line" />
				<LegendItem color={VIZ.danger} label={`P95 目标（首字 ${formatSeconds(targets.ttftP95Ms, 0)} · 总延迟 ${formatSeconds(targets.latencyP95Ms, 0)}）`} shape="line" />
				<span>样本 = 有首字数据的请求；失败率含失败、上游中断、未完成</span>
				{slow.length ? (
					<span className="flex items-center gap-1 text-warning">
						<TriangleAlertIcon className="size-3.5" />
						{slow.map((model) => model.model).join("、")} 的 P95 首字与总延迟均超出目标
					</span>
				) : null}
			</div>
		</SectionCard>
	);
}

function fiveHourWindow(quota: QuotaOverview | null, accountId: string) {
	return quota?.accounts.find((entry) => entry.accountId === accountId)?.windows.find((window) => window.category === "codex" && (window.kind === "five_hour" || window.kind === "primary")) ?? null;
}

/** Figma "card · 上游账户可靠性": success, 429 distribution and quota per account. */
export function UpstreamReliabilityCard({
	analytics,
	now,
	quota,
	accounts,
	groups,
	onOpenLogs,
	onOpenGroup,
}: {
	analytics: UsageAnalytics;
	now: number;
	quota: QuotaOverview | null;
	/** All Codex accounts (shows idle ones too). */
	accounts: { id: string; name: string; enabled: boolean }[];
	groups: { id: string; name: string; accountIds: string[] }[];
	onOpenLogs: (accountId: string) => void;
	onOpenGroup: (groupId: string) => void;
}) {
	const [scope, setScope] = useState("all");
	const stats = new Map(analytics.upstreamAccounts.map((row) => [row.codexAccountId, row]));
	const visible = accounts.filter((account) => scope === "all" || groups.find((group) => group.id === scope)?.accountIds.includes(account.id));
	const ordered = [...visible].sort((left, right) => (stats.get(right.id)?.requests ?? -1) - (stats.get(left.id)?.requests ?? -1));
	const total429 = analytics.upstreamAccounts.reduce((sum, row) => sum + row.rateLimited, 0);
	const max429 = Math.max(1, ...analytics.upstreamAccounts.flatMap((row) => row.rateLimitSeries));
	const worst = analytics.upstreamAccounts.reduce<UpstreamAccountStats | null>((best, row) => (row.rateLimited > (best?.rateLimited ?? 0) ? row : best), null);
	const worstBucket = worst ? worst.rateLimitSeries.reduce((best, value, index) => (value > (worst.rateLimitSeries[best] ?? 0) ? index : best), 0) : 0;
	const worstGroup = worst ? groups.find((group) => group.id === worst.accountGroupId || group.accountIds.includes(worst.codexAccountId)) : undefined;
	const bucketMs = analytics.bucketMs;
	const span = analytics.endAt - analytics.startAt;
	return (
		<SectionCard
			actions={
				<>
					{total429 ? <Tag tone="danger">429 {formatCount(total429)} 次</Tag> : null}
					{groups.length ? (
						<Segmented
							label="范围"
							onChange={setScope}
							options={[{ value: "all", label: "全部账户" }, ...groups.map((group) => ({ value: group.id, label: group.name }))]}
							value={scope}
						/>
					) : null}
				</>
			}
			contentClassName="gap-2"
			description={`条形为每 ${bucketMs >= 86_400_000 ? "天" : `${bucketMs / 3_600_000} 小时`}的 429 次数`}
			title="上游账户可靠性"
		>
			{ordered.length === 0 ? (
				<NoData />
			) : (
				<div className="overflow-x-auto">
					<Table className="min-w-[980px] text-[13px]">
						<TableHeader>
							<TableRow className="hover:bg-transparent [&>th]:h-8 [&>th]:text-[11.5px] [&>th]:font-medium [&>th]:text-faint">
								<TableHead className="w-52">账户</TableHead>
								<TableHead className="text-right">请求</TableHead>
								<TableHead className="w-24">成功率</TableHead>
								<TableHead className="text-right">429</TableHead>
								<TableHead className="w-48">429 分布</TableHead>
								<TableHead className="text-right">流中断</TableHead>
								<TableHead className="text-right">P95 首字</TableHead>
								<TableHead className="text-right">P95 总延迟</TableHead>
								<TableHead className="w-32">5h 额度</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{ordered.map((account) => {
								const row = stats.get(account.id);
								const window = fiveHourWindow(quota, account.id);
								const plan = quota?.accounts.find((entry) => entry.accountId === account.id)?.planType;
								const group = groups.find((entry) => entry.accountIds.includes(account.id));
								const risky = row && row.rateLimited > 0 && row === worst;
								const success = row ? successRate(row) : null;
								const used = window?.usedPercent ?? null;
								return (
									<TableRow className={cn("hover:bg-transparent [&>td]:py-3", risky && "bg-warning-soft/60", !row && "text-faint")} key={account.id}>
										<TableCell>
											<div className="flex flex-col">
												<span className="flex items-center gap-1.5 font-medium">
													<UserRoundIcon className="size-3.5 text-muted-foreground" />
													<span className="truncate">{account.name}</span>
												</span>
												<span className="pl-5 text-[11px] text-faint">{[group?.name, plan, account.enabled ? null : "已停用"].filter(Boolean).join(" · ") || "—"}</span>
											</div>
										</TableCell>
										{row ? (
											<>
												<TableCell className="text-right tabular-nums">{formatCount(row.requests)}</TableCell>
												<TableCell>
													<div className="flex flex-col gap-1">
														<span className={cn("font-semibold tabular-nums", (success ?? 1) < 0.95 ? "text-danger" : "text-success")}>{formatPercent(success)}</span>
														<Meter color={(success ?? 1) < 0.95 ? VIZ.danger : VIZ.success} height={4} ratio={success} />
													</div>
												</TableCell>
												<TableCell className={cn("text-right tabular-nums", row.rateLimited ? "font-semibold text-danger" : "text-faint")}>{formatCount(row.rateLimited)}</TableCell>
												<TableCell>
													{row.rateLimited ? (
														<div className="flex h-6 items-end gap-px rounded bg-surface-subtle px-0.5">
															{row.rateLimitSeries.map((value, index) => (
																<span className="min-w-px flex-1 rounded-t-[1px] bg-danger" key={index} style={{ height: value ? `${Math.max(12, (value / max429) * 100)}%` : 0 }} title={`${bucketTick(analytics.startAt + index * bucketMs, bucketMs, span)}：${value}`} />
															))}
														</div>
													) : (
														<span className="flex h-6 items-center rounded bg-surface-subtle px-2 text-[11px] text-faint">无 429</span>
													)}
												</TableCell>
												<TableCell className={cn("text-right tabular-nums", row.interrupted > 3 && "text-warning")}>{formatCount(row.interrupted)}</TableCell>
												<TableCell className="text-right tabular-nums">{formatSeconds(row.ttft.p95, 1)}</TableCell>
												<TableCell className="text-right tabular-nums">{formatSeconds(row.latency.p95, 1)}</TableCell>
											</>
										) : (
											<TableCell className="text-xs" colSpan={7}>
												本周期无请求
											</TableCell>
										)}
										<TableCell>
											{window && used !== null ? (
												<div className="flex flex-col gap-1">
													<span className={cn("text-xs font-semibold tabular-nums", used >= 70 && "text-warning")}>{used.toFixed(0)}%</span>
													<Meter color={used >= 90 ? VIZ.danger : used >= 70 ? VIZ.amber : VIZ.gray} height={4} ratio={used / 100} />
													<span className={cn("text-[11px] text-faint", window.exhaustsBeforeReset && "text-warning")}>
														{window.exhaustsBeforeReset && window.exhaustAt ? `约 ${formatUntil(window.exhaustAt, now)}耗尽` : window.resetAt ? `${formatUntil(window.resetAt, now)}重置` : "—"}
													</span>
												</div>
											) : (
												<span className="text-xs text-faint">—</span>
											)}
										</TableCell>
									</TableRow>
								);
							})}
						</TableBody>
					</Table>
				</div>
			)}
			{worst && worst.rateLimited > 0 ? (
				<div className="flex flex-wrap items-center gap-3 rounded-lg border border-warning-border bg-warning-soft px-4 py-3">
					<TriangleAlertIcon className="size-4 shrink-0 text-warning" />
					<div className="flex min-w-0 flex-1 flex-col">
						<span className="text-[13px] font-semibold">{worst.codexAccountName} 限流风险</span>
						<span className="text-xs text-muted-foreground">
							{formatCount(worst.rateLimited)} 次 429 · 最集中于 {bucketTick(analytics.startAt + worstBucket * bucketMs, bucketMs, span)}（{formatCount(worst.rateLimitSeries[worstBucket] ?? 0)} 次）
							{(() => {
								const window = fiveHourWindow(quota, worst.codexAccountId);
								return window?.usedPercent != null ? ` · 5 小时额度已用 ${window.usedPercent.toFixed(0)}%` : "";
							})()}
						</span>
					</div>
					<Button onClick={() => onOpenLogs(worst.codexAccountId)} size="sm" type="button" variant="outline">
						<ScrollTextIcon data-icon="inline-start" />
						查看 429 日志
					</Button>
					{worstGroup ? (
						<Button onClick={() => onOpenGroup(worstGroup.id)} size="sm" type="button">
							<Layers3Icon data-icon="inline-start" />
							调整{worstGroup.name}
						</Button>
					) : null}
				</div>
			) : null}
		</SectionCard>
	);
}

function TierRows({ rows, highlight }: { rows: FastBreakdown[]; highlight: string | null }) {
	return (
		<div className="flex flex-col gap-1">
			{rows.map((row) => {
				const rate = row.requested ? row.fulfilled / row.requested : null;
				return (
					<div
						className={cn("grid grid-cols-[minmax(0,8.5rem)_1fr_4.5rem_3.25rem_3.75rem] items-center gap-2 rounded-md px-1 py-1 text-xs", row.name === highlight && "bg-warning-soft")}
						key={row.name}
					>
						<span className="truncate">{row.name}</span>
						<StackedBar
							height={6}
							segments={[
								{ key: "ok", value: row.fulfilled, color: VIZ.success },
								{ key: "down", value: row.downgraded, color: VIZ.amber },
							]}
							total={row.requested}
						/>
						<span className="text-right text-faint tabular-nums">
							{formatCount(row.fulfilled)} / {formatCount(row.requested)}
						</span>
						<span className={cn("text-right font-semibold tabular-nums", (rate ?? 1) < 0.9 && "text-warning")}>{formatPercent(rate)}</span>
						<span className={cn("text-right tabular-nums", row.downgraded ? "text-warning" : "text-faint")}>{row.downgraded ? `${row.downgraded} 降级` : "—"}</span>
					</div>
				);
			})}
		</div>
	);
}

/** Figma "card · 速度档位兑现": Fast requested vs served. */
export function FastFulfilmentCard({ analytics }: { analytics: UsageAnalytics }) {
	const [view, setView] = useState<"fast" | "all">("fast");
	const fast = analytics.fast;
	const rate = fast.requested ? fast.fulfilled / fast.requested : null;
	const before = analytics.previous;
	const previousRate = before && before.fastRequests ? before.fastFulfilled / before.fastRequests : null;
	const worstAccount = fast.accounts.reduce<FastBreakdown | null>((best, row) => (row.downgraded > (best?.downgraded ?? 0) ? row : best), null);
	const maxDowngrade = Math.max(1, ...analytics.series.map((bucket) => bucket.fastDowngraded));
	const span = analytics.endAt - analytics.startAt;
	const tiers = new Map<string, number>();
	for (const cell of analytics.effortTiers) tiers.set(cell.tier, (tiers.get(cell.tier) ?? 0) + cell.requests);
	const tierTotal = [...tiers.values()].reduce((sum, value) => sum + value, 0);
	return (
		<SectionCard
			actions={
				<Segmented
					label="视图"
					onChange={setView}
					options={[
						{ value: "fast", label: "Fast" },
						{ value: "all", label: "全部档位" },
					]}
					value={view}
				/>
			}
			description="请求档位与实际档位对比"
			title="速度档位兑现"
		>
			{view === "fast" ? (
				<>
					<div className="flex flex-wrap items-end gap-6">
						<div className="flex flex-col">
							<span className="flex items-center gap-2">
								<span className="text-[30px] leading-10 font-bold tracking-[-0.035em] tabular-nums">{formatPercent(rate)}</span>
								<DeltaBadge delta={pointDelta(rate, previousRate)} />
							</span>
							<span className="text-xs text-muted-foreground">Fast 兑现率 · 较上一周期</span>
						</div>
						<div className="flex gap-5">
							<span className="flex flex-col">
								<span className="text-xl leading-7 font-bold tabular-nums">{formatCount(fast.requested)}</span>
								<span className="text-xs text-muted-foreground">请求 Fast</span>
							</span>
							<span className="flex flex-col">
								<span className="text-xl leading-7 font-bold text-success tabular-nums">{formatCount(fast.fulfilled)}</span>
								<span className="text-xs text-muted-foreground">实际 Fast</span>
							</span>
							<span className="flex flex-col">
								<span className="text-xl leading-7 font-bold text-warning tabular-nums">{formatCount(fast.downgraded)}</span>
								<span className="text-xs text-muted-foreground">降为标准</span>
							</span>
						</div>
					</div>
					<StackedBar
						height={8}
						segments={[
							{ key: "ok", value: fast.fulfilled, color: VIZ.success },
							{ key: "down", value: fast.downgraded, color: VIZ.amber },
						]}
						total={fast.requested}
					/>
					{fast.requested === 0 ? (
						<NoData title="此范围内没有 Fast 请求" />
					) : (
						<>
							<div className="flex flex-col gap-2 border-t border-border/60 pt-4">
								<SectionLabel aside="实际 / 请求">按上游账户</SectionLabel>
								<TierRows highlight={worstAccount?.downgraded ? worstAccount.name : null} rows={fast.accounts} />
							</div>
							<div className="flex flex-col gap-2 border-t border-border/60 pt-4">
								<SectionLabel aside="实际 / 请求">按模型</SectionLabel>
								<TierRows highlight={null} rows={fast.models} />
							</div>
							<div className="flex flex-col gap-2 border-t border-border/60 pt-4">
								<SectionLabel aside={`共 ${formatCount(fast.downgraded)} 次`}>降级时间分布</SectionLabel>
								<div className="flex h-10 items-end gap-px rounded bg-surface-subtle px-0.5">
									{analytics.series.map((bucket) => (
										<span
											className="min-w-px flex-1 rounded-t-[1px]"
											key={bucket.startAt}
											style={{ height: bucket.fastDowngraded ? `${Math.max(10, (bucket.fastDowngraded / maxDowngrade) * 100)}%` : 0, background: VIZ.amber }}
											title={`${bucketTick(bucket.startAt, analytics.bucketMs, span)}：${bucket.fastDowngraded}`}
										/>
									))}
								</div>
								<div className="flex justify-between text-[10.5px] text-faint">
									<span>{formatDay(analytics.startAt)}</span>
									<span>{formatDay(analytics.totals.lastAt ?? analytics.endAt)}</span>
								</div>
							</div>
							{worstAccount && worstAccount.downgraded ? (
								<span className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
									<InfoIcon className="size-3.5" />
									{worstAccount.name} 降级最多（{worstAccount.downgraded} 次）
								</span>
							) : null}
						</>
					)}
				</>
			) : (
				<div className="flex flex-col gap-3">
					<StackedBar
						height={10}
						segments={[...tiers.entries()].map(([tier, value], index) => ({ key: tier || "none", value, color: [VIZ.blue, VIZ.violet, VIZ.teal, VIZ.gray][index % 4] ?? VIZ.gray, label: tierLabel(tier) }))}
					/>
					<Legend className="flex-col items-start gap-2">
						{[...tiers.entries()].map(([tier, value], index) => (
							<LegendItem
								color={[VIZ.blue, VIZ.violet, VIZ.teal, VIZ.gray][index % 4] ?? VIZ.gray}
								key={tier || "none"}
								label={`实际 ${tierLabel(tier)}`}
								value={`${formatCount(value)} · ${formatPercent(tierTotal ? value / tierTotal : 0)}`}
							/>
						))}
					</Legend>
				</div>
			)}
		</SectionCard>
	);
}
