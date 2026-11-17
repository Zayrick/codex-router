// Unified console session: one sign-in for administrators and users (with any of their credentials).

export const CONSOLE_API_BASE = "/admin/api";

export interface ConsoleSession {
	role: "admin" | "user";
	createdAt: number;
	expiresAt: number;
	absoluteExpiresAt: number;
}

export class SessionRequestError extends Error {
	readonly status: number;
	readonly code: string | undefined;

	constructor(message: string, status: number, code?: string) {
		super(message);
		this.name = "SessionRequestError";
		this.status = status;
		this.code = code;
	}
}

export async function fetchSession(): Promise<ConsoleSession | null> {
	const response = await fetch(`${CONSOLE_API_BASE}/session`, {
		credentials: "same-origin",
		headers: { Accept: "application/json" },
	});
	if (!response.ok) throw await sessionError(response, "暂时无法读取登录状态，请稍后重试。");
	return parseSession(await response.json());
}

export async function signIn(credential: string): Promise<ConsoleSession> {
	const response = await fetch(`${CONSOLE_API_BASE}/session`, {
		method: "POST",
		credentials: "same-origin",
		headers: { Accept: "application/json", "Content-Type": "application/json" },
		body: JSON.stringify({ credential }),
	});
	if (!response.ok) throw await sessionError(response, "登录失败，请稍后重试。");
	const session = parseSession(await response.json());
	if (!session) throw new SessionRequestError("登录服务返回了无法识别的数据。", 502);
	return session;
}

export async function signOut(): Promise<void> {
	const response = await fetch(`${CONSOLE_API_BASE}/session`, {
		method: "DELETE",
		credentials: "same-origin",
		headers: { Accept: "application/json", "Content-Type": "application/json" },
	});
	if (!response.ok && response.status !== 401) {
		throw await sessionError(response, "退出登录失败，请稍后重试。");
	}
}

function parseSession(payload: unknown): ConsoleSession | null {
	if (!isRecord(payload) || !isRecord(payload.session)) return null;
	const session = payload.session;
	if (session.role !== "admin" && session.role !== "user") return null;
	return session as unknown as ConsoleSession;
}

async function sessionError(response: Response, fallback: string): Promise<SessionRequestError> {
	let payload: unknown = null;
	try {
		payload = await response.json();
	} catch {}
	const error = isRecord(payload) && isRecord(payload.error) ? payload.error : null;
	const code = error && typeof error.code === "string" ? error.code : undefined;
	return new SessionRequestError(sessionErrorMessage(response, code, fallback), response.status, code);
}

function sessionErrorMessage(response: Response, code: string | undefined, fallback: string): string {
	switch (code) {
		case "invalid_credentials":
			return "凭据无效或已停用，请检查后重试。";
		case "too_many_sign_in_attempts": {
			const seconds = Number(response.headers.get("Retry-After"));
			const minutes = Number.isFinite(seconds) && seconds > 0 ? Math.ceil(seconds / 60) : null;
			return minutes ? `失败次数过多，请 ${minutes} 分钟后再试。` : "失败次数过多，请稍后再试。";
		}
		case "insecure_admin_origin":
			return "管理员登录需要通过 HTTPS 访问。";
		case "invalid_origin":
			return "请求来源与服务配置的 public_origin 不一致。";
		default:
			return fallback;
	}
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}
