// View model shared by the API Keys and 下游账户 pages.

import type { AccountGroup, AuthProxyAccount, ClientApiKey, CodexAccount, OutcomeCounts, RouteAssignment } from "@/admin-api";

export type IdentityKind = "api_key" | "auth_proxy";

export interface IdentityEntry {
	kind: IdentityKind;
	id: string;
	name: string;
	/** API key or account_id. */
	credential: string;
	enabled: boolean;
	raw: ClientApiKey | AuthProxyAccount;
}

export function apiKeyEntries(keys: ClientApiKey[]): IdentityEntry[] {
	return keys.map((key) => ({ kind: "api_key", id: key.id, name: key.name, credential: key.key, enabled: key.enabled, raw: key }));
}

export function proxyEntries(accounts: AuthProxyAccount[]): IdentityEntry[] {
	return accounts.map((account) => ({
		kind: "auth_proxy",
		id: account.id,
		name: account.name,
		credential: account.accountId,
		enabled: account.enabled,
		raw: account,
	}));
}

export interface RouteInfo {
	type: "group" | "account" | "unassigned";
	label: string;
	group: AccountGroup | null;
}

export function routeInfo(routes: RouteAssignment[], groups: AccountGroup[], accounts: CodexAccount[], entry: IdentityEntry): RouteInfo {
	const route = routes.find((item) => item.consumerType === entry.kind && item.consumerId === entry.id);
	if (!route) return { type: "unassigned", label: "未分配", group: null };
	if (route.targetType === "group") {
		const group = groups.find((item) => item.id === route.targetId) ?? null;
		return { type: "group", label: group?.name ?? "账户组已移除", group };
	}
	return { type: "account", label: accounts.find((item) => item.id === route.targetId)?.name ?? "账户已移除", group: null };
}

/** Successful share of requests that finished (cancellations excluded). */
export function successRate(outcomes: OutcomeCounts): number | null {
	const success = outcomes.completed + outcomes.incomplete;
	const total = success + outcomes.failed + outcomes.interrupted;
	return total > 0 ? success / total : null;
}

/** "codex_cli_rs 0.63.0 · Ubuntu" from a raw User-Agent. */
export function clientSummary(userAgent: string): string {
	const value = userAgent.trim();
	if (!value) return "";
	const product = value.split(/\s+/)[0] ?? "";
	const [client, version] = product.split("/");
	const os = /\(([^;)]+)/.exec(value)?.[1]?.trim();
	return [version ? `${client} ${version}` : client, os].filter(Boolean).join(" · ");
}
