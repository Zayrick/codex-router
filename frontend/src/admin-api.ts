export interface OAuthStatus {
	email: string | null;
	accountId: string | null;
	expiresAt: number;
}

export interface SubscriptionMetadata {
	planType: string | null;
	subscriptionActiveStart: number | null;
	subscriptionActiveUntil: number | null;
}

export type QuotaWindowKind =
	| "five_hour"
	| "weekly"
	| "monthly"
	| "primary"
	| "secondary";

export type QuotaCategory = "codex" | "code_review" | "additional";

export interface QuotaWindow {
	id: string;
	category: QuotaCategory;
	name: string;
	kind: QuotaWindowKind;
	usedPercent: number | null;
	remainingPercent: number | null;
	limitWindowSeconds: number | null;
	resetAt: number | null;
	allowed: boolean | null;
	limitReached: boolean;
}

export interface SubscriptionInfo extends SubscriptionMetadata {
	windows: QuotaWindow[];
	rateLimitResetCredits: {
		availableCount: number | null;
		applicableAvailableCount: number | null;
	};
	fetchedAt: number;
}

export interface CodexAccount {
	id: string;
	name: string;
	enabled: boolean;
	oauth: OAuthStatus | null;
	subscription: SubscriptionMetadata | null;
}

export interface CodexAccountUpdate {
	name: string;
	enabled: boolean;
}

export interface CodexCredentialImport {
	name?: string;
	credentials: Record<string, unknown>;
}

/** API key, or the ChatGPT account id of a client relaying its own sign-in. */
export type CredentialType = "api_key" | "account_id";

export interface UserCredentialInput {
	/** Omitted for new credentials. */
	id?: string;
	type: CredentialType;
	value: string;
	/** Optional label, e.g. the device using it. */
	name: string;
	enabled: boolean;
}

export interface UserCredential extends UserCredentialInput {
	id: string;
}

export interface UserInput {
	name: string;
	enabled: boolean;
	credentials: UserCredentialInput[];
}

export interface User extends UserInput {
	id: string;
	credentials: UserCredential[];
}

export type AccountGroupStrategy =
	| "round-robin"
	| "weighted-round-robin"
	| "fallback";

export interface AccountGroup {
	id: string;
	name: string;
	accountIds: string[];
	strategy: AccountGroupStrategy;
	sessionAffinity: boolean;
	sessionAffinityTtl: string;
}

export type RouteTargetType = "account" | "group";

export interface RouteAssignment {
	userId: string;
	targetType: RouteTargetType;
	targetId: string;
}

export interface AccountRoutingConfiguration {
	accountGroups: AccountGroup[];
	routes: RouteAssignment[];
}

export interface AdminState extends AccountRoutingConfiguration {
	codexAccounts: CodexAccount[];
	users: User[];
}

export interface AdminSettings {
	publicAccount: {
		showQuota: boolean;
	};
	upstream: {
		codexClientVersion: string;
	};
	usage: {
		/** Cost multiplier for Fast (priority tier) requests. */
		fastCostMultiplier: number;
	};
	notifications: {
		resetWatchEnabled: boolean;
		quotaResetEnabled: boolean;
		usageWarningEnabled: boolean;
		allAccounts: boolean;
		accountIds: string[];
		resetWatchApiUrl: string;
		bark: {
			enabled: boolean;
			pushUrl: string;
		};
		dingtalk: {
			enabled: boolean;
			webhookUrl: string;
			secret: string;
		};
	};
}

export type UsageRange = "cycle" | "24h" | "7d" | "30d" | "all";

export type UsageIdentityType =
	| "user"
	| "credential"
	| "codex_account"
	| "account_group";

export interface UsageIdentityFilter {
	identityType: UsageIdentityType;
	identityId: string;
}

export interface UsageFilters {
	downstream?: UsageIdentityFilter | null;
	upstream?: UsageIdentityFilter | null;
}

export interface UsageBounds {
	startAt: number;
	endAt: number;
}

export interface UsageTotals {
	requests: number;
	inputTokens: number;
	cachedInputTokens: number;
	cacheCreationInputTokens: number;
	outputTokens: number;
	reasoningOutputTokens: number;
	totalTokens: number;
	costUsd: number;
}

