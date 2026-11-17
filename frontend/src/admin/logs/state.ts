// Request-log filter state, its URL contract and saved views.
//
// URL contract (other pages link here with `navigate("logs", params)`), all optional:
//   range        1h | today | 7d | 30d | custom   (default today)
//   startAt/endAt  epoch ms, with range=custom
//   downstreamType/downstreamId   api_key | auth_proxy + id
//   upstreamType/upstreamId       account_group | codex_account + id
//   model, status, effort, tier, transport, sessionKind   comma-separated lists
//   sessionId    request's session or parent session
//   q            request / response / session ID search
//   view         all | failed | slow | downgraded | subagent | rate_limited
//   group        request | session

import type { EventPageQuery, EventView, UsageEventStatus, UsageIdentityFilter, UsageIdentityType } from "@/admin-api";
import { DAY_MS, HOUR_MS, MINUTE_MS, startOfLocalDay } from "@/lib/format";

export type LogRange = "1h" | "today" | "24h" | "7d" | "30d" | "custom";

export interface LogFilters {
	range: LogRange;
	custom: { startAt: number; endAt: number } | null;
	downstream: UsageIdentityFilter | null;
	upstream: UsageIdentityFilter | null;
	models: string[];
	statuses: UsageEventStatus[];
	efforts: string[];
	tiers: string[];
	transports: string[];
	sessionKinds: string[];
	sessionId: string;
	search: string;
}

export const EMPTY_FILTERS: LogFilters = {
	range: "today",
	custom: null,
	downstream: null,
	upstream: null,
	models: [],
	statuses: [],
	efforts: [],
	tiers: [],
	transports: [],
	sessionKinds: [],
	sessionId: "",
	search: "",
};

export const RANGE_OPTIONS: { value: LogRange; label: string }[] = [
	{ value: "1h", label: "1 小时" },
	{ value: "today", label: "今天" },
	{ value: "7d", label: "7 天" },
	{ value: "30d", label: "30 天" },
	{ value: "custom", label: "自定义" },
];

const IDENTITY_TYPES: UsageIdentityType[] = ["api_key", "auth_proxy", "codex_account", "account_group"];
const STATUSES: UsageEventStatus[] = ["completed", "incomplete", "failed", "cancelled", "interrupted"];
const VIEWS: EventView[] = ["all", "failed", "slow", "downgraded", "subagent", "rate_limited"];

function list(params: URLSearchParams, name: string): string[] {
	return params
		.getAll(name)
		.flatMap((value) => value.split(","))
		.map((value) => value.trim())
		.filter(Boolean);
}

function identity(params: URLSearchParams, kind: string, id: string): UsageIdentityFilter | null {
	const type = params.get(kind) as UsageIdentityType | null;
	const value = params.get(id);
	return type && value && IDENTITY_TYPES.includes(type) ? { identityType: type, identityId: value } : null;
}

export function filtersFromParams(params: URLSearchParams): { filters: LogFilters; view: EventView; grouped: boolean } {
	const range = (["1h", "today", "24h", "7d", "30d", "custom"] as LogRange[]).find((value) => value === params.get("range")) ?? "today";
	const startAt = Number(params.get("startAt"));
	const endAt = Number(params.get("endAt"));
	const custom = range === "custom" && startAt > 0 && endAt > startAt ? { startAt, endAt } : null;
	const view = VIEWS.find((value) => value === params.get("view")) ?? "all";
	return {
		filters: {
			range: range === "custom" && !custom ? "today" : range,
			custom,
			downstream: identity(params, "downstreamType", "downstreamId"),
			upstream: identity(params, "upstreamType", "upstreamId"),
			models: list(params, "model"),
			statuses: list(params, "status").filter((value): value is UsageEventStatus => STATUSES.includes(value as UsageEventStatus)),
			efforts: list(params, "effort"),
			tiers: list(params, "tier"),
			transports: list(params, "transport"),
			sessionKinds: list(params, "sessionKind"),
			sessionId: params.get("sessionId") ?? "",
			search: params.get("q") ?? "",
		},
		view,
		grouped: params.get("group") === "session",
	};
}

