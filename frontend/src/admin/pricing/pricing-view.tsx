import { useMemo, useState, type FormEvent } from "react";
import {
	CircleDollarSignIcon,
	CopyIcon,
	ExternalLinkIcon,
	Globe2Icon,
	Link2Icon,
	PencilIcon,
	PlusIcon,
	RefreshCwIcon,
	SaveIcon,
	TriangleAlertIcon,
} from "lucide-react";

import type { ModelPrice, ModelStats, UsageAnalytics } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { useAnalytics } from "@/admin/hooks";
import { StatusDot, Tag } from "@/components/app/badges";
import { Meter, StackedBar } from "@/components/app/bars";
import { SectionCard } from "@/components/app/section-card";
import { VIZ, seriesColor } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { formatAgo, formatCost, formatCount, formatDateTime, formatPercent, formatTokens, ratio } from "@/lib/format";
import { cn } from "@/lib/utils";

import { useLastSeen } from "@/admin/identities/use-last-seen";

import { useSavePrices } from "./use-save-prices";
import {
	PRICE_FIELDS,
	PRICE_FIELD_LABELS,
	estimateCost,
	parseValue,
	priceChanges,
	sameModel,
	samePrice,
	setDraft,
	toPrice,
	toRow,
	usePriceDraft,
	validRow,
	type DraftRow,
	type PriceChange,
	type PriceField,
} from "./store";

const DEFAULT_SOURCE_URL = "https://models.dev/api.json";

export function PricingView() {
	const { pricing } = useAdmin();
	const analytics = useAnalytics({ range: "7d" });
	const draft = usePriceDraft();
	const rows = useMemo(() => draft ?? pricing.prices.map(toRow), [draft, pricing.prices]);
	const [focusModel, setFocusModel] = useState<string | null>(null);

	function addModel(model: string, base?: ModelPrice) {
		const name = model.trim();
		if (!name || rows.some((row) => sameModel(row.model, name))) return false;
		const row = base ? { ...toRow(base), model: name } : { model: name, input: "", output: "", cacheRead: "0", cacheWrite: "0", multiplier: "1" };
		setDraft([row, ...rows]);
		setFocusModel(name);
		return true;
	}

	return (
		<div className="flex flex-col gap-4">
			<SourceStrip analytics={analytics.data} />
			{analytics.data ? <UnpricedAlert analytics={analytics.data} onAdd={addModel} rows={rows} /> : null}
			<PriceTable analytics={analytics.data} focusModel={focusModel} loading={!analytics.data && analytics.loading} onAdd={addModel} rows={rows} />
		</div>
	);
}

function StripCell({ icon, label, children, className }: { icon: React.ReactNode; label: React.ReactNode; children: React.ReactNode; className?: string }) {
	return (
		<div className={cn("flex min-w-0 flex-col gap-2 px-6 py-4", className)}>
			<span className="flex items-center gap-1.5 text-[11.5px] leading-4 font-medium text-faint [&_svg]:size-3.5">
				{icon}
				{label}
			</span>
			{children}
		</div>
	);
}