export interface UsageSeriesPoint extends UsageTotals {
	startAt: number;
	successfulRequests: number;
	failedRequests: number;
}

export interface UsageModelRow extends UsageTotals {
	model: string;
}

/** `value` is empty for attempts that failed before the request was read. */
export interface UsageDimensionRow extends UsageTotals {
	value: string;
}

export interface UsageIdentityRow extends UsageTotals {
	userId: string;
	userName: string;
}

export type UsageEventStatus = "completed" | "incomplete" | "failed" | "cancelled" | "interrupted";

/** Role of the request in its Codex session tree; empty when the client sent no session. */
export type UsageSessionKind = "" | "main" | "subagent" | "fork" | "compaction";

export interface UsageEvent extends Omit<UsageTotals, "requests"> {
	id: number;
	recordedAt: number;
	requestId: string;
	responseId: string;
	userId: string;
	userName: string;
	credentialType: CredentialType;
	credentialId: string;
	/** Credential label, or the last characters of its value. */
	credentialName: string;
	codexAccountId: string;
	codexAccountName: string;
	accountGroupId: string;
	accountGroupName: string;
	/** Model in the request; empty when the attempt failed before the request was read. */
	requestedModel: string;
	/** Model reported by the upstream, falling back to `requestedModel`. */
	model: string;
	transport: "http" | "websocket";
	endpoint: string;
	stream: boolean;
	status: UsageEventStatus;
	statusCode: number | null;
	errorCode: string;
	reasoningEffort: string;
	serviceTier: string;
	responseServiceTier: string;
	sessionId: string;
	parentSessionId: string;
	sessionKind: UsageSessionKind;
	clientIp: string;
	xForwardedFor: string;
	userAgent: string;
	latencyMs: number;
	/** Time to the first streamed output delta; null when none arrived. */
	ttftMs: number | null;
}

export interface UsageDashboard {
	range: UsageRange;
	startAt: number;
	endAt: number;
	totals: UsageTotals;
	series: UsageSeriesPoint[];
	models: UsageModelRow[];
	identities: UsageIdentityRow[];
	reasoningEfforts: UsageDimensionRow[];
	serviceTiers: UsageDimensionRow[];
	recentEvents: UsageEvent[];
	unpricedModels: string[];
}

// ---------------------------------------------------------------------------
// Analytics (GET /usage/analytics) and request log (GET /usage/events)
// ---------------------------------------------------------------------------

export type AnalyticsRange = "24h" | "7d" | "30d" | "all" | "custom";

export interface AnalyticsQuery {
	range: AnalyticsRange;
	/** Required with `custom`. */
	startAt?: number;
	endAt?: number;
	/** Bucket size in minutes; defaults by range length. */
	bucketMinutes?: number;
	downstream?: UsageIdentityFilter | null;
	upstream?: UsageIdentityFilter | null;
	models?: string[];
	statuses?: UsageEventStatus[];
	efforts?: string[];
	/** Requested service tiers: priority, default, auto, flex. */
	tiers?: string[];
	transports?: string[];
	sessionKinds?: string[];
	sessionId?: string;
	search?: string;
}

export type EventView = "all" | "failed" | "slow" | "downgraded" | "subagent" | "rate_limited";

export interface EventPageQuery extends AnalyticsQuery {
	view?: EventView;
	offset?: number;
	limit?: number;
	group?: "request" | "session";
}

export interface Percentiles {
	p50: number | null;
	p95: number | null;
	max: number | null;
	samples: number;
}

export interface OutcomeCounts {
	requests: number;
	completed: number;
	incomplete: number;
	failed: number;
	cancelled: number;
	interrupted: number;
	/** Requests answered with HTTP 429. */
	rateLimited: number;
}

export interface TokenCounts {
	inputTokens: number;
	cachedInputTokens: number;
	cacheCreationInputTokens: number;
	outputTokens: number;
	reasoningOutputTokens: number;
	totalTokens: number;
	costUsd: number;
}

