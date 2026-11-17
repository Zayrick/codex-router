// Console-wide state and actions for administrators. Pages read data and call
// actions through `useAdmin()`; dialogs that several pages open live here.

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { toast } from "sonner";

import {
	AdminApiClient,
	AdminApiError,
	AdminSessionExpiredError,
	type AccountGroup,
	type AdminSettings,
	type AdminState,
	type CodexAccount,
	type CodexAccountDeviceAuthorization,
	type CodexAccountUpdate,
	type CodexCredentialImport,
	type ModelPrice,
	type NotificationChannel,
	type PricingSyncStatus,
	type QuotaOverview,
	type RouteAssignment,
	type User,
	type UserInput,
} from "@/admin-api";
import { CONSOLE_API_BASE, signOut } from "@/session-api";

import { CredentialImportDialog, DeviceLoginDialog, UserEditor } from "./dialogs";
import { replaceUserRoute, routeTargetValue, userRoute, type RouteTargetSelection } from "./routing-model";
import { errorMessage, type InitialData } from "./load";
import { pageFromSearch, type ManagementPage } from "./pages";

const QUOTA_REFRESH_MS = 60_000;

export interface PricingState {
	prices: ModelPrice[];
	usedModels: string[];
	sync: PricingSyncStatus | null;
}

export interface AdminContextValue {
	api: AdminApiClient;
	state: AdminState;
	settings: AdminSettings;
	pricing: PricingState;
	/** Cached quota snapshots with forecasts; null until loaded. */
	quota: QuotaOverview | null;
	/** Wall clock refreshed every 30 s. */
	now: number;
	page: ManagementPage;
	/** Search params of the current page (filters passed by `navigate`). */
	params: URLSearchParams;
	navigate: (page: ManagementPage, params?: Record<string, string>) => void;
	/** Shows an error toast unless it is an expired session (which signs out). */
	reportError: (error: unknown, fallback: string) => void;
	notify: (text: string, tone?: "success" | "error") => void;
	/** Keys `${kind}:${id}` of rows with a pending request. */
	busy: ReadonlySet<string>;
	logout: () => Promise<void>;

	refreshState: () => Promise<void>;
	/** Reloads quota; `sample` first samples every account (`true`) or one account id. */
	refreshQuota: (sample?: boolean | string) => Promise<void>;
	quotaRefreshing: boolean;

	openUserEditor: (entry: User | "new") => void;
	toggleUser: (entry: User) => Promise<void>;
	deleteUser: (entry: User) => Promise<void>;

	updateCodexAccount: (account: CodexAccount, value: CodexAccountUpdate) => Promise<void>;
	deleteCodexAccount: (account: CodexAccount) => Promise<void>;
	startDeviceLogin: () => void;
	deviceLoginActive: boolean;
	openCredentialImport: () => void;

	saveRouting: (groups: AccountGroup[], routes: RouteAssignment[]) => Promise<boolean>;
	saveSettings: (settings: AdminSettings) => Promise<boolean>;
	testNotification: (channel: NotificationChannel) => Promise<void>;
	savePricing: (prices: ModelPrice[]) => Promise<boolean>;
	syncPricing: () => Promise<void>;
	pricingSyncing: boolean;
	/** Increments when pricing or settings change costs; analytics refetch on it. */
	costVersion: number;
}

const AdminContext = createContext<AdminContextValue | null>(null);

// oxlint-disable-next-line react/only-export-components -- the hook belongs with its provider.
export function useAdmin(): AdminContextValue {
	const value = useContext(AdminContext);
	if (!value) throw new Error("useAdmin must be used inside <AdminProvider>");
	return value;
}

