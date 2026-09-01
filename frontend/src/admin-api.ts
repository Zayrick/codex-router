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

export interface ClientApiKeyInput {
	name: string;
	key: string;
	enabled: boolean;
}

export interface ClientApiKey extends ClientApiKeyInput {
	id: string;
}

export interface AuthProxyAccountInput {
	name: string;
	accountId: string;
	enabled: boolean;
}

export interface AuthProxyAccount extends AuthProxyAccountInput {
	id: string;
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

export type RouteConsumerType = "api_key" | "auth_proxy";
export type RouteTargetType = "account" | "group";

export interface RouteAssignment {
	consumerType: RouteConsumerType;
	consumerId: string;
	targetType: RouteTargetType;
	targetId: string;
}

export interface AccountRoutingConfiguration {
	accountGroups: AccountGroup[];
	routes: RouteAssignment[];
}

export interface AdminState extends AccountRoutingConfiguration {
	codexAccounts: CodexAccount[];
	apiKeys: ClientApiKey[];
	authProxyAccounts: AuthProxyAccount[];
}

export type UsageRange = "cycle" | "24h" | "7d" | "30d" | "all";

export type UsageIdentityType =
	| "api_key"
	| "auth_proxy"
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

export interface UsageIdentityRow extends UsageTotals {
	identityType: "api_key" | "auth_proxy";
	identityId: string;
	identityName: string;
}

export interface UsageEvent extends Omit<UsageTotals, "requests"> {
	id: number;
	recordedAt: number;
	identityType: "api_key" | "auth_proxy";
	identityId: string;
	identityName: string;
	codexAccountId: string;
	codexAccountName: string;
	accountGroupId: string;
	accountGroupName: string;
	model: string;
	transport: "http" | "websocket";
	endpoint: string;
	status: "completed" | "incomplete" | "failed";
}

export interface UsageDashboard {
	range: UsageRange;
	startAt: number;
	endAt: number;
	totals: UsageTotals;
	series: UsageSeriesPoint[];
	models: UsageModelRow[];
	identities: UsageIdentityRow[];
	recentEvents: UsageEvent[];
	unpricedModels: string[];
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
}

export interface PricingSyncResult {
	source: string;
	sourceUrl: string;
	prices: ModelPrice[];
	matchedModels: string[];
	unmatchedModels: string[];
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
		super("管理会话已失效，请重新登录。", 401, "invalid_admin_session");
		this.name = "AdminSessionExpiredError";
	}
}

export class AdminApiClient {
	readonly basePath: string;

	constructor(basePath: string) {
		this.basePath = basePath.replace(/\/$/, "");
	}

	async login(secret: string): Promise<void> {
		await this.submitSessionForm(
			"/login",
			new URLSearchParams({ secret }),
			false,
		);
	}

	async logout(): Promise<void> {
		await this.submitSessionForm("/logout", undefined, true);
	}

	getState(): Promise<AdminState> {
		return this.requestJson<AdminState>("/state");
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

	async createApiKey(value: ClientApiKeyInput): Promise<ClientApiKey[]> {
		const result = await this.requestJson<{ apiKeys: ClientApiKey[] }>(
			"/api-keys",
			jsonRequest("POST", value),
		);
		return result.apiKeys;
	}

	async updateApiKey(
		id: string,
		value: ClientApiKeyInput,
	): Promise<ClientApiKey[]> {
		const result = await this.requestJson<{ apiKeys: ClientApiKey[] }>(
			"/api-keys",
			jsonRequest("PUT", { id, ...value }),
		);
		return result.apiKeys;
	}

	async deleteApiKey(id: string): Promise<ClientApiKey[]> {
		const result = await this.requestJson<{ apiKeys: ClientApiKey[] }>(
			"/api-keys",
			jsonRequest("DELETE", { id }),
		);
		return result.apiKeys;
	}

	async createAuthProxyAccount(
		value: AuthProxyAccountInput,
	): Promise<AuthProxyAccount[]> {
		const result = await this.requestJson<{ authProxyAccounts: AuthProxyAccount[] }>(
			"/auth-proxy",
			jsonRequest("POST", value),
		);
		return result.authProxyAccounts;
	}

	async updateAuthProxyAccount(
		id: string,
		value: AuthProxyAccountInput,
	): Promise<AuthProxyAccount[]> {
		const result = await this.requestJson<{ authProxyAccounts: AuthProxyAccount[] }>(
			"/auth-proxy",
			jsonRequest("PUT", { id, ...value }),
		);
		return result.authProxyAccounts;
	}

	async deleteAuthProxyAccount(id: string): Promise<AuthProxyAccount[]> {
		const result = await this.requestJson<{ authProxyAccounts: AuthProxyAccount[] }>(
			"/auth-proxy",
			jsonRequest("DELETE", { id }),
		);
		return result.authProxyAccounts;
	}

	private async submitSessionForm(
		path: string,
		body: URLSearchParams | undefined,
		sessionRequired: boolean,
	): Promise<void> {
		const headers = new Headers({ Accept: "application/json" });
		const init: RequestInit = {
			method: "POST",
			credentials: "same-origin",
			headers,
			...(body ? { body } : {}),
		};
		const response = await fetch(`${this.basePath}${path}`, init);
		if (response.ok) return;
		throw await responseError(response, sessionRequired);
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