export interface AnalyticsTotals extends OutcomeCounts, TokenCounts {
	unpricedTokens: number;
	unpricedRequests: number;
	fastRequests: number;
	/** Cost of Fast requests before the Fast multiplier. */
	fastBaseCostUsd: number;
	/** Fast requests served as Fast / with another tier. */
	fastFulfilled: number;
	fastDowngraded: number;
	sessions: number;
	ttft: Percentiles;
	latency: Percentiles;
	/** Median output tokens per second. */
	outputSpeed: number | null;
	firstAt: number | null;
	lastAt: number | null;
}

export interface AnalyticsBucket extends OutcomeCounts, TokenCounts {
	startAt: number;
	ttftP50: number | null;
	ttftP95: number | null;
	latencyP50: number | null;
	latencyP95: number | null;
	fastRequests: number;
	fastDowngraded: number;
}

export interface NamedCount {
	name: string;
	requests: number;
}

export interface ModelStats extends OutcomeCounts, TokenCounts {
	model: string;
	priced: boolean;
	ttft: Percentiles;
	latency: Percentiles;
	outputSpeed: number | null;
	outputSpeedFirstHalf: number | null;
	outputSpeedSecondHalf: number | null;
	/** Requested models that were answered by this model. */
	redirects: NamedCount[];
	firstAt: number;
	lastAt: number;
	/** Total tokens per seventh of the range. */
	trend: number[];
}

export interface IdentityStats extends OutcomeCounts, TokenCounts {
	userId: string;
	userName: string;
	sessions: number;
	lastAt: number;
	lastUserAgent: string;
	trend: number[];
}

export interface ConsumerShare {
	userId: string;
	userName: string;
	requests: number;
	totalTokens: number;
}

export interface UpstreamAccountStats extends OutcomeCounts, TokenCounts {
	codexAccountId: string;
	codexAccountName: string;
	accountGroupId: string;
	accountGroupName: string;
	ttft: Percentiles;
	latency: Percentiles;
	fastRequests: number;
	fastFulfilled: number;
	fastDowngraded: number;
	sessions: number;
	lastAt: number;
	/** HTTP 429 responses per series bucket. */
	rateLimitSeries: number[];
	trend: number[];
	consumers: ConsumerShare[];
}

export interface UpstreamGroupStats extends OutcomeCounts, TokenCounts {
	/** Empty for requests routed directly to an account. */
	accountGroupId: string;
	accountGroupName: string;
	sessions: number;
	sessionAffinityRate: number | null;
	trend: number[];
	consumers: ConsumerShare[];
	/** Usage of each member account through this group. */
	accounts: { codexAccountId: string; codexAccountName: string; requests: number; totalTokens: number; rateLimited: number }[];
}

export interface IdentityModelCell {
	userId: string;
	userName: string;
	model: string;
	requests: number;
	totalTokens: number;
	costUsd: number;
}

export interface EffortTierCell {
	effort: string;
	/** Tier the upstream answered with; empty when unreported. */
	tier: string;
	requests: number;
	totalTokens: number;
}

export interface FastBreakdown {
	name: string;
	requested: number;
	fulfilled: number;
	downgraded: number;
}

export interface FastStats {
	requested: number;
	fulfilled: number;
	downgraded: number;
	accounts: FastBreakdown[];
	models: FastBreakdown[];
}

export interface ErrorStats {
	status: UsageEventStatus;
	statusCode: number | null;
	errorCode: string;
	requests: number;
	lastAt: number;
	accounts: NamedCount[];
}

export interface SessionKindStats {
	kind: UsageSessionKind;
	requests: number;
	sessions: number;
}

export interface SessionSummary {
	sessionId: string;
	sessionKind: UsageSessionKind;
	parentSessionId: string;
	userName: string;
	requests: number;
	childSessions: number;
	totalTokens: number;
	costUsd: number;
	firstAt: number;
	lastAt: number;
}

export interface SessionStats {
	sessions: number;
	requestsWithSession: number;
	maxRequests: number;
	medianDurationMs: number | null;
	kinds: SessionKindStats[];
	recent: SessionSummary[];
}

export interface ClientStats {
	client: string;
	version: string;
	os: string;
	requests: number;
	failed: number;
	websocketRequests: number;
	ttft: Percentiles;
	lastAt: number;
}

