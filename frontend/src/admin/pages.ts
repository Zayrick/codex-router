// Information architecture v2: 5 groups, 10 pages (Figma "信息架构 v2").

export type ManagementPage =
	| "overview"
	| "logs"
	| "usage"
	| "performance"
	| "codex"
	| "groups"
	| "users"
	| "pricing"
	| "settings";

export interface PageMeta {
	page: ManagementPage;
	/** Sidebar label. */
	label: string;
	/** Page heading. */
	title: string;
	eyebrow: string | null;
	description: string;
}

export const PAGE_META: Record<ManagementPage, PageMeta> = {
	overview: { page: "overview", label: "概览", title: "运行概览", eyebrow: "DASHBOARD", description: "今天的流量、健康与待处理事项。" },
	logs: { page: "logs", label: "调用日志", title: "调用日志", eyebrow: "OBSERVABILITY", description: "逐条查看每次请求的路由、耗时、Token 与错误。" },
	usage: { page: "usage", label: "用量分析", title: "用量分析", eyebrow: "OBSERVABILITY", description: "按用户、上游路由和模型分析请求、Token 与成本。" },
	performance: { page: "performance", label: "性能与可靠性", title: "性能与可靠性", eyebrow: "OBSERVABILITY", description: "按模型和上游账户定位延迟、失败与限流问题。" },
	codex: { page: "codex", label: "Codex 账户", title: "Codex 账户", eyebrow: "SCHEDULING", description: "管理 Codex 凭据，查看额度与耗尽预测。" },
	groups: { page: "groups", label: "账户组", title: "账户组", eyebrow: "SCHEDULING", description: "将 Codex 账户组成调度池，再分配给用户。" },
	users: { page: "users", label: "用户", title: "用户", eyebrow: "USERS", description: "管理用户及其 API Key、账户 ID 与路由。" },
	pricing: { page: "pricing", label: "模型价格", title: "模型价格", eyebrow: "CONFIGURATION", description: "配置模型单价，用于成本统计。" },
	settings: { page: "settings", label: "设置", title: "设置", eyebrow: null, description: "管理用户端展示、计费倍率与通知。" },
};

export const NAV_GROUPS: { label: string; pages: ManagementPage[] }[] = [
	{ label: "运行", pages: ["overview", "logs"] },
	{ label: "分析", pages: ["usage", "performance"] },
	{ label: "调度", pages: ["codex", "groups"] },
	{ label: "用户", pages: ["users"] },
	{ label: "配置", pages: ["pricing", "settings"] },
];

const LEGACY_PAGES: Record<string, ManagementPage> = {
	account: "codex",
	routing: "groups",
	accounts: "users",
	"api-keys": "users",
	downstream: "users",
};

export function pageFromSearch(search: string): ManagementPage {
	const value = new URLSearchParams(search).get("page") ?? "";
	if (value in PAGE_META) return value as ManagementPage;
	return LEGACY_PAGES[value] ?? "overview";
}

export function pageHref(basePath: string, page: ManagementPage, params?: Record<string, string>): string {
	const search = new URLSearchParams(params);
	if (page !== "overview") search.set("page", page);
	const query = search.toString();
	return query ? `${basePath}?${query}` : basePath;
}
