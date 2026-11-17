// Columns of the request table; time and expand are always shown.

export type ColumnKey =
	| "identity"
	| "model"
	| "upstream"
	| "session"
	| "status"
	| "latency"
	| "tokens"
	| "cost"
	| "requestId"
	| "client"
	| "ip"
	| "endpoint";

export const COLUMNS: { key: ColumnKey; label: string; defaultVisible: boolean; adminOnly?: boolean }[] = [
	{ key: "identity", label: "用户", defaultVisible: true, adminOnly: true },
	{ key: "model", label: "模型 · 强度 / 档位", defaultVisible: true },
	{ key: "upstream", label: "上游路由", defaultVisible: true },
	{ key: "session", label: "会话 · 传输", defaultVisible: true },
	{ key: "status", label: "状态", defaultVisible: true },
	{ key: "latency", label: "首字 · 总延迟", defaultVisible: true },
	{ key: "tokens", label: "Token", defaultVisible: true },
	{ key: "cost", label: "成本", defaultVisible: true },
	{ key: "requestId", label: "请求 ID", defaultVisible: false },
	{ key: "client", label: "客户端", defaultVisible: false },
	{ key: "ip", label: "客户端 IP", defaultVisible: false },
	{ key: "endpoint", label: "端点", defaultVisible: false },
];
