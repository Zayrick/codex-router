import { useMemo, useState } from "react";

import type { AnalyticsQuery } from "@/admin-api";
import { CardSkeleton, KpiSkeletonRow, LoadError } from "@/admin/analytics/common";
import { LATENCY_TARGETS } from "@/admin/analytics/metrics";
import { cycleBounds, useAnalyticsState } from "@/admin/analytics/filter-state";
import { AnalyticsFilterMenus } from "@/admin/analytics/filters";
import { LatencyDiagnosisCard, LatencyTrendCard, OutcomesCard, PerformanceKpis, TransportStreamCard, type LatencyTargets } from "@/admin/analytics/PerformanceCards";
import { FastFulfilmentCard, ModelPerformanceCard, UpstreamReliabilityCard } from "@/admin/analytics/ReliabilityCards";
import { useAdmin } from "@/admin/context";
import { useAnalytics } from "@/admin/hooks";
import { rangeQuery, type RangeKey } from "@/admin/range";
import { RangeToolbar } from "@/components/app/range-toolbar";

const TARGETS_KEY = "codex-router.latency-targets";

function loadTargets(): LatencyTargets {
	try {
		const value = JSON.parse(localStorage.getItem(TARGETS_KEY) ?? "null") as Partial<LatencyTargets> | null;
		if (value && Number(value.ttftP95Ms) > 0 && Number(value.latencyP95Ms) > 0) {
			return { ttftP95Ms: Number(value.ttftP95Ms), latencyP95Ms: Number(value.latencyP95Ms) };
		}
	} catch {
		// fall back to the defaults
	}
	return { ...LATENCY_TARGETS };
}

/** Bucket used for a range when the user has not picked one. */
function defaultBucket(range: RangeKey): string {
	return range === "24h" ? "60" : range === "7d" || range === "cycle" ? "360" : "1440";
}

/** Figma "Admin v2 / 性能与可靠性". */
export default function PerformancePage() {
	const { now, quota, state, navigate } = useAdmin();
	const { range, setRange, filters, setFilters } = useAnalyticsState();
	const [bucket, setBucket] = useState<string | null>(null);
	const [targets, setTargets] = useState(loadTargets);
	const cycle = cycleBounds(quota, filters.upstream);
	const effectiveRange: RangeKey = range === "cycle" && !cycle ? "7d" : range;
	const bucketMinutes = bucket ?? defaultBucket(effectiveRange);
	const query = useMemo<AnalyticsQuery>(
		() => ({
			...rangeQuery(effectiveRange, now, cycle),
			bucketMinutes: Number(bucketMinutes),
			downstream: filters.downstream,
			upstream: filters.upstream,
			models: filters.models,
			transports: filters.transports,
		}),
		// eslint-disable-next-line react-hooks/exhaustive-deps
		[effectiveRange, cycle?.startAt, cycle?.endAt, filters, bucketMinutes],
	);
	const { data, error, loading, reload } = useAnalytics(query);
	const options: RangeKey[] = cycle ? ["cycle", "24h", "7d", "30d", "all"] : ["24h", "7d", "30d", "all"];

	function changeRange(next: RangeKey) {
		setRange(next);
		setBucket(null);
	}

	function changeTargets(next: LatencyTargets) {
		setTargets(next);
		localStorage.setItem(TARGETS_KEY, JSON.stringify(next));
	}

	const upstreamParams = (accountId: string) => ({ upstreamType: "codex_account", upstreamId: accountId, view: "failed" });

	return (
		<div className="flex flex-col gap-4">
			<RangeToolbar
				bounds={data ? { startAt: data.startAt, endAt: data.endAt } : null}
				loading={loading}
				now={now}
				onChange={changeRange}
				onRefresh={reload}
				options={options}
				value={effectiveRange}
			>
				<AnalyticsFilterMenus filters={filters} onChange={setFilters} show={["upstream", "model", "transport"]} />
			</RangeToolbar>
			{error ? <LoadError message={error} onRetry={reload} /> : null}
			{!data ? (
				<>
					<KpiSkeletonRow />
					<CardSkeleton height={240} />
					<CardSkeleton height={320} />
				</>
			) : (
				<>
					<PerformanceKpis analytics={data} now={now} />
					<LatencyTrendCard
						analytics={data}
						bucketMinutes={bucketMinutes}
						now={now}
						onBucketChange={setBucket}
						onTargetsChange={changeTargets}
						targets={targets}
					/>
					<div className="grid gap-4 xl:grid-cols-2">
						<OutcomesCard
							analytics={data}
							now={now}
							onOpenLogs={() =>
								navigate("logs", {
									view: "failed",
									...(filters.upstream ? { upstreamType: filters.upstream.identityType, upstreamId: filters.upstream.identityId } : {}),
								})
							}
						/>
						<LatencyDiagnosisCard analytics={data} />
					</div>
					<ModelPerformanceCard analytics={data} targets={targets} />
					<UpstreamReliabilityCard
						accounts={state.codexAccounts}
						analytics={data}
						groups={state.accountGroups}
						now={now}
						onOpenGroup={() => navigate("groups")}
						onOpenLogs={(accountId) => navigate("logs", upstreamParams(accountId))}
						quota={quota}
					/>
					<div className="grid items-start gap-4 xl:grid-cols-2">
						<FastFulfilmentCard analytics={data} />
						<TransportStreamCard analytics={data} />
					</div>
				</>
			)}
		</div>
	);
}
