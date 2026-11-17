import { useMemo } from "react";

import type { AnalyticsQuery } from "@/admin-api";
import { CardSkeleton, KpiSkeletonRow, LoadError } from "@/admin/analytics/common";
import { cycleBounds, useAnalyticsState } from "@/admin/analytics/filter-state";
import { AnalyticsFilterMenus } from "@/admin/analytics/filters";
import { EffortTierCard, IdentityModelCard } from "@/admin/analytics/MatrixCards";
import { ModelEfficiencyCard } from "@/admin/analytics/ModelEfficiencyCard";
import { DownstreamCard, UpstreamCard } from "@/admin/analytics/RankingCards";
import { ClientsCard, SessionMixCard, TransportEndpointsCard } from "@/admin/analytics/SessionClientCards";
import { TokenTrendCard } from "@/admin/analytics/TokenTrendCard";
import { UsageKpis } from "@/admin/analytics/UsageKpis";
import { useAdmin } from "@/admin/context";
import { useAnalytics } from "@/admin/hooks";
import { rangeQuery, type RangeKey } from "@/admin/range";
import { RangeToolbar } from "@/components/app/range-toolbar";
import { planLabel } from "@/components/app/viz";

import { strategyLabel } from "../routing-model";

/** Figma "Admin v2 / 用量分析". */
export default function UsagePage() {
	const { now, quota, state, navigate } = useAdmin();
	const { range, setRange, filters, setFilters } = useAnalyticsState();
	const cycle = cycleBounds(quota, filters.upstream);
	const effectiveRange: RangeKey = range === "cycle" && !cycle ? "7d" : range;
	const query = useMemo<AnalyticsQuery>(
		() => ({
			...rangeQuery(effectiveRange, now, cycle),
			downstream: filters.downstream,
			upstream: filters.upstream,
			models: filters.models,
		}),
		// `now` only matters for ranges resolved on the client; avoid refetching every tick.
		// eslint-disable-next-line react-hooks/exhaustive-deps
		[effectiveRange, cycle?.startAt, cycle?.endAt, filters],
	);
	const { data, error, loading, reload } = useAnalytics(query);
	const options: RangeKey[] = cycle ? ["cycle", "24h", "7d", "30d", "all"] : ["24h", "7d", "30d", "all"];
	const meta = {
		groupStrategy: (id: string) => {
			const group = state.accountGroups.find((entry) => entry.id === id);
			return group ? strategyLabel(group.strategy) : null;
		},
		groupSize: (id: string) => state.accountGroups.find((entry) => entry.id === id)?.accountIds.length ?? null,
		accountPlan: (id: string) => planLabel(quota?.accounts.find((entry) => entry.accountId === id)?.planType) || null,
	};

	return (
		<div className="flex flex-col gap-4">
			<RangeToolbar
				bounds={data ? { startAt: data.startAt, endAt: data.endAt } : null}
				loading={loading}
				now={now}
				onChange={setRange}
				onRefresh={reload}
				options={options}
				value={effectiveRange}
			>
				<AnalyticsFilterMenus filters={filters} onChange={setFilters} />
			</RangeToolbar>
			{error ? <LoadError message={error} onRetry={reload} /> : null}
			{!data ? (
				<>
					<KpiSkeletonRow />
					<CardSkeleton height={258} />
					<CardSkeleton height={320} />
				</>
			) : (
				<>
					<UsageKpis analytics={data} now={now} />
					<TokenTrendCard analytics={data} now={now} />
					<ModelEfficiencyCard analytics={data} />
					<div className="grid gap-4 xl:grid-cols-2">
						<DownstreamCard
							analytics={data}
							onSelect={(identity) => setFilters({ ...filters, downstream: { identityType: identity.identityType, identityId: identity.identityId } })}
						/>
						<UpstreamCard
							analytics={data}
							meta={meta}
							onSelectAccount={(account) => navigate("logs", { upstreamType: "codex_account", upstreamId: account.codexAccountId })}
						/>
					</div>
					<div className="grid gap-4 xl:grid-cols-3">
						<IdentityModelCard analytics={data} className="xl:col-span-2" />
						<EffortTierCard analytics={data} />
					</div>
					<div className="grid gap-4 lg:grid-cols-3">
						<SessionMixCard analytics={data} />
						<ClientsCard analytics={data} />
						<TransportEndpointsCard analytics={data} />
					</div>
				</>
			)}
		</div>
	);
}
