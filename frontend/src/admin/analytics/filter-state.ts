import { useState } from "react";

import type { QuotaOverview, UsageIdentityFilter, UsageIdentityType } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { parseRangeKey, type Bounds, type RangeKey } from "@/admin/range";

export interface AdminAnalyticsFilters {
	downstream: UsageIdentityFilter | null;
	upstream: UsageIdentityFilter | null;
	models: string[];
	transports: string[];
}

function identityParam(params: URLSearchParams, kind: string, id: string): UsageIdentityFilter | null {
	const type = params.get(kind);
	const value = params.get(id);
	if (!type || !value) return null;
	if (!["user", "credential", "codex_account", "account_group"].includes(type)) return null;
	return { identityType: type as UsageIdentityType, identityId: value };
}

/** Filter and range state initialized from the page's search params. */
export function useAnalyticsState(defaultRange: RangeKey = "7d") {
	const { params } = useAdmin();
	const [range, setRange] = useState<RangeKey>(() => parseRangeKey(params.get("range"), defaultRange));
	const [filters, setFilters] = useState<AdminAnalyticsFilters>(() => ({
		downstream: identityParam(params, "downstreamType", "downstreamId"),
		upstream: identityParam(params, "upstreamType", "upstreamId"),
		models: params.getAll("model"),
		transports: params.getAll("transport"),
	}));
	return { range, setRange, filters, setFilters };
}

/** Weekly Codex window of one account, used as "本周期". */
export function cycleBounds(quota: QuotaOverview | null, upstream: UsageIdentityFilter | null): Bounds | null {
	if (!quota || upstream?.identityType !== "codex_account") return null;
	const account = quota.accounts.find((entry) => entry.accountId === upstream.identityId);
	const window =
		account?.windows.find((entry) => entry.category === "codex" && entry.kind === "weekly") ??
		account?.windows.find((entry) => entry.category === "codex" && entry.kind === "secondary");
	if (!window?.windowStartAt || !window.resetAt) return null;
	return { startAt: window.windowStartAt, endAt: window.resetAt };
}
