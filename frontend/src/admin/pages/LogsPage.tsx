import { useCallback, useMemo } from "react";
import { DownloadIcon } from "lucide-react";

import { AdminSessionExpiredError, type EventPageQuery } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { publishExportUrl, useExportUrl } from "@/admin/logs/export-store";
import type { FilterOptions } from "@/admin/logs/filter-bar";
import { LogsView, type LogsDataSource } from "@/admin/logs/LogsView";
import { Button } from "@/components/ui/button";

/** 调用日志: every request with filters, summary, session grouping and details. */
export default function LogsPage() {
	const { api, state, pricing, params, now, reportError } = useAdmin();
	const search = params.toString();

	const source = useMemo<LogsDataSource>(
		() => ({
			scope: "admin",
			getEvents: (query: EventPageQuery, signal: AbortSignal) => api.getEvents(query, signal),
			eventsCsvUrl: (query: EventPageQuery) => api.eventsCsvUrl(query),
			onSessionExpired: () => reportError(new AdminSessionExpiredError(), ""),
		}),
		[api, reportError],
	);

	const options = useMemo<FilterOptions>(
		() => ({
			identities: [
				...state.apiKeys.map((key) => ({ filter: { identityType: "api_key" as const, identityId: key.id }, label: key.name, hint: "API Key" })),
				...state.authProxyAccounts.map((account) => ({
					filter: { identityType: "auth_proxy" as const, identityId: account.id },
					label: account.name,
					hint: "下游账户",
				})),
			],
			upstreams: [
				...state.accountGroups.map((group) => ({ filter: { identityType: "account_group" as const, identityId: group.id }, label: group.name, hint: "账户组" })),
				...state.codexAccounts.map((account) => ({
					filter: { identityType: "codex_account" as const, identityId: account.id },
					label: account.name,
					hint: "Codex 账户",
				})),
			],
			models: pricing.usedModels,
		}),
		[pricing.usedModels, state],
	);

	const onExportUrl = useCallback((url: string) => publishExportUrl(url), []);

	return <LogsView initialSearch={search} now={now} onExportUrl={onExportUrl} options={options} source={source} />;
}

/** Heading action: CSV export of the current filters (up to 50,000 rows). */
export function PageAction() {
	const url = useExportUrl();
	return (
		<Button asChild={Boolean(url)} disabled={!url} type="button" variant="outline">
			{url ? (
				<a download href={url}>
					<DownloadIcon data-icon="inline-start" />
					导出 CSV
				</a>
			) : (
				<>
					<DownloadIcon data-icon="inline-start" />
					导出 CSV
				</>
			)}
		</Button>
	);
}