export interface IpStats {
	ip: string;
	forwardedFor: string;
	requests: number;
	lastAt: number;
}

export interface ClientOverview {
	clients: ClientStats[];
	distinctClients: number;
	ips: IpStats[];
	distinctIps: number;
	forwardedRequests: number;
}

export interface TransportStats extends OutcomeCounts {
	name: string;
	ttft: Percentiles;
	latency: Percentiles;
	averageInputTokens: number;
}

export interface TransportOverview {
	/** `http` / `websocket`. */
	transports: TransportStats[];
	/** `stream` / `non_stream`. */
	streaming: TransportStats[];
	endpoints: TransportStats[];
}

export interface LatencyDensity {
	ttftEdgesMs: number[];
	latencyEdgesMs: number[];
	/** `cells[latencyBin][ttftBin]`. */
	cells: number[][];
	missingTtft: number;
}

export interface UnpricedModel {
	model: string;
	requests: number;
	totalTokens: number;
	firstAt: number;
	lastAt: number;
}

export interface UsageAnalytics {
	range: AnalyticsRange;
	startAt: number;
	endAt: number;
	bucketMs: number;
	totals: AnalyticsTotals;
	/** Equally long period right before, up to the same elapsed time. */
	previous: AnalyticsTotals | null;
	series: AnalyticsBucket[];
	models: ModelStats[];
	identities: IdentityStats[];
	upstreamAccounts: UpstreamAccountStats[];
	upstreamGroups: UpstreamGroupStats[];
	identityModels: IdentityModelCell[];
	effortTiers: EffortTierCell[];
	fast: FastStats;
	errors: ErrorStats[];
	sessions: SessionStats;
	clients: ClientOverview;
	transports: TransportOverview;
	latencyDensity: LatencyDensity;
	unpricedModels: UnpricedModel[];
	recentFailures: UsageEvent[];
}

export interface EventViewCounts {
	all: number;
	failed: number;
	slow: number;
	downgraded: number;
	subagent: number;
	rateLimited: number;
}

/** `[recordedAt, latencyMs, status]` */
export type TimelinePoint = [number, number, UsageEventStatus];

export interface SessionChild extends OutcomeCounts {
	sessionId: string;
	sessionKind: UsageSessionKind;
	totalTokens: number;
	costUsd: number;
	firstAt: number;
	lastAt: number;
	accounts: string[];
	fastDowngraded: number;
	timeline: TimelinePoint[];
}

export interface SessionGroup extends OutcomeCounts {
	/** Root session; empty for requests without a session (grouped per identity). */
	sessionId: string;
	userId: string;
	userName: string;
	accountGroupName: string;
	accounts: string[];
	userAgent: string;
	totalTokens: number;
	costUsd: number;
	firstAt: number;
	lastAt: number;
	kinds: Partial<Record<UsageSessionKind, number>>;
	timeline: TimelinePoint[];
	children: SessionChild[];
	/** Latest requests of the root session itself. */
	events: UsageEvent[];
	eventCount: number;
}

export interface UsageEventPage {
	startAt: number;
	endAt: number;
	bucketMs: number;
	/** Requests in the range before filters and views. */
	rangeTotal: number;
	/** Matching requests, or session groups when grouped. */
	total: number;
	offset: number;
	summary: AnalyticsTotals;
	series: AnalyticsBucket[];
	views: EventViewCounts;
	slowThresholdMs: number | null;
	events: UsageEvent[];
	sessions: SessionGroup[] | null;
	/** Models seen in the range (filter options). */
	models: string[];
	/** Requests per session kind for each root session on this page, across the range. */
	sessionCounts: Record<string, Partial<Record<UsageSessionKind, number>>>;
}

// ---------------------------------------------------------------------------
// Quota forecasts (GET /quota)
// ---------------------------------------------------------------------------

