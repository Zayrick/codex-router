// Usage tab of the API key / downstream account view (Figma "User v2 / 用量").

import { TriangleAlertIcon } from "lucide-react";
import { useMemo, useState } from "react";

import type { UsageAnalytics } from "@/admin-api";
import { CardSkeleton, KpiSkeletonRow, LoadError } from "@/admin/analytics/common";
import { ModelEfficiencyCard } from "@/admin/analytics/ModelEfficiencyCard";
import { SessionsClientsCard } from "@/admin/analytics/SessionClientCards";
import { TokenTrendCard } from "@/admin/analytics/TokenTrendCard";
import { ActivityHeatmapCard, HealthLatencyCard, ModelShareCard } from "@/admin/analytics/UserCards";
import { UsageKpis } from "@/admin/analytics/UsageKpis";
import { rangeQuery, type RangeKey } from "@/admin/range";
import { useResource } from "@/admin/use-resource";
import { FilterMenu } from "@/components/app/filter-menu";
import { RangeToolbar } from "@/components/app/range-toolbar";
import { Button } from "@/components/ui/button";
import { DAY_MS, formatClock, formatSpan, startOfLocalDay } from "@/lib/format";
import type { UserApiClient, UserProfile } from "@/user-api";

import UserRouteQuota from "./UserRouteQuota";

const REFRESH_MS = 5 * 60_000;
const RANGES: RangeKey[] = ["24h", "7d", "30d", "all"];

interface ThrottleForecast {
	accountName: string;
	windowLabel: string;
	exhaustAt: number;
	others: string[];
}

/** Soonest window of the routed accounts projected to run out before it resets. */
function throttleForecast(profile: UserProfile | null): ThrottleForecast | null {
	const accounts = profile?.quota?.accounts ?? [];
	let best: ThrottleForecast | null = null;
	for (const account of accounts) {
		for (const window of account.windows) {
			if (window.category !== "codex" || !window.exhaustsBeforeReset || !window.exhaustAt) continue;
			if (best && best.exhaustAt <= window.exhaustAt) continue;
			best = {
				accountName: account.accountName,
				windowLabel: window.kind === "five_hour" || window.kind === "primary" ? "5 小时" : window.kind === "weekly" || window.kind === "secondary" ? "7 天" : window.name,
				exhaustAt: window.exhaustAt,
				others: accounts.filter((entry) => entry.accountId !== account.accountId && entry.enabled).map((entry) => entry.accountName.split("@")[0] ?? entry.accountName),
			};
		}
	}
	return best;
}

export default function UserUsage({
	api,
	profile,
	now,
	onSessionExpired,
	onOpenLogs,
}: {
	api: UserApiClient;
	profile: UserProfile | null;
	now: number;
	onSessionExpired: () => void;
	/** Opens the log tab with these filters. */
	onOpenLogs?: (params: Record<string, string>) => void;
}) {
	const [range, setRange] = useState<RangeKey>("7d");
	const [models, setModels] = useState<string[]>([]);
	const query = useMemo(() => ({ ...rangeQuery(range, now), models }), [range, models]); // eslint-disable-line react-hooks/exhaustive-deps
	const usage = useResource<UsageAnalytics>((signal) => api.getAnalytics(query, signal), [JSON.stringify(query)], { refreshMs: REFRESH_MS, onSessionExpired });
	const heatStart = startOfLocalDay(now) - 6 * DAY_MS;
	const heatmap = useResource<UsageAnalytics>(
		(signal) => api.getAnalytics({ range: "custom", startAt: heatStart, endAt: heatStart + 7 * DAY_MS, bucketMinutes: 60, models }, signal),
		[heatStart, JSON.stringify(models)],
		{ refreshMs: REFRESH_MS, onSessionExpired },
	);
	const data = usage.data;
	const forecast = throttleForecast(profile);
	const modelOptions = (data?.models ?? []).map((model) => ({ value: model.model, label: <span className="font-mono">{model.model}</span>, text: model.model }));

	return (
		<div className="flex flex-col gap-4">
			<RangeToolbar
				bounds={data ? { startAt: data.startAt, endAt: data.endAt } : null}
				loading={usage.loading}
				now={now}
				onChange={setRange}
				onRefresh={() => {
					usage.reload();
					heatmap.reload();
				}}
				options={RANGES}
				value={range}
			>
				<span className="flex items-center gap-1.5 text-[11.5px] text-faint max-md:hidden">
					<span aria-hidden="true" className="size-1.5 rounded-full bg-success" />
					每 5 分钟自动刷新{data ? ` · ${formatClock(now, false)} 更新` : ""}
				</span>
				<FilterMenu align="end" label="模型" onChange={setModels} options={modelOptions} selected={models} />
			</RangeToolbar>
			{forecast ? (
				<div className="flex flex-wrap items-center gap-3 rounded-xl border border-warning-border bg-warning-soft px-5 py-3">
					<TriangleAlertIcon className="size-4 shrink-0 text-warning" />
					<div className="flex min-w-0 flex-1 flex-col">
						<span className="text-[13px] font-semibold text-warning">
							{forecast.accountName} 的 {forecast.windowLabel}额度约 {formatSpan(forecast.exhaustAt - now)}后（{formatClock(forecast.exhaustAt, false)}）耗尽
						</span>
						<span className="text-xs text-muted-foreground">
							{forecast.others.length ? `请求会自动转到 ${forecast.others.join(" / ")}，不会中断。` : "耗尽后请求将被限流，直到额度重置。"}
						</span>
					</div>
					<Button onClick={() => document.getElementById("user-route-quota")?.scrollIntoView({ behavior: "smooth" })} size="sm" type="button" variant="ghost">
						查看额度
					</Button>
				</div>
			) : null}
			{usage.error ? <LoadError message={usage.error} onRetry={usage.reload} /> : null}
			{!data ? (
				<>
					<KpiSkeletonRow />
					<CardSkeleton height={260} />
				</>
			) : (
				<>
					<UsageKpis analytics={data} now={now} />
					<div className="grid gap-4 xl:grid-cols-3">
						<TokenTrendCard analytics={data} className="xl:col-span-2" now={now} />
						<ModelShareCard analytics={data} />
					</div>
					<div className="grid gap-4 lg:grid-cols-2">
						<ActivityHeatmapCard metric="tokens" now={now} series={heatmap.data?.series ?? null} />
						<ActivityHeatmapCard metric="cost" now={now} series={heatmap.data?.series ?? null} />
					</div>
					<div className="grid items-start gap-4 xl:grid-cols-2">
						<HealthLatencyCard analytics={data} />
						<SessionsClientsCard
							analytics={data}
							now={now}
							{...(onOpenLogs ? { onOpenLogs: () => onOpenLogs({}), onOpenSession: (sessionId: string) => onOpenLogs({ sessionId }) } : {})}
						/>
					</div>
					<ModelEfficiencyCard analytics={data} title="使用的模型" />
				</>
			)}
			{profile ? (
				<div id="user-route-quota">
					<UserRouteQuota analytics={data} now={now} profile={profile} />
				</div>
			) : null}
		</div>
	);
}
