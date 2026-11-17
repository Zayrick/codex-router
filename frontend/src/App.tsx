import { useCallback, useEffect, useMemo, useState, type ComponentType } from "react";
import { RefreshCwIcon, TriangleAlertIcon } from "lucide-react";

import { AdminApiClient, AdminSessionExpiredError } from "@/admin-api";
import { AdminProvider, useAdmin } from "@/admin/context";
import { errorMessage, loadInitialData } from "@/admin/load";
import type { ManagementPage } from "@/admin/pages";
import CodexAccountsPage, { PageAction as CodexAccountsAction } from "@/admin/pages/CodexAccountsPage";
import GroupsPage, { PageAction as GroupsAction } from "@/admin/pages/GroupsPage";
import LogsPage, { PageAction as LogsAction } from "@/admin/pages/LogsPage";
import OverviewPage from "@/admin/pages/OverviewPage";
import PerformancePage from "@/admin/pages/PerformancePage";
import PricingPage, { PageAction as PricingAction } from "@/admin/pages/PricingPage";
import SettingsPage from "@/admin/pages/SettingsPage";
import UsagePage from "@/admin/pages/UsagePage";
import UsersPage, { PageAction as UsersAction } from "@/admin/pages/UsersPage";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import ManagementShell, { ProductMark } from "./ManagementShell";
import { PAGE_META } from "@/admin/pages";
import { CONSOLE_API_BASE, signOut } from "./session-api";
import "./App.css";

const PAGES: Record<ManagementPage, { Page: ComponentType; Action?: ComponentType }> = {
	overview: { Page: OverviewPage },
	logs: { Page: LogsPage, Action: LogsAction },
	usage: { Page: UsagePage },
	performance: { Page: PerformancePage },
	codex: { Page: CodexAccountsPage, Action: CodexAccountsAction },
	groups: { Page: GroupsPage, Action: GroupsAction },
	users: { Page: UsersPage, Action: UsersAction },
	pricing: { Page: PricingPage, Action: PricingAction },
	settings: { Page: SettingsPage },
};

type Loaded = Awaited<ReturnType<typeof loadInitialData>>;

interface AppProps {
	/** Returns to the sign-in screen, optionally explaining why. */
	onSignedOut: (error?: string | null) => void;
}

function App({ onSignedOut }: AppProps) {
	const api = useMemo(() => new AdminApiClient(CONSOLE_API_BASE), []);
	const [initial, setInitial] = useState<Loaded | null>(null);
	const [error, setError] = useState<string | null>(null);

	const load = useCallback(() => {
		loadInitialData(api)
			.then(setInitial)
			.catch((reason: unknown) => {
				if (reason instanceof AdminSessionExpiredError) onSignedOut(null);
				else setError(errorMessage(reason, "无法读取管理状态，请稍后重试。"));
			});
	}, [api, onSignedOut]);

	useEffect(load, [load]);

	if (error) {
		return (
			<main className="loading-screen">
				<ProductMark />
				<Alert className="max-w-md" variant="destructive">
					<TriangleAlertIcon />
					<AlertDescription>{error}</AlertDescription>
				</Alert>
				<div className="flex gap-2">
					<Button
						onClick={() => {
							setError(null);
							load();
						}}
						type="button"
						variant="outline"
					>
						<RefreshCwIcon data-icon="inline-start" />
						重试
					</Button>
					<Button onClick={() => void signOut().finally(() => onSignedOut(null))} type="button" variant="ghost">
						退出登录
					</Button>
				</div>
			</main>
		);
	}
	if (!initial) {
		return (
			<div className="loading-screen">
				<ProductMark />
				<Spinner />
				<strong>加载中…</strong>
			</div>
		);
	}
	return (
		<AdminProvider initial={initial} onSignedOut={onSignedOut}>
			<Console />
		</AdminProvider>
	);
}

function Console() {
	const { page, navigate, logout } = useAdmin();
	const { Page, Action } = PAGES[page];

	useEffect(() => {
		document.title = `${PAGE_META[page].title} · Codex Router`;
	}, [page]);

	return (
		<ManagementShell activePage={page} basePath="/admin" onLogout={() => void logout()} onNavigate={(next) => navigate(next)} pageAction={Action ? <Action /> : undefined}>
			<Page key={page} />
		</ManagementShell>
	);
}

export default App;