export interface WindowForecast {
	id: string;
	category: QuotaCategory;
	name: string;
	kind: QuotaWindowKind;
	usedPercent: number | null;
	remainingPercent: number | null;
	limitWindowSeconds: number | null;
	resetAt: number | null;
	limitReached: boolean;
	windowStartAt: number | null;
	/** Usage recorded on the account since the window started. */
	requests: number;
	totalTokens: number;
	costUsd: number;
	tokensPerPercent: number | null;
	costPerPercent: number | null;
	burnPercentPerHour: number | null;
	exhaustAt: number | null;
	exhaustsBeforeReset: boolean;
}

export interface AccountQuota {
	accountId: string;
	accountName: string;
	enabled: boolean;
	/** Taken out of routing after a 429 or an exhausted window. */
	quarantined: boolean;
	sampledAt: number | null;
	planType: string | null;
	windows: WindowForecast[];
}

export interface GroupShares {
	groupId: string;
	members: { accountId: string; share: number }[];
}

export interface QuotaOverview {
	sampledAt: number | null;
	accounts: AccountQuota[];
	groups: GroupShares[];
}

export interface PricingSyncStatus {
	syncedAt: number;
	source: string;
	sourceUrl: string;
	matchedModels: string[];
	unmatchedModels: string[];
	updatedModels: string[];
	/** Models with hand-edited prices that the sync left unchanged. */
	skippedModels: string[];
	/** Prices as written by the sync; a different current price was edited by hand. */
	syncedPrices: ModelPrice[];
}

export type NotificationChannel = "bark" | "dingtalk";

/** When each model and identity was last used, across all recorded events. */
export interface LastSeen {
	models: { model: string; firstAt: number; lastAt: number }[];
	identities: { userId: string; lastAt: number; userAgent: string }[];
	credentials: { credentialId: string; lastAt: number; userAgent: string }[];
}

/** Query string shared by analytics and the request log. */
export function analyticsSearchParams(query: AnalyticsQuery | EventPageQuery): URLSearchParams {
	const params = new URLSearchParams({ range: query.range });
	if (query.startAt !== undefined) params.set("startAt", String(Math.floor(query.startAt)));
	if (query.endAt !== undefined) params.set("endAt", String(Math.ceil(query.endAt)));
	if (query.bucketMinutes) params.set("bucket", String(query.bucketMinutes));
	if (query.downstream) {
		params.set("downstreamType", query.downstream.identityType);
		params.set("downstreamId", query.downstream.identityId);
	}
	if (query.upstream) {
		params.set("upstreamType", query.upstream.identityType);
		params.set("upstreamId", query.upstream.identityId);
	}
	const lists: [string, string[] | undefined][] = [
		["model", query.models],
		["status", query.statuses],
		["effort", query.efforts],
		["tier", query.tiers],
		["transport", query.transports],
		["sessionKind", query.sessionKinds],
	];
	for (const [name, values] of lists) for (const value of values ?? []) params.append(name, value);
	if (query.sessionId) params.set("sessionId", query.sessionId);
	if (query.search?.trim()) params.set("q", query.search.trim());
	if ("view" in query && query.view && query.view !== "all") params.set("view", query.view);
	if ("offset" in query && query.offset) params.set("offset", String(query.offset));
	if ("limit" in query && query.limit) params.set("limit", String(query.limit));
	if ("group" in query && query.group === "session") params.set("group", "session");
	return params;
}

export interface ModelPrice {
	model: string;
	input: number;
	output: number;
	cacheRead: number;
	cacheWrite: number;
	multiplier: number;
}

export interface PricingResponse {
	prices: ModelPrice[];
	usedModels: string[];
	sync: PricingSyncStatus | null;
}

export interface PricingSyncResult {
	source: string;
	sourceUrl: string;
	prices: ModelPrice[];
	matchedModels: string[];
	unmatchedModels: string[];
	sync: PricingSyncStatus;
}

export interface DeviceAuthorization {
	verificationUri: string;
	userCode: string;
	expiresIn: number;
	interval: number;
	state: string;
}

export interface CodexAccountDeviceAuthorization {
	accountId: string;
	authorization: DeviceAuthorization;
}

export type CodexAccountDevicePollResult =
	| { status: "pending"; retryAfter: number }
	| { status: "stored"; account: CodexAccount };

export class AdminApiError extends Error {
	readonly status: number;
	readonly code: string | undefined;

