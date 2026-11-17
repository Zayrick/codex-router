import { AdminSessionExpiredError, type AnalyticsQuery, type EventPageQuery, type UsageAnalytics, type UsageEventPage } from "@/admin-api";

import { useAdmin } from "./context";
import { useResource, type Resource } from "./use-resource";

/** Analytics for `query` (null skips loading); refetches when prices or identity metadata change. */
export function useAnalytics(query: AnalyticsQuery | null, options: { refreshMs?: number } = {}): Resource<UsageAnalytics> {
	const { api, state, costVersion, reportError } = useAdmin();
	return useResource((signal) => api.getAnalytics(query as AnalyticsQuery, signal), [JSON.stringify(query), costVersion, state.users, state.codexAccounts, state.accountGroups], {
		enabled: query !== null,
		onSessionExpired: () => reportError(new AdminSessionExpiredError(), ""),
		...(options.refreshMs ? { refreshMs: options.refreshMs } : {}),
	});
}

export function useEvents(query: EventPageQuery | null, options: { refreshMs?: number } = {}): Resource<UsageEventPage> {
	const { api, state, costVersion, reportError } = useAdmin();
	return useResource((signal) => api.getEvents(query as EventPageQuery, signal), [JSON.stringify(query), costVersion, state.users, state.codexAccounts, state.accountGroups], {
		enabled: query !== null,
		onSessionExpired: () => reportError(new AdminSessionExpiredError(), ""),
		...(options.refreshMs ? { refreshMs: options.refreshMs } : {}),
	});
}
