export interface OAuthStatus {
	email: string | null;
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
	oauth: OAuthStatus | null;
}

export interface AdminState {
	oauth: OAuthStatus | null;
	subscription: SubscriptionMetadata | null;
	apiKeys: ClientApiKey[];
	authProxyAccounts: AuthProxyAccount[];
}

export interface DeviceAuthorization {
	verificationUri: string;
	userCode: string;
	expiresIn: number;
	interval: number;
	state: string;
}

export type DevicePollResult =
	| { status: "pending"; retryAfter: number }
	| {
			status: "stored";
			oauth: OAuthStatus;
			subscription: SubscriptionMetadata;
	  };

export type AuthProxyDevicePollResult =
	| { status: "pending"; retryAfter: number }
	| { status: "stored"; oauth: OAuthStatus };

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

	async getSubscription(): Promise<SubscriptionInfo> {
		const value = await this.requestJson<{ subscription: SubscriptionInfo }>(
			"/subscription",
		);
		return value.subscription;
	}

	startDeviceAuthorization(): Promise<DeviceAuthorization> {
		return this.requestJson<DeviceAuthorization>(
			"/oauth/device",
			{ method: "POST" },
		);
	}

	pollDeviceAuthorization(state: string): Promise<DevicePollResult> {
		return this.requestJson<DevicePollResult>(
			"/oauth/device/poll",
			jsonRequest("POST", { state }),
		);
	}

	async removeOAuth(): Promise<void> {
		await this.requestJson<{ oauth: null }>(
			"/oauth",
			{ method: "DELETE" },
		);
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

	async createAuthProxyAccount(value: AuthProxyAccountInput): Promise<AuthProxyAccount[]> {
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

	startAuthProxyDeviceAuthorization(id: string): Promise<DeviceAuthorization> {
		return this.requestJson<DeviceAuthorization>(
			"/auth-proxy/oauth/device",
			jsonRequest("POST", { id }),
		);
	}

	pollAuthProxyDeviceAuthorization(
		id: string,
		state: string,
	): Promise<AuthProxyDevicePollResult> {
		return this.requestJson<AuthProxyDevicePollResult>(
			"/auth-proxy/oauth/device/poll",
			jsonRequest("POST", { id, state }),
		);
	}

	async removeAuthProxyOAuth(id: string): Promise<void> {
		await this.requestJson<{ oauth: null }>(
			"/auth-proxy/oauth",
			jsonRequest("DELETE", { id }),
		);
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

	private async requestJson<T>(
		path: string,
		init?: RequestInit,
	): Promise<T> {
		const headers = new Headers(init?.headers);
		headers.set("Accept", "application/json");
		const response = await fetch(`${this.basePath}${path}`, {
			...init,
			credentials: "same-origin",
			headers,
		});
		if (!response.ok) throw await responseError(response, true);
		let payload: unknown;
		try {
			payload = await response.json();
		} catch {
			throw invalidPayload();
		}
		return payload as T;
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
	const message = error && typeof error.message === "string"
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