	constructor(message: string, status: number, code?: string) {
		super(message);
		this.name = "AdminApiError";
		this.status = status;
		this.code = code;
	}
}

export class AdminSessionExpiredError extends AdminApiError {
	constructor() {
		super("登录已失效，请重新登录。", 401, "invalid_session");
		this.name = "AdminSessionExpiredError";
	}
}

export class AdminApiClient {
	readonly basePath: string;

	constructor(basePath: string) {
		this.basePath = basePath.replace(/\/$/, "");
	}

	getState(): Promise<AdminState> {
		return this.requestJson<AdminState>("/state");
	}

	getSettings(): Promise<AdminSettings> {
		return this.requestJson<AdminSettings>("/settings");
	}

	replaceSettings(settings: AdminSettings): Promise<AdminSettings> {
		return this.requestJson<AdminSettings>(
			"/settings",
			jsonRequest("PUT", settings),
		);
	}

	async getCodexAccountSubscription(id: string): Promise<SubscriptionInfo> {
		const query = new URLSearchParams({ id });
		const value = await this.requestJson<{ subscription: SubscriptionInfo }>(
			`/codex-accounts/subscription?${query.toString()}`,
		);
		return value.subscription;
	}

	startCodexAccountDeviceAuthorization(): Promise<CodexAccountDeviceAuthorization> {
		return this.requestJson<CodexAccountDeviceAuthorization>(
			"/codex-accounts/oauth/device",
			{ method: "POST" },
		);
	}

	pollCodexAccountDeviceAuthorization(
		accountId: string,
		state: string,
	): Promise<CodexAccountDevicePollResult> {
		return this.requestJson<CodexAccountDevicePollResult>(
			"/codex-accounts/oauth/device/poll",
			jsonRequest("POST", { accountId, state }),
		);
	}

	async updateCodexAccount(
		id: string,
		value: CodexAccountUpdate,
	): Promise<CodexAccount[]> {
		const result = await this.requestJson<{ codexAccounts: CodexAccount[] }>(
			"/codex-accounts",
			jsonRequest("PUT", { id, ...value }),
		);
		return result.codexAccounts;
	}

	async importCodexCredentials(value: CodexCredentialImport): Promise<CodexAccount> {
		const result = await this.requestJson<{ account: CodexAccount }>(
			"/codex-accounts/import",
			jsonRequest("POST", value),
		);
		return result.account;
	}

	async deleteCodexAccount(id: string): Promise<CodexAccount[]> {
		const result = await this.requestJson<{ codexAccounts: CodexAccount[] }>(
			"/codex-accounts",
			jsonRequest("DELETE", { id }),
		);
		return result.codexAccounts;
	}

	getAccountRouting(): Promise<AccountRoutingConfiguration> {
		return this.requestJson<AccountRoutingConfiguration>("/account-routing");
	}

	updateAccountRouting(
		accountGroups: AccountGroup[],
		routes: RouteAssignment[],
	): Promise<AccountRoutingConfiguration> {
		return this.requestJson<AccountRoutingConfiguration>(
			"/account-routing",
			jsonRequest("PUT", { groups: accountGroups, routes }),
		);
	}

	getUsage(
		range: UsageRange,
		filters: UsageFilters = {},
		bounds: UsageBounds | null = null,
	): Promise<UsageDashboard> {
		const query = new URLSearchParams({ range });
		if (filters.downstream) {
			query.set("downstreamType", filters.downstream.identityType);
			query.set("downstreamId", filters.downstream.identityId);
		}
		if (filters.upstream) {
			query.set("upstreamType", filters.upstream.identityType);
			query.set("upstreamId", filters.upstream.identityId);
		}
		if (bounds) {
			query.set("startAt", String(bounds.startAt));
			query.set("endAt", String(bounds.endAt));
		}
		return this.requestJson<UsageDashboard>(`/usage?${query.toString()}`);
	}

	getAnalytics(query: AnalyticsQuery, signal?: AbortSignal): Promise<UsageAnalytics> {
		return this.requestJson<UsageAnalytics>(`/usage/analytics?${analyticsSearchParams(query).toString()}`, { signal: signal ?? null });
	}

