import { useCallback, useMemo } from "react";
import { DownloadIcon } from "lucide-react";

import { AdminSessionExpiredError, type EventPageQuery } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { publishExportUrl, useExportUrl } from "@/admin/logs/export-store";
import type { FilterOptions } from "@/admin/logs/filter-bar";
import { LogsView, type LogsDataSource } from "@/admin/logs/LogsView";
import { identityTypeLabel } from "@/components/app/viz";
import { Button } from "@/components/ui/button";

/** 调用日志: every request with filters, summary, session grouping and details. */
export default function LogsPage() {
	const { api, state, pricing, params, now, reportError, live } = useAdmin();
	const pushTick = live.connected ? live.usageTick : undefined;
	const search = params.toString();

	const source = useMemo<LogsDataSource>(
		() => ({
			scope: "admin",
			revision: state,
			getEvents: (query: EventPageQuery, signal: AbortSignal) => api.getEvents(query, signal),
			eventsCsvUrl: (query: EventPageQuery) => api.eventsCsvUrl(query),
			onSessionExpired: () => reportError(new AdminSessionExpiredError(), ""),
			...(pushTick !== undefined ? { pushTick } : {}),
		}),
		[api, pushTick, reportError, state],
	);

	const options = useMemo<FilterOptions>(
		() => ({
			identities: [
				...state.users.map((user) => ({ filter: { identityType: "user" as const, identityId: user.id }, label: user.name, hint: "用户" })),
				...state.users.flatMap((user) =>
					user.credentials.map((credential) => ({
						filter: { identityType: "credential" as const, identityId: credential.id },
						label: credential.name || `${identityTypeLabel(credential.type)} ···${credential.value.slice(-4)}`,
						hint: user.name,
					})),
				),
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
