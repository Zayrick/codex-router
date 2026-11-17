// API of the signed-in API key / downstream account views. Every endpoint is
// limited to the signed-in identity by the server.

import {
	AdminApiError,
	AdminSessionExpiredError,
	analyticsSearchParams,
	type AnalyticsQuery,
	type EventPageQuery,
	type QuotaOverview,
	type RouteTargetType,
	type UsageAnalytics,
	type UsageEventPage,
} from "./admin-api";
import { CONSOLE_API_BASE } from "./session-api";

export interface UserIdentity {
	identityType: "api_key" | "auth_proxy";
	name: string;
	/** Last characters of the API key or account_id. */
	credentialHint: string;
}

export interface UserGroupSettings {
	id: string;
	name: string;
	strategy: string;
	sessionAffinity: boolean;
	sessionAffinityTtl: string;
}

export interface UserProfile {
	/** Whether administrators allow users to see account quota. */
	visible: boolean;
	identity: UserIdentity;
	route: { targetType: RouteTargetType; targetId: string } | null;
	group?: UserGroupSettings | null;
	quota?: QuotaOverview;
}

/** Same query shape as the admin API; identity filters are ignored by the server. */
export type UserAnalyticsQuery = Omit<AnalyticsQuery, "downstream">;
export type UserEventPageQuery = Omit<EventPageQuery, "downstream">;

export class UserApiClient {
	readonly basePath = `${CONSOLE_API_BASE}/account`;

	getProfile(): Promise<UserProfile> {
		return this.requestJson<UserProfile>("/quota");
	}

	getAnalytics(query: UserAnalyticsQuery, signal?: AbortSignal): Promise<UsageAnalytics> {
		return this.requestJson<UsageAnalytics>(`/analytics?${analyticsSearchParams(query).toString()}`, signal);
	}

	getEvents(query: UserEventPageQuery, signal?: AbortSignal): Promise<UsageEventPage> {
		return this.requestJson<UsageEventPage>(`/events?${analyticsSearchParams(query).toString()}`, signal);
	}

	eventsCsvUrl(query: UserEventPageQuery): string {
		const params = analyticsSearchParams({ ...query, offset: undefined, limit: undefined, group: undefined });
		params.set("format", "csv");
		return `${this.basePath}/events?${params.toString()}`;
	}

	private async requestJson<T>(path: string, signal?: AbortSignal): Promise<T> {
		const response = await fetch(`${this.basePath}${path}`, {
			credentials: "same-origin",
			headers: { Accept: "application/json" },
			signal: signal ?? null,
		});
		if (response.status === 401) throw new AdminSessionExpiredError();
		if (!response.ok) {
			let message = "读取用量失败，请稍后重试。";
			try {
				const payload = (await response.json()) as { error?: { message?: unknown } };
				if (typeof payload.error?.message === "string") message = payload.error.message;
			} catch {
				// keep the fallback message
			}
			throw new AdminApiError(message, response.status);
		}
		return (await response.json()) as T;
	}
}