	getEvents(query: EventPageQuery, signal?: AbortSignal): Promise<UsageEventPage> {
		return this.requestJson<UsageEventPage>(`/usage/events?${analyticsSearchParams(query).toString()}`, { signal: signal ?? null });
	}

	getLastSeen(signal?: AbortSignal): Promise<LastSeen> {
		return this.requestJson<LastSeen>("/usage/last-seen", { signal: signal ?? null });
	}

	/** URL of the CSV export of the request log matching `query`. */
	eventsCsvUrl(query: EventPageQuery): string {
		const params = analyticsSearchParams({ ...query, offset: undefined, limit: undefined, group: undefined });
		params.set("format", "csv");
		return `${this.basePath}/usage/events?${params.toString()}`;
	}

	/**
	 * Cached quota snapshots with forecasts. `refresh` samples every enabled
	 * account first, or only the account whose id is given.
	 */
	getQuota(refresh: boolean | string = false): Promise<QuotaOverview> {
		if (refresh === false) return this.requestJson<QuotaOverview>("/quota");
		const target = refresh === true ? "1" : refresh;
		return this.requestJson<QuotaOverview>(`/quota?refresh=${encodeURIComponent(target)}`);
	}

	async testNotification(channel: NotificationChannel): Promise<void> {
		await this.requestJson<{ sent: boolean }>("/settings/notifications/test", jsonRequest("POST", { channel }));
	}

	getPricing(): Promise<PricingResponse> {
		return this.requestJson<PricingResponse>("/pricing");
	}

	async replacePricing(prices: ModelPrice[]): Promise<ModelPrice[]> {
		const result = await this.requestJson<{ prices: ModelPrice[] }>(
			"/pricing",
			jsonRequest("PUT", { prices }),
		);
		return result.prices;
	}

	syncPricing(): Promise<PricingSyncResult> {
		return this.requestJson<PricingSyncResult>("/pricing/sync", {
			method: "POST",
		});
	}

	async createUser(value: UserInput): Promise<User[]> {
		const result = await this.requestJson<{ users: User[] }>("/users", jsonRequest("POST", value));
		return result.users;
	}

	async updateUser(id: string, value: UserInput): Promise<User[]> {
		const result = await this.requestJson<{ users: User[] }>("/users", jsonRequest("PUT", { id, ...value }));
		return result.users;
	}

	async deleteUser(id: string): Promise<User[]> {
		const result = await this.requestJson<{ users: User[] }>("/users", jsonRequest("DELETE", { id }));
		return result.users;
	}

	private async requestJson<T>(path: string, init?: RequestInit): Promise<T> {
		const headers = new Headers(init?.headers);
		headers.set("Accept", "application/json");
		const response = await fetch(`${this.basePath}${path}`, {
			...init,
			credentials: "same-origin",
			headers,
		});
		if (!response.ok) throw await responseError(response, true);
		try {
			return (await response.json()) as T;
		} catch {
			throw invalidPayload();
		}
	}
}

function jsonRequest(method: string, value: unknown): RequestInit {
	return {
		method,
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify(value),
	};
}

async function responseError(
	response: Response,
	sessionRequired: boolean,
): Promise<AdminApiError> {
	if (sessionRequired && response.status === 401) {
		return new AdminSessionExpiredError();
	}

	let payload: unknown = null;
	try {
		payload = await response.json();
	} catch {}
	const error = isRecord(payload) && isRecord(payload.error) ? payload.error : null;
	// The browser's session was replaced by a non-administrator one (for example
	// from another tab): leave the management view like an expired session.
	if (sessionRequired && response.status === 403 && error?.code === "admin_required") {
		return new AdminSessionExpiredError();
	}
	const message =
		error && typeof error.message === "string"
			? error.message
			: "管理请求失败，请稍后重试。";
	const code = error && typeof error.code === "string" ? error.code : undefined;
	return new AdminApiError(message, response.status, code);
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function invalidPayload(): AdminApiError {
	return new AdminApiError(
		"管理服务返回了无法识别的数据。",
		502,
		"invalid_admin_response",
	);
}
