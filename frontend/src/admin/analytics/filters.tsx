import type { UsageIdentityFilter, UsageIdentityType } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { FilterMenu } from "@/components/app/filter-menu";

import type { AdminAnalyticsFilters } from "./filter-state";


const encode = (filter: UsageIdentityFilter | null) => (filter ? [`${filter.identityType}:${filter.identityId}`] : []);
function decode(values: string[]): UsageIdentityFilter | null {
	const value = values[0];
	if (!value) return null;
	const index = value.indexOf(":");
	return { identityType: value.slice(0, index) as UsageIdentityType, identityId: value.slice(index + 1) };
}

/** Toolbar filters: 用户 / 上游路由 / 模型 / 传输方式. */
export function AnalyticsFilterMenus({
	filters,
	onChange,
	show = ["downstream", "upstream", "model"],
}: {
	filters: AdminAnalyticsFilters;
	onChange: (filters: AdminAnalyticsFilters) => void;
	show?: ("downstream" | "upstream" | "model" | "transport")[];
}) {
	const { state, pricing } = useAdmin();
	return (
		<>
			{show.includes("downstream") ? (
				<FilterMenu
					align="end"
					allLabel="全部"
					label="用户"
					multiple={false}
					onChange={(values) => onChange({ ...filters, downstream: decode(values) })}
					options={state.users.map((user) => ({ value: `user:${user.id}`, label: user.name, text: user.name }))}
					selected={encode(filters.downstream)}
				/>
			) : null}
			{show.includes("upstream") ? (
				<FilterMenu
					align="end"
					allLabel="全部"
					label="上游路由"
					multiple={false}
					onChange={(values) => onChange({ ...filters, upstream: decode(values) })}
					options={[
						...state.accountGroups.map((group) => ({ value: `account_group:${group.id}`, label: group.name, text: group.name, hint: "账户组" })),
						...state.codexAccounts.map((account) => ({ value: `codex_account:${account.id}`, label: account.name, text: account.name, hint: "Codex 账户" })),
					]}
					selected={encode(filters.upstream)}
				/>
			) : null}
			{show.includes("model") ? (
				<FilterMenu
					align="end"
					label="模型"
					onChange={(models) => onChange({ ...filters, models })}
					options={pricing.usedModels.map((model) => ({ value: model, label: <span className="font-mono">{model}</span>, text: model }))}
					selected={filters.models}
				/>
			) : null}
			{show.includes("transport") ? (
				<FilterMenu
					align="end"
					label="传输"
					onChange={(transports) => onChange({ ...filters, transports })}
					options={[
						{ value: "websocket", label: "WebSocket", text: "WebSocket" },
						{ value: "http", label: "HTTP", text: "HTTP" },
					]}
					selected={filters.transports}
				/>
			) : null}
		</>
	);
}