export function AdminProvider({
	initial,
	onSignedOut,
	children,
}: {
	initial: InitialData;
	onSignedOut: (error?: string | null) => void;
	children: ReactNode;
}) {
	const api = useMemo(() => new AdminApiClient(CONSOLE_API_BASE), []);
	const [state, setState] = useState(initial.state);
	const [settings, setSettings] = useState(initial.settings);
	const [pricing, setPricing] = useState(initial.pricing);
	const [quota, setQuota] = useState<QuotaOverview | null>(null);
	const [quotaRefreshing, setQuotaRefreshing] = useState(false);
	const [now, setNow] = useState(() => Date.now());
	const [location, setLocation] = useState(() => ({ page: pageFromSearch(window.location.search), search: window.location.search }));
	const [busy, setBusy] = useState<ReadonlySet<string>>(() => new Set());
	const [userEditor, setUserEditor] = useState<User | "new" | null>(null);
	const [editorSaving, setEditorSaving] = useState(false);
	const [deviceFlow, setDeviceFlow] = useState<CodexAccountDeviceAuthorization | null>(null);
	const [deviceOpen, setDeviceOpen] = useState(false);
	const [deviceLoading, setDeviceLoading] = useState(false);
	const [deviceError, setDeviceError] = useState<string | null>(null);
	const [importOpen, setImportOpen] = useState(false);
	const [pricingSyncing, setPricingSyncing] = useState(false);
	const [costVersion, setCostVersion] = useState(0);
	const mounted = useRef(true);
	const deviceFlowId = useRef(0);
	const deviceTimer = useRef<number | null>(null);

	const signedOut = useCallback(() => {
		deviceFlowId.current += 1;
		onSignedOut(null);
	}, [onSignedOut]);

	const reportError = useCallback(
		(error: unknown, fallback: string) => {
			if (error instanceof AdminSessionExpiredError) {
				signedOut();
				return;
			}
			if (error instanceof DOMException && error.name === "AbortError") return;
			toast.error(errorMessage(error, fallback));
		},
		[signedOut],
	);

	const notify = useCallback((text: string, tone: "success" | "error" = "success") => {
		if (tone === "error") toast.error(text);
		else toast.success(text);
	}, []);

	useEffect(() => {
		mounted.current = true;
		const tick = window.setInterval(() => {
			if (!document.hidden) setNow(Date.now());
		}, 30_000);
		const onPopState = () => setLocation({ page: pageFromSearch(window.location.search), search: window.location.search });
		window.addEventListener("popstate", onPopState);
		return () => {
			mounted.current = false;
			window.clearInterval(tick);
			window.removeEventListener("popstate", onPopState);
			if (deviceTimer.current !== null) window.clearTimeout(deviceTimer.current);
		};
	}, []);

	const navigate = useCallback((page: ManagementPage, params?: Record<string, string>) => {
		const url = new URL(window.location.href);
		url.search = "";
		if (page !== "overview") url.searchParams.set("page", page);
		for (const [key, value] of Object.entries(params ?? {})) url.searchParams.set(key, value);
		window.history.pushState(null, "", url);
		setLocation({ page, search: url.search });
		document.querySelector("[data-console-scroll] [data-slot=scroll-area-viewport]")?.scrollTo({ top: 0 });
	}, []);

	const withBusy = useCallback(async (key: string, task: () => Promise<void>) => {
		setBusy((current) => new Set(current).add(key));
		try {
			await task();
		} finally {
			if (mounted.current) {
				setBusy((current) => {
					const next = new Set(current);
					next.delete(key);
					return next;
				});
			}
		}
	}, []);

	const refreshState = useCallback(async () => {
		try {
			const next = await api.getState();
			if (mounted.current) setState(next);
		} catch (error) {
			reportError(error, "读取管理状态失败。");
		}
	}, [api, reportError]);

	const refreshQuota = useCallback(
		async (sample: boolean | string = false) => {
			if (sample) setQuotaRefreshing(true);
			try {
				const next = await api.getQuota(sample);
				if (mounted.current) setQuota(next);
			} catch (error) {
				reportError(error, "读取额度失败。");
			} finally {
				if (mounted.current && sample) setQuotaRefreshing(false);
			}
		},
		[api, reportError],
	);

	useEffect(() => {
		// Load once on mount (outside the effect's synchronous phase), then poll.
		const first = window.setTimeout(() => void refreshQuota(), 0);
		const timer = window.setInterval(() => {
			if (!document.hidden) void refreshQuota();
		}, QUOTA_REFRESH_MS);
		return () => {
			window.clearTimeout(first);
			window.clearInterval(timer);
		};
	}, [refreshQuota]);

	const logout = useCallback(async () => {
		try {
			await signOut();
		} catch (error) {
			if (!(error instanceof AdminSessionExpiredError)) {
				toast.error(errorMessage(error, "退出管理会话失败。"));
				return;
			}
		}
		signedOut();
	}, [signedOut]);

	const saveRouting = useCallback(
		async (groups: AccountGroup[], routes: RouteAssignment[]) => {
			try {
				const next = await api.updateAccountRouting(groups, routes);
				if (!mounted.current) return false;
				setState((current) => ({ ...current, ...next }));
				toast.success("路由已保存。");
				void refreshQuota();
				return true;
			} catch (error) {
				reportError(error, "保存账户组失败。");
				return false;
			}
		},
		[api, refreshQuota, reportError],
	);

	async function saveUserRoute(id: string, target: RouteTargetSelection) {
		if (routeTargetValue(userRoute(state.routes, id)) === routeTargetValue(target)) return;
		const routing = await api.updateAccountRouting(state.accountGroups, replaceUserRoute(state.routes, id, target));
		if (mounted.current) setState((value) => ({ ...value, ...routing }));
	}

	async function saveUser(value: UserInput, target: RouteTargetSelection) {
		if (!userEditor) return;
		const editor = userEditor;
		let saved = false;
		setEditorSaving(true);
		try {
			const next = editor === "new" ? await api.createUser(value) : await api.updateUser(editor.id, value);
			const entry = next.find((item) => (editor === "new" ? item.name === value.name : item.id === editor.id));
			if (!entry) throw new AdminApiError("管理服务返回了无法识别的数据。", 502);
			saved = true;
			setState((current) => ({ ...current, users: next }));
			await saveUserRoute(entry.id, target);
			setUserEditor(null);
			toast.success(editor === "new" ? "用户已添加。" : "用户已更新。");
		} catch (error) {
			reportError(error, saved ? "用户已保存，但路由分配失败，请重试。" : "保存用户失败。");
		} finally {
			if (mounted.current) setEditorSaving(false);
		}
	}

	const toggleUser = useCallback(
		(entry: User) =>
			withBusy(`user:${entry.id}`, async () => {
				try {
					const next = await api.updateUser(entry.id, { name: entry.name, enabled: !entry.enabled, credentials: entry.credentials });
					if (mounted.current) setState((current) => ({ ...current, users: next }));
				} catch (error) {
					reportError(error, "更新用户失败。");
				}
			}),
		[api, reportError, withBusy],
	);

	const deleteUser = useCallback(
		(entry: User) =>
			withBusy(`user:${entry.id}`, async () => {
				try {
					const next = await api.deleteUser(entry.id);
					if (!mounted.current) return;
					setState((current) => ({
						...current,
						users: next,
						routes: current.routes.filter((route) => route.userId !== entry.id),
					}));
					toast.success("用户已删除。");
				} catch (error) {
					reportError(error, "删除用户失败。");
				}
			}),
		[api, reportError, withBusy],
	);

	const updateCodexAccount = useCallback(
		(account: CodexAccount, value: CodexAccountUpdate) =>
			withBusy(`codex:${account.id}`, async () => {
				try {
					const accounts = await api.updateCodexAccount(account.id, value);
					const routing = await api.getAccountRouting();
					if (!mounted.current) return;
					setState((current) => ({ ...current, codexAccounts: accounts, ...routing }));
					toast.success("账户已更新。");
					void refreshQuota();
				} catch (error) {
					reportError(error, "更新账户失败。");
				}
			}),
		[api, refreshQuota, reportError, withBusy],
	);

	const deleteCodexAccount = useCallback(
		(account: CodexAccount) =>
			withBusy(`codex:${account.id}`, async () => {
				try {
					const accounts = await api.deleteCodexAccount(account.id);
					const routing = await api.getAccountRouting();
					if (!mounted.current) return;
					setState((current) => ({ ...current, codexAccounts: accounts, ...routing }));
					setSettings((current) => ({
						...current,
						notifications: {
							...current.notifications,
							accountIds: current.notifications.accountIds.filter((id) => id !== account.id),
						},
					}));
					toast.success("账户已删除。");
					void refreshQuota();
				} catch (error) {
					reportError(error, "删除账户失败。");
				}
			}),
		[api, refreshQuota, reportError, withBusy],
	);

	function clearDeviceTimer() {
		if (deviceTimer.current !== null) {
			window.clearTimeout(deviceTimer.current);
			deviceTimer.current = null;
		}
	}

	function cancelDeviceLogin() {
		deviceFlowId.current += 1;
		clearDeviceTimer();
		setDeviceOpen(false);
		setDeviceFlow(null);
		setDeviceError(null);
		setDeviceLoading(false);
	}

	function scheduleDevicePoll(flowId: number, accountId: string, flowState: string, retryAfter: number) {
		clearDeviceTimer();
		deviceTimer.current = window.setTimeout(() => void pollDeviceLogin(flowId, accountId, flowState), Math.max(1, retryAfter) * 1_000);
	}

	async function pollDeviceLogin(flowId: number, accountId: string, flowState: string) {
		if (flowId !== deviceFlowId.current) return;
		try {
			const result = await api.pollCodexAccountDeviceAuthorization(accountId, flowState);
			if (!mounted.current || flowId !== deviceFlowId.current) return;
			if (result.status === "pending") {
				scheduleDevicePoll(flowId, accountId, flowState, result.retryAfter);
				return;
			}
			setState((current) => ({
				...current,
				codexAccounts: [...current.codexAccounts.filter((entry) => entry.id !== result.account.id), result.account],
			}));
			cancelDeviceLogin();
			toast.success("账户已添加。");
			void refreshQuota(true);
		} catch (error) {
			if (!mounted.current || flowId !== deviceFlowId.current) return;
			if (error instanceof AdminSessionExpiredError) signedOut();
			else setDeviceError(errorMessage(error, "检查设备登录状态失败。"));
		}
	}

	async function startDeviceLogin() {
		const flowId = ++deviceFlowId.current;
		clearDeviceTimer();
		setDeviceOpen(true);
		setDeviceFlow(null);
		setDeviceError(null);
		setDeviceLoading(true);
		try {
			const flow = await api.startCodexAccountDeviceAuthorization();
			if (!mounted.current || flowId !== deviceFlowId.current) return;
			setDeviceFlow(flow);
			scheduleDevicePoll(flowId, flow.accountId, flow.authorization.state, flow.authorization.interval);
		} catch (error) {
			if (!mounted.current || flowId !== deviceFlowId.current) return;
			if (error instanceof AdminSessionExpiredError) signedOut();
			else setDeviceError(errorMessage(error, "无法创建设备登录码。"));
		} finally {
			if (mounted.current && flowId === deviceFlowId.current) setDeviceLoading(false);
		}
	}

	async function importCodexCredentials(value: CodexCredentialImport) {
		try {
			const account = await api.importCodexCredentials(value);
			if (!mounted.current) return;
			setState((current) => ({ ...current, codexAccounts: [...current.codexAccounts, account] }));
			setImportOpen(false);
			toast.success("凭据已导入，账户已添加。");
			void refreshQuota(true);
		} catch (error) {
			if (error instanceof AdminSessionExpiredError) {
				signedOut();
				return;
			}
			throw error;
		}
	}

	const saveSettings = useCallback(
		async (next: AdminSettings) => {
			try {
				const saved = await api.replaceSettings(next);
				if (!mounted.current) return false;
				const costsChanged = saved.usage.fastCostMultiplier !== settings.usage.fastCostMultiplier;
				setSettings(saved);
				if (costsChanged) setCostVersion((value) => value + 1);
				toast.success("设置已保存。");
				return true;
			} catch (error) {
				reportError(error, "保存设置失败。");
				return false;
			}
		},
		[api, reportError, settings.usage.fastCostMultiplier],
	);

	const testNotification = useCallback(
		async (channel: NotificationChannel) => {
			try {
				await api.testNotification(channel);
				toast.success("测试通知已发送。");
			} catch (error) {
				reportError(error, "发送测试通知失败。");
			}
		},
		[api, reportError],
	);

	const savePricing = useCallback(
		async (prices: ModelPrice[]) => {
			try {
				const next = await api.replacePricing(prices);
				if (!mounted.current) return false;
				setPricing((current) => ({ ...current, prices: next }));
				setCostVersion((value) => value + 1);
				toast.success("模型价格已保存。");
				return true;
			} catch (error) {
				reportError(error, "保存模型价格失败。");
				return false;
			}
		},
		[api, reportError],
	);

	const syncPricing = useCallback(async () => {
		setPricingSyncing(true);
		try {
			const result = await api.syncPricing();
			if (!mounted.current) return;
			setPricing((current) => ({ ...current, prices: result.prices, sync: result.sync }));
			setCostVersion((value) => value + 1);
			if (result.unmatchedModels.length) toast.warning(`已匹配 ${result.matchedModels.length} 个模型，${result.unmatchedModels.length} 个未匹配。`);
			else toast.success(`已匹配 ${result.matchedModels.length} 个模型价格。`);
		} catch (error) {
			reportError(error, "从 Models.dev 获取价格失败。");
		} finally {
			if (mounted.current) setPricingSyncing(false);
		}
	}, [api, reportError]);

	const value: AdminContextValue = {
		api,
		state,
		settings,
		pricing,
		quota,
		now,
		page: location.page,
		params: new URLSearchParams(location.search),
		navigate,
		reportError,
		notify,
		busy,
		logout,
		refreshState,
		refreshQuota,
		quotaRefreshing,
		openUserEditor: setUserEditor,
		toggleUser,
		deleteUser,
		updateCodexAccount,
		deleteCodexAccount,
		startDeviceLogin: () => void startDeviceLogin(),
		deviceLoginActive: deviceOpen,
		openCredentialImport: () => setImportOpen(true),
		saveRouting,
		saveSettings,
		testNotification,
		savePricing,
		syncPricing,
		pricingSyncing,
		costVersion,
	};

	return (
		<AdminContext.Provider value={value}>
			{children}
			{userEditor ? (
				<UserEditor
					accounts={state.codexAccounts}
					entry={userEditor}
					groups={state.accountGroups}
					loading={editorSaving}
					onCancel={() => setUserEditor(null)}
					onSave={(input, target) => void saveUser(input, target)}
					route={userEditor === "new" ? null : userRoute(state.routes, userEditor.id)}
				/>
			) : null}
			{deviceOpen ? (
				<DeviceLoginDialog
					error={deviceError}
					flow={deviceFlow}
					loading={deviceLoading}
					onCancel={cancelDeviceLogin}
					onRetry={() => void startDeviceLogin()}
				/>
			) : null}
			{importOpen ? <CredentialImportDialog onCancel={() => setImportOpen(false)} onImport={importCodexCredentials} /> : null}
		</AdminContext.Provider>
	);
}