function SourceStrip({ analytics }: { analytics: UsageAnalytics | null }) {
	const { pricing, now } = useAdmin();
	const sync = pricing.sync;
	const sourceUrl = sync?.sourceUrl ?? DEFAULT_SOURCE_URL;
	const totals = analytics?.totals;
	const pricedTokens = totals ? totals.totalTokens - totals.unpricedTokens : 0;
	return (
		<Card className="grid gap-0 divide-y py-0 sm:grid-cols-2 sm:divide-y-0 xl:grid-cols-4 xl:divide-x">
			<StripCell icon={<Globe2Icon />} label="数据源">
				<a className="flex items-center gap-1.5 text-lg leading-6 font-bold tracking-[-0.02em] hover:underline" href={sourceUrl} rel="noreferrer" target="_blank">
					{sync?.source ?? "Models.dev"}
					<ExternalLinkIcon className="size-3.5 text-muted-foreground" />
				</a>
				<span className="truncate text-[11.5px] leading-4 text-muted-foreground">{sourceUrl.replace(/^https?:\/\//, "")}</span>
			</StripCell>
			<StripCell className="sm:border-l xl:border-l-0" icon={<RefreshCwIcon />} label="上次同步">
				<div className="flex items-baseline gap-2">
					<span className="text-lg leading-6 font-bold tracking-[-0.02em] tabular-nums">{sync ? formatDateTime(sync.syncedAt) : "从未同步"}</span>
					{sync ? <span className="text-[11.5px] text-faint">{formatAgo(sync.syncedAt, now)}</span> : null}
				</div>
				<span className="text-[11.5px] leading-4 text-muted-foreground">
					{sync ? `更新 ${sync.updatedModels.length} · 跳过手动价格 ${sync.skippedModels.length}` : "点击「从 Models.dev 同步」获取价格"}
				</span>
			</StripCell>
			<StripCell icon={<Link2Icon />} label="Models.dev 匹配">
				<div className="flex items-baseline gap-3">
					<span className="flex items-baseline gap-1">
						<span className="text-lg leading-6 font-bold tabular-nums">{sync?.matchedModels.length ?? "—"}</span>
						<span className="text-[11.5px] font-medium text-success">已匹配</span>
					</span>
					<span className="flex items-baseline gap-1">
						<span className="text-lg leading-6 font-bold tabular-nums">{sync?.unmatchedModels.length ?? "—"}</span>
						<span className="text-[11.5px] font-medium text-warning">未匹配</span>
					</span>
				</div>
				<span className="truncate text-[11.5px] leading-4 text-muted-foreground">
					{sync?.unmatchedModels.length ? (
						<>
							未匹配：<span className="font-mono text-[11px]">{sync.unmatchedModels.join("、")}</span>
						</>
					) : sync ? (
						"全部已用模型均已匹配"
					) : (
						"—"
					)}
				</span>
			</StripCell>
			<StripCell className="sm:border-l xl:border-l-0" icon={<CircleDollarSignIcon />} label="7 天成本覆盖">
				{totals ? (
					<>
						<div className="flex items-baseline gap-2">
							<span className="text-lg leading-6 font-bold tracking-[-0.02em] tabular-nums">{formatPercent(ratio(pricedTokens, totals.totalTokens) ?? 1)}</span>
							<span className="text-[11.5px] text-faint">Token 已计价</span>
						</div>
						<StackedBar
							height={6}
							segments={[
								{ key: "priced", value: pricedTokens, color: VIZ.teal, label: "已计价" },
								{ key: "unpriced", value: totals.unpricedTokens, color: VIZ.pink, label: "未计价" },
							]}
						/>
						<span className="text-[11.5px] leading-4 text-muted-foreground">
							{totals.unpricedTokens ? `未计价 ${formatTokens(totals.unpricedTokens)} Token · ${formatCount(totals.unpricedRequests)} 请求` : "全部 Token 已计价"}
						</span>
					</>
				) : (
					<Skeleton className="h-14 w-full" />
				)}
			</StripCell>
		</Card>
	);
}

function UnpricedAlert({ analytics, rows, onAdd }: { analytics: UsageAnalytics; rows: DraftRow[]; onAdd: (model: string, base?: ModelPrice) => boolean }) {
	const { pricing } = useAdmin();
	const pending = analytics.unpricedModels.filter((model) => !rows.some((row) => sameModel(row.model, model.model)));
	if (pending.length === 0) return null;
	const reference = analytics.models
		.filter((model) => model.priced)
		.map((model) => ({ model, price: pricing.prices.find((price) => sameModel(price.model, model.model)) }))
		.find((item) => item.price)?.price;
	return (
		<div className="flex gap-4 rounded-xl border border-warning-border bg-warning-soft px-6 py-4">
			<div className="h-fit shrink-0 rounded-lg bg-card p-2">
				<TriangleAlertIcon className="size-4 text-warning" />
			</div>
			<div className="flex min-w-0 flex-1 flex-col gap-3">
				<div className="flex flex-col gap-1">
					<span className="text-sm leading-5 font-semibold">未定价模型 · {pending.length}</span>
					<span className="text-[12.5px] leading-5 text-muted-foreground">近 7 天有用量但未定价，不计入成本。</span>
				</div>
				{pending.map((model) => {
					const stats = analytics.models.find((item) => item.model === model.model);
					const estimate = reference && stats ? estimateCost(stats, reference) : null;
					return (
						<div className="flex flex-wrap items-center gap-x-6 gap-y-3 rounded-lg border border-warning-border bg-card px-4 py-3" key={model.model}>
							<div className="flex min-w-60 flex-1 flex-col gap-1">
								<div className="flex flex-wrap items-center gap-2">
									<span aria-hidden="true" className="size-2 rounded-[2px]" style={{ background: VIZ.pink }} />
									<span className="font-mono text-[13px] leading-5 font-bold">{model.model}</span>
									{pricing.sync?.unmatchedModels.some((name) => sameModel(name, model.model)) ? <Tag tone="warning">Models.dev 无匹配</Tag> : null}
								</div>
								<span className="text-[11.5px] leading-4 text-faint">
									首次出现 {formatDateTime(model.firstAt)} · 最近使用 {formatDateTime(model.lastAt)}
								</span>
							</div>
							<MiniStat label="7 天 Token" value={formatTokens(model.totalTokens)} />
							<MiniStat label="请求" value={formatCount(model.requests)} />
							<MiniStat label="Token 占比" value={formatPercent(ratio(model.totalTokens, analytics.totals.totalTokens))} />
							{reference && estimate !== null ? <MiniStat label={`按 ${reference.model} 估算`} value={`≈ ${formatCost(estimate)}`} /> : null}
							<div className="flex gap-2">
								{reference ? (
									<Button onClick={() => onAdd(model.model, reference)} size="sm" type="button" variant="outline">
										<CopyIcon data-icon="inline-start" />
										参照 {reference.model}
									</Button>
								) : null}
								<Button onClick={() => onAdd(model.model)} size="sm" type="button">
									<PlusIcon data-icon="inline-start" />
									添加并定价
								</Button>
							</div>
						</div>
					);
				})}
			</div>
		</div>
	);
}

function MiniStat({ label, value }: { label: string; value: string }) {
	return (
		<div className="flex flex-col gap-0.5">
			<span className="text-sm leading-5 font-bold tabular-nums">{value}</span>
			<span className="text-[11px] leading-4 text-faint">{label}</span>
		</div>
	);
}

function PriceTable({
	analytics,
	rows,
	loading,
	focusModel,
	onAdd,
}: {
	analytics: UsageAnalytics | null;
	rows: DraftRow[];
	loading: boolean;
	focusModel: string | null;
	onAdd: (model: string, base?: ModelPrice) => boolean;
}) {
	const { pricing, settings, notify } = useAdmin();
	const [newModel, setNewModel] = useState("");
	const lastSeen = useLastSeen();
	const changes = priceChanges(pricing.prices, rows === null ? null : rows);
	const dirty = changes.length > 0 && rows !== null;
	const stats = (model: string) => analytics?.models.find((item) => sameModel(item.model, model)) ?? null;
	const rank = (model: string) => analytics?.models.findIndex((item) => sameModel(item.model, model)) ?? -1;
	const totalTokens = analytics?.totals.totalTokens ?? 0;
	const totalCost = analytics?.totals.costUsd ?? 0;
	const multiplier = settings.usage.fastCostMultiplier;
	const surcharge = analytics ? analytics.totals.fastBaseCostUsd * (multiplier - 1) : 0;

	const sorted = [...rows].sort((left, right) => {
		const leftNew = !pricing.prices.some((price) => sameModel(price.model, left.model));
		const rightNew = !pricing.prices.some((price) => sameModel(price.model, right.model));
		if (leftNew !== rightNew) return leftNew ? -1 : 1;
		const leftStats = stats(left.model);
		const rightStats = stats(right.model);
		return (
			(rightStats?.costUsd ?? 0) - (leftStats?.costUsd ?? 0) ||
			(rightStats?.totalTokens ?? 0) - (leftStats?.totalTokens ?? 0) ||
			left.model.localeCompare(right.model)
		);
	});

	function update(model: string, field: PriceField, value: string) {
		setDraft(rows.map((row) => (row.model === model ? { ...row, [field]: value } : row)));
	}
	function remove(model: string) {
		setDraft(rows.filter((row) => row.model !== model));
	}
	function submit(event: FormEvent<HTMLFormElement>) {
		event.preventDefault();
		if (!newModel.trim()) return;
		if (onAdd(newModel)) setNewModel("");
		else notify(`模型 ${newModel.trim()} 已在价格表中。`, "error");
	}

	return (
		<SectionCard
			actions={
				<form className="flex flex-wrap items-center gap-2" onSubmit={submit}>
					<label className="text-[12.5px] font-medium text-muted-foreground" htmlFor="pricing-new-model">
						新增模型
					</label>
					<Input
						className="w-44 sm:w-55"
						id="pricing-new-model"
						list="pricing-used-models"
						onChange={(event) => setNewModel(event.target.value)}
						placeholder="模型名称"
						value={newModel}
					/>
					<datalist id="pricing-used-models">
						{pricing.usedModels
							.filter((model) => !rows.some((row) => sameModel(row.model, model)))
							.map((model) => (
								<option key={model} value={model} />
							))}
					</datalist>
					<Button disabled={!newModel.trim()} type="submit" variant="outline">
						<PlusIcon data-icon="inline-start" />
						添加
					</Button>
				</form>
			}
			description="按 7 天成本排序 · 倍率作用于该模型全部单价"
			title="价格表"
		>
			{dirty ? <UnsavedBar analytics={analytics} changes={changes} rows={rows} /> : null}
			<div className="overflow-x-auto rounded-lg border">
				<Table className="min-w-[1040px] table-fixed">
					<colgroup>
						<col className="w-[232px]" />
						<col className="w-[88px]" />
						<col className="w-[88px]" />
						<col className="w-[88px]" />
						<col className="w-[88px]" />
						<col className="w-[72px]" />
						<col className="w-[128px]" />
						<col className="w-[136px]" />
						<col className="w-[96px]" />
						<col />
					</colgroup>
					<TableHeader className="bg-surface-subtle">
						<TableRow className="hover:bg-transparent">
							<TableHead className="h-12 pl-4 text-xs font-medium text-muted-foreground">模型</TableHead>
							{PRICE_FIELDS.map((field) => (
								<TableHead className="h-12 text-xs font-medium text-muted-foreground" key={field}>
									<div className="flex flex-col leading-4">
										{PRICE_FIELD_LABELS[field]}
										{field === "input" ? <span className="text-[10.5px] font-normal text-faint">USD / 1M</span> : null}
									</div>
								</TableHead>
							))}
							<TableHead className="h-12 text-xs font-medium text-muted-foreground">7 天 Token</TableHead>
							<TableHead className="h-12 text-xs font-medium text-muted-foreground">7 天成本 · 占比</TableHead>
							<TableHead className="h-12 text-xs font-medium text-muted-foreground">最近使用</TableHead>
							<TableHead className="h-12 pr-4" />
						</TableRow>
					</TableHeader>
					<TableBody>
						{sorted.map((row) => (
							<PriceRow
								autoFocus={focusModel === row.model}
								costShare={ratio(stats(row.model)?.costUsd ?? 0, totalCost)}
								key={row.model}
								loading={loading}
								onChange={(field, value) => update(row.model, field, value)}
								onRemove={() => remove(row.model)}
								rank={rank(row.model)}
								seenAt={lastSeen.data?.models.find((item) => sameModel(item.model, row.model))?.lastAt ?? null}
								row={row}
								stats={stats(row.model)}
								tokenShare={ratio(stats(row.model)?.totalTokens ?? 0, totalTokens)}
							/>
						))}
						{sorted.length === 0 ? (
							<TableRow className="hover:bg-transparent">
								<TableCell className="py-10 text-center text-sm text-muted-foreground" colSpan={10}>
									还没有模型价格，可从 Models.dev 同步或手动添加。
								</TableCell>
							</TableRow>
						) : null}
					</TableBody>
				</Table>
			</div>
			<div className="flex flex-wrap items-center justify-between gap-2 text-xs leading-4 text-muted-foreground">
				<span className="flex flex-wrap gap-4">
					<span>金额单位：USD / 1M Token</span>
					{analytics ? (
						<span className="text-faint">
							7 天成本 {formatCost(totalCost - surcharge)}
							{surcharge > 0 ? ` · 含 Fast ${multiplier}× 加成共 ${formatCost(totalCost)}` : ""}
						</span>
					) : null}
				</span>
				<a className="inline-flex items-center gap-1 font-medium text-foreground hover:underline" href={pricing.sync?.sourceUrl ?? DEFAULT_SOURCE_URL} rel="noreferrer" target="_blank">
					查看数据源
					<ExternalLinkIcon className="size-3" />
				</a>
			</div>
		</SectionCard>
	);
}

function UnsavedBar({ changes, rows, analytics }: { changes: PriceChange[]; rows: DraftRow[]; analytics: UsageAnalytics | null }) {
	const { pricing } = useAdmin();
	const { save, saving } = useSavePrices();
	const first = changes[0];
	const summary = !first
		? ""
		: first.kind === "added"
			? `${first.model} · 新增`
			: first.kind === "removed"
				? `${first.model} · 删除`
				: `${first.model} · ${first.fields
						.slice(0, 2)
						.map((change) => `${PRICE_FIELD_LABELS[change.field]} ${change.from} → ${Number.isFinite(change.to) ? change.to : "—"}`)
						.join("、")}`;
	let delta: number | null = null;
	if (analytics && rows.every(validRow)) {
		delta = 0;
		for (const change of changes) {
			const stats = analytics.models.find((item) => sameModel(item.model, change.model));
			if (!stats) continue;
			const before = pricing.prices.find((price) => sameModel(price.model, change.model));
			const after = rows.find((row) => sameModel(row.model, change.model));
			delta += (after ? estimateCost(stats, toPrice(after)) : 0) - (before ? estimateCost(stats, before) : 0);
		}
	}
	return (
		<div className="flex flex-wrap items-center gap-3 rounded-lg border border-warning-border bg-warning-soft px-4 py-2">
			<PencilIcon aria-hidden="true" className="size-3.5 text-warning" />
			<div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 text-[12.5px] leading-5">
				<span className="font-semibold text-warning">{changes.length} 个模型有未保存的修改</span>
				<span className="truncate text-muted-foreground">
					{summary}
					{changes.length > 1 ? ` 等` : ""}
					{delta !== null && Math.abs(delta) >= 0.005 ? ` · 预计 7 天成本 ${formatCost(delta, { signed: true })}` : ""}
				</span>
			</div>
			<Button onClick={() => setDraft(null)} size="sm" type="button" variant="ghost">
				撤销修改
			</Button>
			<Button disabled={saving} onClick={() => void save()} size="sm" type="button">
				{saving ? <Spinner data-icon="inline-start" /> : <SaveIcon data-icon="inline-start" />}
				保存价格
			</Button>
		</div>
	);
}

function PriceRow({
	row,
	stats,
	rank,
	tokenShare,
	costShare,
	loading,
	autoFocus,
	seenAt,
	onChange,
	onRemove,
}: {
	row: DraftRow;
	/** Last use across all recorded events. */
	seenAt: number | null;
	stats: ModelStats | null;
	rank: number;
	tokenShare: number | null;
	costShare: number | null;
	loading: boolean;
	autoFocus: boolean;
	onChange: (field: PriceField, value: string) => void;
	onRemove: () => void;
}) {
	const { pricing, now } = useAdmin();
	const lastAt = stats?.lastAt ?? seenAt;
	const saved = pricing.prices.find((price) => sameModel(price.model, row.model));
	const changedFields = PRICE_FIELDS.filter((field) => saved && parseValue(row[field]) !== saved[field]);
	const dirty = !saved || changedFields.length > 0;
	const synced = saved ? pricing.sync?.syncedPrices.some((price) => samePrice(price, saved)) : false;
	const color = rank >= 0 ? seriesColor(rank) : VIZ.gray;
	const everUsed = lastAt !== null || pricing.usedModels.some((model) => sameModel(model, row.model));

	return (
		<TableRow className={cn("hover:bg-surface-subtle/50", dirty && "bg-warning-soft/55 hover:bg-warning-soft/70")}>
			<TableCell className={cn("py-3 pr-2 pl-4 whitespace-normal", dirty && "shadow-[inset_3px_0_0_var(--warning)]")}>
				<div className="flex min-w-0 flex-col gap-1">
					<div className="flex min-w-0 items-center gap-2">
						<span aria-hidden="true" className="size-2 shrink-0 rounded-[2px]" style={{ background: color }} />
						<span className="truncate font-mono text-[12.5px] leading-5 font-bold" title={row.model}>
							{row.model}
						</span>
						{saved && dirty ? (
							<Tag tone="warning">
								<StatusDot tone="warning">已修改</StatusDot>
							</Tag>
						) : null}
					</div>
					<div className="pl-4">
						{!saved ? (
							<Tag tone="info">新增</Tag>
						) : synced ? (
							<Tag>
								<StatusDot tone="success">Models.dev</StatusDot>
							</Tag>
						) : (
							<Tag>手动</Tag>
						)}
					</div>
				</div>
			</TableCell>
			{PRICE_FIELDS.map((field, index) => {
				const changed = changedFields.includes(field);
				const value = parseValue(row[field]);
				const invalid = !Number.isFinite(value) || value < 0;
				return (
					<TableCell className="px-2 py-3 align-middle" key={field}>
						<div className="flex flex-col gap-1">
							<Input
								aria-invalid={invalid || undefined}
								aria-label={`${row.model} ${PRICE_FIELD_LABELS[field]}`}
								autoFocus={autoFocus && index === 0}
								className={cn("h-7 rounded-md px-2 font-mono text-xs", changed && "border-[1.5px] border-warning", value === 0 && "text-faint")}
								inputMode="decimal"
								onChange={(event) => onChange(field, event.target.value)}
								value={row[field]}
							/>
							{changed && saved ? <span className="text-[10.5px] leading-4 text-warning">原 {saved[field]}</span> : null}
						</div>
					</TableCell>
				);
			})}
			<TableCell className="px-2 py-3">
				{loading ? (
					<Skeleton className="h-8 w-full" />
				) : stats ? (
					<div className="flex flex-col gap-1">
						<span className="flex items-baseline gap-1.5">
							<span className="text-[12.5px] leading-5 font-semibold tabular-nums">{formatTokens(stats.totalTokens)}</span>
							<span className="text-[11px] text-faint">{formatPercent(tokenShare, 0)}</span>
						</span>
						<Meter color={color} height={4} ratio={tokenShare} />
					</div>
				) : (
					<span className="text-xs text-muted-foreground">{everUsed ? "近 7 天未使用" : "从未使用"}</span>
				)}
			</TableCell>
			<TableCell className="px-2 py-3">
				{loading ? (
					<Skeleton className="h-8 w-full" />
				) : stats && stats.costUsd > 0 ? (
					<div className="flex flex-col gap-1">
						<span className="flex items-baseline gap-1.5">
							<span className="text-[12.5px] leading-5 font-semibold tabular-nums">{formatCost(stats.costUsd)}</span>
							<span className="text-[11px] text-faint">{formatPercent(costShare)}</span>
						</span>
						<Meter color={VIZ.teal} height={4} ratio={costShare} />
					</div>
				) : (
					<span className="text-xs text-muted-foreground tabular-nums">$0.00</span>
				)}
			</TableCell>
			<TableCell className="px-2 py-3">
				<div className="flex flex-col gap-1 text-xs leading-4">
					<span className="tabular-nums">{lastAt ? formatDateTime(lastAt) : "—"}</span>
					<span className="text-[11px] text-faint">{lastAt ? formatAgo(lastAt, now) : everUsed ? "—" : "从未使用"}</span>
				</div>
			</TableCell>
			<TableCell className="py-3 pr-4 pl-2 text-right">
				<Button aria-label={`删除 ${row.model}`} onClick={onRemove} size="sm" type="button" variant="destructive">
					删除
				</Button>
			</TableCell>
		</TableRow>
	);
}