export interface ResolvedRange {
	startAt: number;
	endAt: number;
	/** Bucket minutes for the distribution chart. */
	bucket: number;
}

/** Concrete bounds for a range at `now`. */
export function resolveRange(filters: LogFilters, now: number): ResolvedRange {
	switch (filters.range) {
		case "1h": {
			const endAt = Math.ceil(now / MINUTE_MS) * MINUTE_MS;
			return { startAt: endAt - HOUR_MS, endAt, bucket: 1 };
		}
		case "24h": {
			const endAt = Math.ceil(now / MINUTE_MS) * MINUTE_MS;
			return { startAt: endAt - DAY_MS, endAt, bucket: 10 };
		}
		case "7d": {
			const endAt = Math.ceil(now / HOUR_MS) * HOUR_MS;
			return { startAt: endAt - 7 * DAY_MS, endAt, bucket: 60 };
		}
		case "30d": {
			const endAt = Math.ceil(now / HOUR_MS) * HOUR_MS;
			return { startAt: endAt - 30 * DAY_MS, endAt, bucket: 360 };
		}
		case "custom":
			if (filters.custom) return { ...filters.custom, bucket: bucketFor(filters.custom.endAt - filters.custom.startAt) };
			break;
		case "today":
			break;
	}
	const startAt = startOfLocalDay(now);
	return { startAt, endAt: startAt + DAY_MS, bucket: 10 };
}

/** About 100–150 bars for any custom length. */
export function bucketFor(length: number): number {
	const target = length / 120 / MINUTE_MS;
	const steps = [1, 2, 5, 10, 15, 30, 60, 120, 180, 360, 720, 1440];
	return steps.find((step) => step >= target) ?? 1440;
}

export function buildQuery(
	filters: LogFilters,
	range: ResolvedRange,
	extra: { view: EventView; limit: number; grouped: boolean },
): EventPageQuery {
	return {
		range: "custom",
		startAt: range.startAt,
		endAt: range.endAt,
		bucketMinutes: range.bucket,
		downstream: filters.downstream,
		upstream: filters.upstream,
		models: filters.models,
		statuses: filters.statuses,
		efforts: filters.efforts,
		tiers: filters.tiers,
		transports: filters.transports,
		sessionKinds: filters.sessionKinds,
		...(filters.sessionId ? { sessionId: filters.sessionId } : {}),
		...(filters.search.trim() ? { search: filters.search.trim() } : {}),
		view: extra.view,
		limit: extra.limit,
		group: extra.grouped ? "session" : "request",
	};
}

/** Number of field conditions (not range, view or search). */
export function conditionCount(filters: LogFilters): number {
	return [
		filters.downstream,
		filters.upstream,
		filters.models.length,
		filters.statuses.length,
		filters.efforts.length,
		filters.tiers.length,
		filters.transports.length,
		filters.sessionKinds.length,
		filters.sessionId,
	].filter(Boolean).length;
}

// ---------------------------------------------------------------------------
// Saved views (localStorage)
// ---------------------------------------------------------------------------

export interface SavedView {
	id: string;
	name: string;
	filters: LogFilters;
	view: EventView;
}

function storageKey(scope: string): string {
	return `codex-router.logs.views.${scope}`;
}

export function loadSavedViews(scope: string): SavedView[] {
	try {
		const raw = window.localStorage.getItem(storageKey(scope));
		const parsed: unknown = raw ? JSON.parse(raw) : [];
		return Array.isArray(parsed)
			? parsed.filter((item): item is SavedView => typeof item === "object" && item !== null && "id" in item && "filters" in item)
			: [];
	} catch {
		return [];
	}
}

export function storeSavedViews(scope: string, views: SavedView[]): void {
	try {
		window.localStorage.setItem(storageKey(scope), JSON.stringify(views));
	} catch {
		// Storage may be unavailable (private mode); views then last for the session only.
	}
}
