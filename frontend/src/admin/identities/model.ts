// View model of the 用户 page.

import type { AccountGroup, CodexAccount, CredentialType, OutcomeCounts, RouteAssignment, User } from "@/admin-api";

export const CREDENTIAL_LABEL: Record<CredentialType, string> = { api_key: "API Key", account_id: "账户 ID" };

/** "2 个 API Key · 1 个账户 ID". */
export function credentialSummary(user: User): string {
	return (["api_key", "account_id"] as const)
		.map((type) => {
			const count = user.credentials.filter((credential) => credential.type === type).length;
			return count ? `${count} 个${CREDENTIAL_LABEL[type]}` : null;
		})
		.filter(Boolean)
		.join(" · ");
}

export interface RouteInfo {
	type: "group" | "account" | "unassigned";
	label: string;
	group: AccountGroup | null;
}

export function routeInfo(routes: RouteAssignment[], groups: AccountGroup[], accounts: CodexAccount[], userId: string): RouteInfo {
	const route = routes.find((item) => item.userId === userId);
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
