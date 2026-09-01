import {
	useEffect,
	useMemo,
	useRef,
	useState,
	type FormEvent,
} from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectSeparator,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { ScrollArea } from "@/components/ui/scroll-area";
import AccountGroups from "./AccountGroups";
import CodexAccounts from "./CodexAccounts";
import DeleteConfirmationDialog from "./DeleteConfirmationDialog";
import {
	AdminApiClient,
	AdminApiError,
	AdminSessionExpiredError,
	type AccountGroup,
	type AdminState,
	type AuthProxyAccount,
	type AuthProxyAccountInput,
	type ClientApiKey,
	type ClientApiKeyInput,
	type CodexAccount,
	type CodexAccountDeviceAuthorization,
	type CodexAccountUpdate,
	type ModelPrice,
	type RouteAssignment,
	type RouteConsumerType,
	type SubscriptionInfo,
	type UsageDashboard,
	type UsageIdentityFilter,
	type UsageIdentityType,
	type UsageRange,
} from "./admin-api";
import ManagementShell, {
	ProductMark,
	type ManagementPage,
} from "./ManagementShell";
import ModelPricingCard from "./ModelPricingCard";
import {
	ActivityHeatmaps,
	DownstreamCostDonut,
	UsageBreakdownDonuts,
	UsageLineCharts,
} from "./UsageVisuals";
import { formatCost } from "./usage-format";
import "./App.css";
import "./UnifiedRouting.css";

const MANAGEMENT_PATH_PATTERN = /^\/[A-Za-z0-9_-]{1,128}\/admin\/?$/;
const MIN_API_KEY_LENGTH = 11;
const MAX_API_KEY_LENGTH = 512;
const GENERATED_API_KEY_LENGTH = 20;
const MAX_ACCOUNT_ID_LENGTH = 256;
const ALL_USAGE_IDENTITIES_VALUE = "__all_usage_identities__";
const UNASSIGNED_ROUTE_TARGET_VALUE = "__unassigned_route_target__";
const INTEGER_FORMAT = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 });
const COMPACT_NUMBER_FORMAT = new Intl.NumberFormat("zh-CN", {
	notation: "compact",
	maximumFractionDigits: 2,
});

const EMPTY_STATE: AdminState = {
	codexAccounts: [],
	apiKeys: [],
	authProxyAccounts: [],
	accountGroups: [],
	routes: [],
};

type Screen = "loading" | "login" | "dashboard" | "invalid-path";
type Notice = { tone: "success" | "error"; text: string };
type EditableKey = ClientApiKey | "new" | null;
type EditableProxy = AuthProxyAccount | "new" | null;
type RouteTargetSelection = Pick<RouteAssignment, "targetType" | "targetId"> | null;

function App() {
	const basePath = useMemo(() => managementBasePath(window.location.pathname), []);
	const api = useMemo(() => basePath ? new AdminApiClient(basePath) : null, [basePath]);
	const [screen, setScreen] = useState<Screen>(basePath ? "loading" : "invalid-path");
	const [activePage, setActivePage] = useState<ManagementPage>(() => managementPageFromSearch(window.location.search));
	const [data, setData] = useState<AdminState>(EMPTY_STATE);
	const [loginLoading, setLoginLoading] = useState(false);
	const [loginError, setLoginError] = useState<string | null>(null);
	const [notice, setNotice] = useState<Notice | null>(null);
	const [now, setNow] = useState(() => Date.now());

	const [usage, setUsage] = useState<UsageDashboard | null>(null);
	const [overviewUsage, setOverviewUsage] = useState<UsageDashboard | null>(null);
	const [usageRange, setUsageRange] = useState<UsageRange>("cycle");
	const [usageIdentity, setUsageIdentity] = useState<UsageIdentityFilter | null>(null);
	const [usageLoading, setUsageLoading] = useState(false);
	const [usageError, setUsageError] = useState<string | null>(null);

	const [modelPrices, setModelPrices] = useState<ModelPrice[]>([]);
	const [usedModels, setUsedModels] = useState<string[]>([]);
	const [pricingLoading, setPricingLoading] = useState(false);
	const [pricingSaving, setPricingSaving] = useState(false);
	const [pricingSyncing, setPricingSyncing] = useState(false);
	const [pricingError, setPricingError] = useState<string | null>(null);

	const [subscriptions, setSubscriptions] = useState<Record<string, SubscriptionInfo>>({});
	const [subscriptionLoading, setSubscriptionLoading] = useState<ReadonlySet<string>>(() => new Set());
	const [subscriptionErrors, setSubscriptionErrors] = useState<Record<string, string>>({});
	const [busyAccounts, setBusyAccounts] = useState<ReadonlySet<string>>(() => new Set());
	const [deviceFlow, setDeviceFlow] = useState<CodexAccountDeviceAuthorization | null>(null);
	const [deviceLoading, setDeviceLoading] = useState(false);
	const [deviceError, setDeviceError] = useState<string | null>(null);

	const [accountGroupsSaving, setAccountGroupsSaving] = useState(false);
	const [keyEditor, setKeyEditor] = useState<EditableKey>(null);
	const [keySaving, setKeySaving] = useState(false);
	const [busyKeys, setBusyKeys] = useState<ReadonlySet<string>>(() => new Set());
	const [proxyEditor, setProxyEditor] = useState<EditableProxy>(null);
	const [proxySaving, setProxySaving] = useState(false);
	const [busyProxies, setBusyProxies] = useState<ReadonlySet<string>>(() => new Set());

	const mountedRef = useRef(false);
	const usageRequestRef = useRef(0);
	const deviceFlowRef = useRef(0);
	const deviceTimerRef = useRef<number | null>(null);
	const subscriptionInFlightRef = useRef<Set<string>>(new Set());
	const initializeRef = useRef(initialize);
	const refreshSubscriptionRef = useRef(refreshSubscription);

	useEffect(() => {
		initializeRef.current = initialize;
		refreshSubscriptionRef.current = refreshSubscription;
	});

	useEffect(() => {
		mountedRef.current = true;
		if (api) void initializeRef.current();
		return () => {
			mountedRef.current = false;
			clearDeviceTimer();
		};
	}, [api]);

	useEffect(() => {
		if (!notice) return;
		const timer = window.setTimeout(() => setNotice(null), 4_500);
		return () => window.clearTimeout(timer);
	}, [notice]);

	useEffect(() => {
		if (screen !== "dashboard") return;
		const timer = window.setInterval(() => {
			if (!document.hidden) setNow(Date.now());
		}, 60_000);
		return () => window.clearInterval(timer);
	}, [screen]);

	useEffect(() => {
		const onPopState = () => setActivePage(managementPageFromSearch(window.location.search));
		window.addEventListener("popstate", onPopState);
		return () => window.removeEventListener("popstate", onPopState);
	}, []);

	useEffect(() => {
		if (screen === "dashboard") document.title = `${managementPageTitle(activePage)} · Codex Router`;
	}, [activePage, screen]);

	useEffect(() => {
		if (screen !== "dashboard" || activePage !== "account") return;
		for (const account of data.codexAccounts) {
			if (account.enabled && !subscriptions[account.id]) void refreshSubscriptionRef.current(account);
		}
	}, [activePage, data.codexAccounts, screen, subscriptions]);

	async function initialize(): Promise<void> {
		if (!api) return;
		try {
			const state = await api.getState();
			if (!mountedRef.current) return;
			setData(state);
			setScreen("dashboard");
			setLoginError(null);
			void refreshUsage("cycle", null);
			void refreshOverviewUsage();
			void refreshPricing();
		} catch (error) {
			if (!mountedRef.current) return;
			if (error instanceof AdminSessionExpiredError) {
				resetForLogin();
			} else {
				setScreen("login");
				setLoginError(errorMessage(error, "无法读取管理状态，请稍后重试。"));
			}
		}
	}

	async function handleLogin(secret: string): Promise<void> {
		if (!api || loginLoading) return;
		setLoginLoading(true);
		setLoginError(null);
		try {
			await api.login(secret);
			if (!mountedRef.current) return;
			setScreen("loading");
			await initialize();
		} catch (error) {
			if (!mountedRef.current) return;
			setLoginError(errorMessage(error, "登录失败，请稍后重试。"));
			setScreen("login");
		} finally {
			if (mountedRef.current) setLoginLoading(false);
		}
	}

	async function handleLogout(): Promise<void> {
		if (!api) return;
		try {
			await api.logout();
		} catch (error) {
			if (!(error instanceof AdminSessionExpiredError)) {
				showNotice(errorMessage(error, "退出管理会话失败。"), "error");
				return;
			}
		}
		if (mountedRef.current) resetForLogin();
	}

	function handleSessionFailure(error: unknown): boolean {
		if (!(error instanceof AdminSessionExpiredError)) return false;
		resetForLogin();
		return true;
	}

	function resetForLogin(): void {
		cancelDeviceLogin();
		setData(EMPTY_STATE);
		setUsage(null);
		setOverviewUsage(null);
		setSubscriptions({});
		setScreen("login");
	}

	function navigateManagementPage(page: ManagementPage): void {
		if (page === activePage) return;
		const url = new URL(window.location.href);
		if (page === "overview") url.searchParams.delete("page");
		else url.searchParams.set("page", page);
		window.history.pushState(null, "", url);
		setActivePage(page);
	}

	async function refreshUsage(
		range = usageRange,
		identity = usageIdentity,
	): Promise<void> {
		if (!api) return;
		const requestId = ++usageRequestRef.current;
		setUsageLoading(true);
		setUsageError(null);
		try {
			const next = await api.getUsage(range, identity);
			if (!mountedRef.current || requestId !== usageRequestRef.current) return;
			setUsage(next);
		} catch (error) {
			if (!mountedRef.current || requestId !== usageRequestRef.current) return;
			if (!handleSessionFailure(error)) setUsageError(errorMessage(error, "读取 Token 用量失败。"));
		} finally {
			if (mountedRef.current && requestId === usageRequestRef.current) setUsageLoading(false);
		}
	}

	async function refreshOverviewUsage(): Promise<void> {
		if (!api) return;
		try {
			const next = await api.getUsage("cycle");
			if (mountedRef.current) setOverviewUsage(next);
		} catch (error) {
			if (mountedRef.current) handleSessionFailure(error);
		}
	}

	function changeUsageRange(range: UsageRange): void {
		setUsageRange(range);
		void refreshUsage(range, usageIdentity);
	}

	function changeUsageIdentity(identity: UsageIdentityFilter | null): void {
		setUsageIdentity(identity);
		void refreshUsage(usageRange, identity);
	}

	async function refreshPricing(): Promise<void> {
		if (!api || pricingLoading) return;
		setPricingLoading(true);
		setPricingError(null);
		try {
			const next = await api.getPricing();
			if (!mountedRef.current) return;
			setModelPrices(next.prices);
			setUsedModels(next.usedModels);
		} catch (error) {
			if (mountedRef.current && !handleSessionFailure(error)) setPricingError(errorMessage(error, "读取模型价格失败。"));
		} finally {
			if (mountedRef.current) setPricingLoading(false);
		}
	}

	async function savePricing(prices: ModelPrice[]): Promise<void> {
		if (!api || pricingSaving) return;
		setPricingSaving(true);
		setPricingError(null);
		try {
			const next = await api.replacePricing(prices);
			if (!mountedRef.current) return;
			setModelPrices(next);
			showNotice("模型价格已保存。", "success");
			void refreshUsage();
			void refreshOverviewUsage();
		} catch (error) {
			if (!handleSessionFailure(error)) setPricingError(errorMessage(error, "保存模型价格失败。"));
		} finally {
			if (mountedRef.current) setPricingSaving(false);
		}
	}

	async function syncPricing(): Promise<void> {
		if (!api || pricingSyncing) return;
		setPricingSyncing(true);
		setPricingError(null);
		try {
			const result = await api.syncPricing();
			if (!mountedRef.current) return;
			setModelPrices(result.prices);
			showNotice(`已匹配 ${result.matchedModels.length} 个模型价格。`, result.unmatchedModels.length ? "error" : "success");
			void refreshUsage();
			void refreshOverviewUsage();
		} catch (error) {
			if (!handleSessionFailure(error)) setPricingError(errorMessage(error, "从 Models.dev 获取价格失败。"));
		} finally {
			if (mountedRef.current) setPricingSyncing(false);
		}
	}

	async function refreshSubscription(account: CodexAccount): Promise<void> {
		if (!api || subscriptionInFlightRef.current.has(account.id)) return;
		subscriptionInFlightRef.current.add(account.id);
		setSubscriptionLoading(new Set(subscriptionInFlightRef.current));
		setSubscriptionErrors((current) => omitKey(current, account.id));
		try {
			const next = await api.getCodexAccountSubscription(account.id);
			if (!mountedRef.current) return;
			setSubscriptions((current) => ({ ...current, [account.id]: next }));
			setNow(Date.now());
		} catch (error) {
			if (!mountedRef.current || handleSessionFailure(error)) return;
			setSubscriptionErrors((current) => ({ ...current, [account.id]: errorMessage(error, "额度同步失败。") }));
		} finally {
			subscriptionInFlightRef.current.delete(account.id);
			if (mountedRef.current) setSubscriptionLoading(new Set(subscriptionInFlightRef.current));
		}
	}

	async function updateCodexAccount(account: CodexAccount, value: CodexAccountUpdate): Promise<void> {
		if (!api || busyAccounts.has(account.id)) return;
		setBusyAccounts((current) => withSetValue(current, account.id, true));
		try {
			const accounts = await api.updateCodexAccount(account.id, value);
			const routing = await api.getAccountRouting();
			if (!mountedRef.current) return;
			setData((current) => ({ ...current, codexAccounts: accounts, ...routing }));
			showNotice("账户已更新。", "success");
		} catch (error) {
			if (!handleSessionFailure(error)) showNotice(errorMessage(error, "更新账户失败。"), "error");
		} finally {
			if (mountedRef.current) setBusyAccounts((current) => withSetValue(current, account.id, false));
		}
	}

	async function deleteCodexAccount(account: CodexAccount): Promise<void> {
		if (!api || busyAccounts.has(account.id)) return;
		setBusyAccounts((current) => withSetValue(current, account.id, true));
		try {
			const accounts = await api.deleteCodexAccount(account.id);
			const routing = await api.getAccountRouting();
			if (!mountedRef.current) return;
			setData((current) => ({ ...current, codexAccounts: accounts, ...routing }));
			setSubscriptions((current) => omitKey(current, account.id));
			showNotice("账户已删除。", "success");
		} catch (error) {
			if (!handleSessionFailure(error)) showNotice(errorMessage(error, "删除账户失败。"), "error");
		} finally {
			if (mountedRef.current) setBusyAccounts((current) => withSetValue(current, account.id, false));
		}
	}

	async function startDeviceLogin(): Promise<void> {
		if (!api || deviceLoading) return;
		const flowId = ++deviceFlowRef.current;
		clearDeviceTimer();
		setDeviceFlow(null);
		setDeviceError(null);
		setDeviceLoading(true);
		try {
			const flow = await api.startCodexAccountDeviceAuthorization();
			if (!mountedRef.current || flowId !== deviceFlowRef.current) return;
			setDeviceFlow(flow);
			scheduleDevicePoll(flowId, flow.accountId, flow.authorization.state, flow.authorization.interval);
		} catch (error) {
			if (!mountedRef.current || flowId !== deviceFlowRef.current) return;
			if (!handleSessionFailure(error)) setDeviceError(errorMessage(error, "无法创建设备登录码。"));
		} finally {
			if (mountedRef.current && flowId === deviceFlowRef.current) setDeviceLoading(false);
		}
	}

	function scheduleDevicePoll(flowId: number, accountId: string, state: string, retryAfter: number): void {
		clearDeviceTimer();
		deviceTimerRef.current = window.setTimeout(
			() => void pollDeviceLogin(flowId, accountId, state),
			Math.max(1, retryAfter) * 1_000,
		);
	}

	async function pollDeviceLogin(flowId: number, accountId: string, state: string): Promise<void> {
		if (!api || flowId !== deviceFlowRef.current) return;
		try {
			const result = await api.pollCodexAccountDeviceAuthorization(accountId, state);
			if (!mountedRef.current || flowId !== deviceFlowRef.current) return;
			if (result.status === "pending") {
				scheduleDevicePoll(flowId, accountId, state, result.retryAfter);
				return;
			}
			setData((current) => ({
				...current,
				codexAccounts: [...current.codexAccounts.filter((entry) => entry.id !== result.account.id), result.account],
			}));
			cancelDeviceLogin();
			showNotice("账户已添加。", "success");
			void refreshSubscription(result.account);
		} catch (error) {
			if (!mountedRef.current || flowId !== deviceFlowRef.current) return;
			if (!handleSessionFailure(error)) setDeviceError(errorMessage(error, "检查设备登录状态失败。"));
		}
	}

	function cancelDeviceLogin(): void {
		deviceFlowRef.current += 1;
		clearDeviceTimer();
		setDeviceFlow(null);
		setDeviceError(null);
		setDeviceLoading(false);
	}

	function clearDeviceTimer(): void {
		if (deviceTimerRef.current !== null) {
			window.clearTimeout(deviceTimerRef.current);
			deviceTimerRef.current = null;
		}
	}

	async function saveAccountGroups(groups: AccountGroup[], routes: RouteAssignment[]): Promise<boolean> {
		if (!api || accountGroupsSaving) return false;
		setAccountGroupsSaving(true);
		try {
			const next = await api.updateAccountRouting(groups, routes);
			if (!mountedRef.current) return false;
			setData((current) => ({ ...current, ...next }));
			showNotice("账户组已保存。", "success");
			return true;
		} catch (error) {
			if (!handleSessionFailure(error)) showNotice(errorMessage(error, "保存账户组失败。"), "error");
			return false;
		} finally {
			if (mountedRef.current) setAccountGroupsSaving(false);
		}
	}

	async function saveApiKey(value: ClientApiKeyInput, target: RouteTargetSelection): Promise<void> {
		if (!api || !keyEditor || keySaving) return;
		const editor = keyEditor;
		let identitySaved = false;
		setKeySaving(true);
		try {
			const next = editor === "new" ? await api.createApiKey(value) : await api.updateApiKey(editor.id, value);
			if (!mountedRef.current) return;
			const saved = editor === "new"
				? next.find((entry) => entry.key === value.key)
				: next.find((entry) => entry.id === editor.id);
			if (!saved) throw invalidAdminResponse();
			identitySaved = true;
			setData((current) => ({ ...current, apiKeys: next }));
			if (editor === "new") setKeyEditor(saved);
			const currentRoute = consumerRoute(data.routes, "api_key", saved.id);
			if (routeTargetValue(currentRoute) !== routeTargetValue(target)) {
				const routing = await api.updateAccountRouting(
					data.accountGroups,
					replaceConsumerRoute(data.routes, "api_key", saved.id, target),
				);
				if (!mountedRef.current) return;
				setData((current) => ({ ...current, apiKeys: next, ...routing }));
			}
			setKeyEditor(null);
			showNotice(editor === "new" ? "API Key 已创建。" : "API Key 已更新。", "success");
		} catch (error) {
			if (!handleSessionFailure(error)) {
				showNotice(errorMessage(error, identitySaved ? "API Key 已保存，但账户分配失败，请重试。" : "保存 API Key 失败。"), "error");
			}
		} finally {
			if (mountedRef.current) setKeySaving(false);
		}
	}

	async function toggleApiKey(entry: ClientApiKey): Promise<void> {
		if (!api || busyKeys.has(entry.id)) return;
		setBusyKeys((current) => withSetValue(current, entry.id, true));
		try {
			const next = await api.updateApiKey(entry.id, { name: entry.name, key: entry.key, enabled: !entry.enabled });
			if (mountedRef.current) setData((current) => ({ ...current, apiKeys: next }));
		} catch (error) {
			if (!handleSessionFailure(error)) showNotice(errorMessage(error, "更新 API Key 失败。"), "error");
		} finally {
			if (mountedRef.current) setBusyKeys((current) => withSetValue(current, entry.id, false));
		}
	}

	async function deleteApiKey(entry: ClientApiKey): Promise<void> {
		if (!api) return;
		setBusyKeys((current) => withSetValue(current, entry.id, true));
		try {
			const next = await api.deleteApiKey(entry.id);
			if (!mountedRef.current) return;
			setData((current) => ({ ...current, apiKeys: next, routes: current.routes.filter((route) => route.consumerType !== "api_key" || route.consumerId !== entry.id) }));
			showNotice("API Key 已删除。", "success");
		} catch (error) {
			if (!handleSessionFailure(error)) showNotice(errorMessage(error, "删除 API Key 失败。"), "error");
		} finally {
			if (mountedRef.current) setBusyKeys((current) => withSetValue(current, entry.id, false));
		}
	}

	async function saveProxy(value: AuthProxyAccountInput, target: RouteTargetSelection): Promise<void> {
		if (!api || !proxyEditor || proxySaving) return;
		const editor = proxyEditor;
		let identitySaved = false;
		setProxySaving(true);
		try {
			const next = editor === "new" ? await api.createAuthProxyAccount(value) : await api.updateAuthProxyAccount(editor.id, value);
			if (!mountedRef.current) return;
			const saved = editor === "new"
				? next.find((entry) => entry.accountId === value.accountId)
				: next.find((entry) => entry.id === editor.id);
			if (!saved) throw invalidAdminResponse();
			identitySaved = true;
			setData((current) => ({ ...current, authProxyAccounts: next }));
			if (editor === "new") setProxyEditor(saved);
			const currentRoute = consumerRoute(data.routes, "auth_proxy", saved.id);
			if (routeTargetValue(currentRoute) !== routeTargetValue(target)) {
				const routing = await api.updateAccountRouting(
					data.accountGroups,
					replaceConsumerRoute(data.routes, "auth_proxy", saved.id, target),
				);
				if (!mountedRef.current) return;
				setData((current) => ({ ...current, authProxyAccounts: next, ...routing }));
			}
			setProxyEditor(null);
			showNotice(editor === "new" ? "下游账户已添加。" : "下游账户已更新。", "success");
		} catch (error) {
			if (!handleSessionFailure(error)) {
				showNotice(errorMessage(error, identitySaved ? "下游账户已保存，但账户分配失败，请重试。" : "保存下游账户失败。"), "error");
			}
		} finally {
			if (mountedRef.current) setProxySaving(false);
		}
	}

	async function toggleProxy(entry: AuthProxyAccount): Promise<void> {
		if (!api || busyProxies.has(entry.id)) return;
		setBusyProxies((current) => withSetValue(current, entry.id, true));
		try {
			const next = await api.updateAuthProxyAccount(entry.id, { name: entry.name, accountId: entry.accountId, enabled: !entry.enabled });
			if (mountedRef.current) setData((current) => ({ ...current, authProxyAccounts: next }));
		} catch (error) {
			if (!handleSessionFailure(error)) showNotice(errorMessage(error, "更新下游账户失败。"), "error");
		} finally {
			if (mountedRef.current) setBusyProxies((current) => withSetValue(current, entry.id, false));
		}
	}

	async function deleteProxy(entry: AuthProxyAccount): Promise<void> {
		if (!api) return;
		setBusyProxies((current) => withSetValue(current, entry.id, true));
		try {
			const next = await api.deleteAuthProxyAccount(entry.id);
			if (!mountedRef.current) return;
			setData((current) => ({ ...current, authProxyAccounts: next, routes: current.routes.filter((route) => route.consumerType !== "auth_proxy" || route.consumerId !== entry.id) }));
			showNotice("下游账户已删除。", "success");
		} catch (error) {
			if (!handleSessionFailure(error)) showNotice(errorMessage(error, "删除下游账户失败。"), "error");
		} finally {
			if (mountedRef.current) setBusyProxies((current) => withSetValue(current, entry.id, false));
		}
	}

	function showNotice(text: string, tone: Notice["tone"]): void {
		setNotice({ text, tone });
	}

	if (screen === "invalid-path") return <InvalidPath />;
	if (screen === "loading") return <LoadingView />;
	if (screen === "login") return <ScrollArea className="h-svh"><LoginView error={loginError} loading={loginLoading} onSubmit={(secret) => void handleLogin(secret)} /></ScrollArea>;
	if (!basePath) return <InvalidPath />;
	const pageAction = activePage === "api-keys" ? (
		<button className="button button-primary" onClick={() => setKeyEditor("new")} type="button">
			<PlusIcon />
			添加 API Key
		</button>
	) : activePage === "accounts" ? (
		<button className="button button-primary" onClick={() => setProxyEditor("new")} type="button">
			<PlusIcon />
			添加下游账户
		</button>
	) : activePage === "account" ? (
		<button
			className="button button-primary"
			disabled={deviceLoading || deviceFlow !== null}
			onClick={() => void startDeviceLogin()}
			type="button"
		>
			<PlusIcon />
			登录新账户
		</button>
	) : undefined;

	return (
		<ManagementShell
			activePage={activePage}
			basePath={basePath}
			onLogout={() => void handleLogout()}
			onNavigate={navigateManagementPage}
			pageAction={pageAction}
		>
			{activePage === "overview" ? <Overview data={data} now={now} onNavigate={navigateManagementPage} usage={overviewUsage} /> : null}
			{activePage === "usage" ? (
				<UsagePanel
					accounts={data.authProxyAccounts}
					apiKeys={data.apiKeys}
					codexAccounts={data.codexAccounts}
					error={usageError}
					groups={data.accountGroups}
					identity={usageIdentity}
					loading={usageLoading}
					now={now}
					onIdentityChange={changeUsageIdentity}
					onRangeChange={changeUsageRange}
					onRefresh={() => void refreshUsage()}
					range={usageRange}
					usage={usage}
				/>
			) : null}
			{activePage === "api-keys" ? (
				<IdentityTable
					busy={busyKeys}
					entries={data.apiKeys}
					kind="api_key"
					onDelete={(entry) => void deleteApiKey(entry as ClientApiKey)}
					onEdit={(entry) => setKeyEditor(entry as ClientApiKey)}
					onToggle={(entry) => void toggleApiKey(entry as ClientApiKey)}
					routes={data.routes}
					targets={{ accounts: data.codexAccounts, groups: data.accountGroups }}
				/>
			) : null}
			{activePage === "accounts" ? (
				<IdentityTable
					busy={busyProxies}
					entries={data.authProxyAccounts}
					kind="auth_proxy"
					onDelete={(entry) => void deleteProxy(entry as AuthProxyAccount)}
					onEdit={(entry) => setProxyEditor(entry as AuthProxyAccount)}
					onToggle={(entry) => void toggleProxy(entry as AuthProxyAccount)}
					routes={data.routes}
					targets={{ accounts: data.codexAccounts, groups: data.accountGroups }}
				/>
			) : null}
			{activePage === "account" ? (
				<>
					<CodexAccounts
						accounts={data.codexAccounts}
						busyAccounts={busyAccounts}
						loginError={deviceError}
						loginFlow={deviceFlow}
						loginLoading={deviceLoading}
						now={now}
						onCancelLogin={cancelDeviceLogin}
						onDelete={(account) => void deleteCodexAccount(account)}
						onRefresh={(account) => void refreshSubscription(account)}
						onStartLogin={() => void startDeviceLogin()}
						onUpdate={(account, value) => void updateCodexAccount(account, value)}
						subscriptionErrors={subscriptionErrors}
						subscriptionLoading={subscriptionLoading}
						subscriptions={subscriptions}
					/>
					<AccountGroups
						accounts={data.codexAccounts}
						groups={data.accountGroups}
						onChange={saveAccountGroups}
						routes={data.routes}
						saving={accountGroupsSaving}
					/>
				</>
			) : null}
			{activePage === "pricing" ? (
				<ModelPricingCard
					error={pricingError}
					loading={pricingLoading}
					onSave={(prices) => void savePricing(prices)}
					onSync={() => void syncPricing()}
					prices={modelPrices}
					saving={pricingSaving}
					syncing={pricingSyncing}
					usedModels={usedModels}
				/>
			) : null}

			{keyEditor ? (
				<ApiKeyEditor
					accounts={data.codexAccounts}
					entry={keyEditor}
					groups={data.accountGroups}
					loading={keySaving}
					onCancel={() => setKeyEditor(null)}
					onSave={(value, target) => void saveApiKey(value, target)}
					route={keyEditor === "new" ? null : consumerRoute(data.routes, "api_key", keyEditor.id)}
				/>
			) : null}
			{proxyEditor ? (
				<ProxyEditor
					accounts={data.codexAccounts}
					entry={proxyEditor}
					groups={data.accountGroups}
					loading={proxySaving}
					onCancel={() => setProxyEditor(null)}
					onSave={(value, target) => void saveProxy(value, target)}
					route={proxyEditor === "new" ? null : consumerRoute(data.routes, "auth_proxy", proxyEditor.id)}
				/>
			) : null}
			{notice ? <StatusToast notice={notice} onClose={() => setNotice(null)} /> : null}
		</ManagementShell>
	);
}

function Overview({
	data,
	now,
	onNavigate,
	usage,
}: {
	data: AdminState;
	now: number;
	onNavigate: (page: ManagementPage) => void;
	usage: UsageDashboard | null;
}) {
	const activeApiKeys = data.apiKeys.filter((entry) => entry.enabled).length;
	const activeProxyAccounts = data.authProxyAccounts.filter((entry) => entry.enabled).length;
	const enabledDownstreams = activeApiKeys + activeProxyAccounts;
	return (
		<div className="overview-cycle-layout">
			<aside className="overview-metrics-column" aria-label="当前周期摘要">
				<OverviewMetricCard
					detail={`Account ID ${activeProxyAccounts} · API Key ${activeApiKeys}`}
					label="下游账户"
					onClick={() => onNavigate("accounts")}
					value={formatCount(enabledDownstreams)}
				/>
				<OverviewMetricCard detail="Codex 周额度周期累计" label="周期内请求数" onClick={() => onNavigate("usage")} value={usage ? formatCount(usage.totals.requests) : "—"} />
				<OverviewMetricCard detail="包含输入、输出与缓存 Token" label="周期内 Token 用量" onClick={() => onNavigate("usage")} value={usage ? formatTokens(usage.totals.totalTokens) : "—"} />
				<OverviewMetricCard detail={usage?.unpricedModels.length ? `${usage.unpricedModels.length} 个模型尚未计价` : "按模型价格配置计算"} label="周期内成本" onClick={() => onNavigate("pricing")} value={usage ? formatCost(usage.totals.costUsd) : "—"} />
			</aside>
			<section className="overview-visuals-column" aria-label="当前周期活动与成本分布">
				{usage ? (
					<>
						<ActivityHeatmaps now={now} stacked usage={usage} />
						<DownstreamCostDonut usage={usage} />
					</>
				) : <div className="card center-state overview-visual-loading"><span className="spinner" aria-hidden="true" />正在读取当前周期…</div>}
			</section>
		</div>
	);
}

function OverviewMetricCard({
	detail,
	label,
	onClick,
	value,
}: {
	detail: string;
	label: string;
	onClick: () => void;
	value: string;
}) {
	return (
		<button className="overview-metric-card" onClick={onClick} type="button">
			<span>{label}</span>
			<strong>{value}</strong>
			<small>{detail}</small>
		</button>
	);
}

function UsagePanel({
	accounts,
	apiKeys,
	codexAccounts,
	error,
	groups,
	identity,
	loading,
	now,
	onIdentityChange,
	onRangeChange,
	onRefresh,
	range,
	usage,
}: {
	accounts: AuthProxyAccount[];
	apiKeys: ClientApiKey[];
	codexAccounts: CodexAccount[];
	error: string | null;
	groups: AccountGroup[];
	identity: UsageIdentityFilter | null;
	loading: boolean;
	now: number;
	onIdentityChange: (identity: UsageIdentityFilter | null) => void;
	onRangeChange: (range: UsageRange) => void;
	onRefresh: () => void;
	range: UsageRange;
	usage: UsageDashboard | null;
}) {
	const totals = usage?.totals;
	return (
		<section className="card usage-card" aria-label="用量详情">
			<div className="card-header unified-section-header">
				<div className="usage-card-actions">
					<div className="usage-identity-control">
						<span id="usage-identity-label">筛选对象</span>
						<Select
							disabled={loading}
							onValueChange={(value) => onIdentityChange(value === ALL_USAGE_IDENTITIES_VALUE ? null : parseUsageIdentityValue(value))}
							value={usageIdentityValue(identity) || ALL_USAGE_IDENTITIES_VALUE}
						>
							<SelectTrigger aria-labelledby="usage-identity-label" className="w-[clamp(13rem,30vw,23rem)] max-w-full data-[size=default]:h-[2.35rem] max-[48rem]:w-full">
								<SelectValue />
							</SelectTrigger>
							<SelectContent align="end" position="popper">
								<SelectGroup>
									<SelectItem value={ALL_USAGE_IDENTITIES_VALUE}>全部</SelectItem>
								</SelectGroup>
								{apiKeys.length || accounts.length || codexAccounts.length || groups.length ? <SelectSeparator /> : null}
								{apiKeys.length ? (
									<SelectGroup>
										<SelectLabel>API Keys</SelectLabel>
										{apiKeys.map((entry) => <SelectItem key={entry.id} value={usageIdentityValue({ identityType: "api_key", identityId: entry.id })}>{entry.name}</SelectItem>)}
									</SelectGroup>
								) : null}
								{accounts.length ? (
									<SelectGroup>
										<SelectLabel>下游账户</SelectLabel>
										{accounts.map((entry) => <SelectItem key={entry.id} value={usageIdentityValue({ identityType: "auth_proxy", identityId: entry.id })}>{entry.name}</SelectItem>)}
									</SelectGroup>
								) : null}
								{codexAccounts.length ? (
									<SelectGroup>
										<SelectLabel>Codex 账户</SelectLabel>
										{codexAccounts.map((entry) => <SelectItem key={entry.id} value={usageIdentityValue({ identityType: "codex_account", identityId: entry.id })}>{entry.name}</SelectItem>)}
									</SelectGroup>
								) : null}
								{groups.length ? (
									<SelectGroup>
										<SelectLabel>账户组</SelectLabel>
										{groups.map((entry) => <SelectItem key={entry.id} value={usageIdentityValue({ identityType: "account_group", identityId: entry.id })}>{entry.name}</SelectItem>)}
									</SelectGroup>
								) : null}
							</SelectContent>
						</Select>
					</div>
					<div className="usage-range-control">
						<span id="usage-range-label">统计范围</span>
						<Select disabled={loading} onValueChange={(value) => onRangeChange(value as UsageRange)} value={range}>
							<SelectTrigger aria-labelledby="usage-range-label" className="data-[size=default]:h-[2.35rem] max-[28rem]:w-full">
								<SelectValue />
							</SelectTrigger>
							<SelectContent align="end" position="popper">
								<SelectGroup>
									<SelectItem value="cycle">当前周期</SelectItem>
									<SelectItem value="24h">最近 24 小时</SelectItem>
									<SelectItem value="7d">最近 7 天</SelectItem>
									<SelectItem value="30d">最近 30 天</SelectItem>
									<SelectItem value="all">全部</SelectItem>
								</SelectGroup>
							</SelectContent>
						</Select>
					</div>
					<button aria-label="刷新用量" className="icon-button" disabled={loading} onClick={onRefresh} type="button"><RefreshIcon spinning={loading} /></button>
				</div>
			</div>
			{error ? <div className="inline-alert error-alert usage-alert">{error}</div> : null}
			{loading && !usage ? <div className="center-state usage-loading"><span className="spinner" />正在读取用量…</div> : null}
			{usage && totals ? (
				<div className={loading ? "usage-content is-refreshing" : "usage-content"}>
					<div className="usage-summary-grid">
						<UsageMetric label="请求数" value={formatCount(totals.requests)} tone="blue" />
						<UsageMetric label="输入 Token" value={formatTokens(totals.inputTokens)} detail={`缓存命中 ${formatTokens(totals.cachedInputTokens)}`} tone="teal" />
						<UsageMetric label="总 Token" value={formatTokens(totals.totalTokens)} tone="violet" />
						<UsageMetric label="总成本" value={formatCost(totals.costUsd)} detail={usage.unpricedModels.length ? `${usage.unpricedModels.length} 个模型未计价` : undefined} tone="orange" />
					</div>
					<div className="usage-range-caption"><strong>{rangeLabel(usage.range)}</strong><span>{formatDate(usage.startAt)} — {formatDate(usage.endAt)}</span></div>
					<ActivityHeatmaps now={now} usage={usage} />
					<UsageLineCharts now={now} usage={usage} />
					<UsageBreakdownDonuts usage={usage} />
					<div className="usage-events">
						<div className="usage-section-heading"><strong>最近请求</strong></div>
						<ScrollArea className="table-wrap" scrollbars="horizontal">
							<table className="usage-events-table"><thead><tr><th>时间</th><th>调用身份</th><th>路由目标</th><th>模型</th><th>Token</th><th>成本</th></tr></thead><tbody>{usage.recentEvents.map((event) => <tr key={event.id}><td><time dateTime={new Date(event.recordedAt).toISOString()}>{formatCompactDate(event.recordedAt)}</time></td><td><strong>{event.identityName}</strong><small>{usageIdentityLabel(event.identityType)}</small></td><td><strong>{event.accountGroupName || event.codexAccountName || "—"}</strong>{event.accountGroupName || event.codexAccountName ? <small>{event.accountGroupName ? event.codexAccountName : "Codex 账户"}</small> : null}</td><td><code>{event.model}</code></td><td><strong>{formatTokens(event.totalTokens)}</strong><small>{statusLabel(event.status)}</small></td><td><strong>{formatCost(event.costUsd)}</strong></td></tr>)}</tbody></table>
						</ScrollArea>
					</div>
				</div>
			) : null}
		</section>
	);
}

function UsageMetric({ label, value, detail, tone }: { label: string; value: string; detail?: string | undefined; tone: "blue" | "teal" | "violet" | "orange" }) {
	return <div className={`usage-metric usage-metric-${tone}`}><span>{label}</span><strong>{value}</strong>{detail ? <small>{detail}</small> : null}</div>;
}

type IdentityEntry = ClientApiKey | AuthProxyAccount;

function IdentityTable({
	busy,
	entries,
	kind,
	onDelete,
	onEdit,
	onToggle,
	routes,
	targets,
}: {
	busy: ReadonlySet<string>;
	entries: IdentityEntry[];
	kind: RouteConsumerType;
	onDelete: (entry: IdentityEntry) => void;
	onEdit: (entry: IdentityEntry) => void;
	onToggle: (entry: IdentityEntry) => void;
	routes: RouteAssignment[];
	targets: { accounts: CodexAccount[]; groups: AccountGroup[] };
}) {
	const isKey = kind === "api_key";
	const [visibleKeys, setVisibleKeys] = useState<ReadonlySet<string>>(new Set());

	function toggleKeyVisibility(id: string): void {
		setVisibleKeys((current) => {
			const next = new Set(current);
			if (next.has(id)) next.delete(id); else next.add(id);
			return next;
		});
	}

	if (entries.length === 0) {
		return (
			<section className="card empty-state identity-empty-state" aria-label={isKey ? "API Keys" : "下游账户"}>
				<strong>{isKey ? "暂无 API Key" : "暂无下游账户"}</strong>
			</section>
		);
	}

	return (
		<ScrollArea className="table-wrap identity-table-wrap" aria-label={isKey ? "API Keys" : "下游账户"} role="region" scrollbars="horizontal">
			<table className="unified-identity-table"><thead><tr><th>名称</th><th>{isKey ? "Key" : "account_id"}</th><th>账户 / 账户组</th><th>状态</th><th>操作</th></tr></thead><tbody>{entries.map((entry) => {
					const route = routes.find((item) => item.consumerType === kind && item.consumerId === entry.id);
					const key = isKey ? (entry as ClientApiKey).key : null;
					const visible = key !== null && visibleKeys.has(entry.id);
					return (
						<tr className={entry.enabled ? "" : "row-disabled"} key={entry.id}>
							<td><strong>{entry.name}</strong></td>
							<td>
								{key !== null ? (
									<div className="key-value-cell">
										<code>{visible ? key : maskApiKey(key)}</code>
										<button
											aria-label={visible ? `隐藏 ${entry.name}` : `显示 ${entry.name}`}
											className="icon-button small-icon-button"
											onClick={() => toggleKeyVisibility(entry.id)}
											title={visible ? "隐藏 Key" : "显示 Key"}
											type="button"
										>
											<EyeIcon off={visible} />
										</button>
										<button
											aria-label={`复制 ${entry.name}`}
											className="icon-button small-icon-button"
											onClick={() => void navigator.clipboard.writeText(key)}
											title="复制 Key"
											type="button"
										>
											<CopyIcon />
										</button>
									</div>
								) : <code title={(entry as AuthProxyAccount).accountId}>{(entry as AuthProxyAccount).accountId}</code>}
							</td>
							<td><span className={`route-state-badge ${route ? "assigned" : "unassigned"}`}>{route ? routeTargetName(route, targets.accounts, targets.groups) : "未分配"}</span></td>
							<td>
								<label className="key-status-switch" title={entry.enabled ? "停用" : "启用"}>
									<input
										aria-label={`${entry.enabled ? "停用" : "启用"} ${entry.name}`}
										checked={entry.enabled}
										className="switch-control"
										disabled={busy.has(entry.id)}
										onChange={() => onToggle(entry)}
										type="checkbox"
									/>
								</label>
							</td>
							<td>
								<div className="table-actions">
									<button className="button button-secondary button-compact" disabled={busy.has(entry.id)} onClick={() => onEdit(entry)} type="button">编辑</button>
									<DeleteConfirmationDialog
										description="此操作无法撤销。"
										onConfirm={() => onDelete(entry)}
										title={`删除${isKey ? " API Key" : "下游账户"}“${entry.name}”？`}
										trigger={<button className="button button-danger button-compact" disabled={busy.has(entry.id)} type="button">删除</button>}
									/>
								</div>
							</td>
						</tr>
					);
			})}</tbody></table>
		</ScrollArea>
	);
}

function ApiKeyEditor({
	accounts,
	entry,
	groups,
	loading,
	onCancel,
	onSave,
	route,
}: {
	accounts: CodexAccount[];
	entry: Exclude<EditableKey, null>;
	groups: AccountGroup[];
	loading: boolean;
	onCancel: () => void;
	onSave: (value: ClientApiKeyInput, target: RouteTargetSelection) => void;
	route: RouteAssignment | null;
}) {
	const initial = entry === "new" ? { name: "", key: generateApiKey(), enabled: true } : entry;
	const [name, setName] = useState(initial.name);
	const [key, setKey] = useState(initial.key);
	const [enabled, setEnabled] = useState(initial.enabled);
	const [visible, setVisible] = useState(entry === "new");
	const [target, setTarget] = useState<RouteTargetSelection>(route);

	function submit(event: FormEvent<HTMLFormElement>): void {
		event.preventDefault();
		if (validApiKey(key) && name.trim()) onSave({ name: name.trim(), key, enabled }, target);
	}

	return (
		<Dialog open onOpenChange={(open) => { if (!open && !loading) onCancel(); }}>
			<DialogContent className="flex max-h-[calc(100svh-2rem)] flex-col p-0 sm:max-w-lg" showCloseButton={!loading}>
				<ScrollArea className="min-h-0 flex-1">
					<div className="grid gap-4 p-4">
						<DialogHeader>
							<DialogTitle>{entry === "new" ? "添加 API Key" : "编辑 API Key"}</DialogTitle>
							<DialogDescription>配置 API Key、路由目标与启用状态。</DialogDescription>
						</DialogHeader>
						<form className="editor-form" onSubmit={submit}>
					<label htmlFor="api-key-name">
						<span>名称</span>
						<input autoFocus disabled={loading} id="api-key-name" maxLength={100} onChange={(event) => setName(event.target.value)} placeholder="例如：my-laptop" required type="text" value={name} />
					</label>
					<label htmlFor="api-key-value">
						<span>Key</span>
						<div className="input-with-action">
							<input autoComplete="off" disabled={loading} id="api-key-value" maxLength={MAX_API_KEY_LENGTH} minLength={MIN_API_KEY_LENGTH} onChange={(event) => setKey(event.target.value)} required spellCheck={false} type={visible ? "text" : "password"} value={key} />
							<button aria-label={visible ? "隐藏" : "显示"} className="input-action" onClick={() => setVisible((value) => !value)} type="button"><EyeIcon off={visible} /></button>
						</div>
					</label>
					<AccountTargetSelect accounts={accounts} groups={groups} loading={loading} onChange={setTarget} target={target} unassignedHint="未分配时，该 API Key 的请求将不可用。" />
					<div className="editor-tools">
						<Button className="shrink-0" disabled={loading} onClick={() => setKey(generateApiKey())} type="button" variant="outline">重新生成</Button>
						<label className="switch-row"><strong>启用</strong><input checked={enabled} className="switch-control" disabled={loading} onChange={(event) => setEnabled(event.target.checked)} type="checkbox" /></label>
					</div>
							<DialogFooter>
								<DialogClose asChild><Button disabled={loading} type="button" variant="outline">取消</Button></DialogClose>
								<Button disabled={loading || !name.trim() || !validApiKey(key)} type="submit">{loading ? <span className="spinner" /> : null}{loading ? "保存中…" : "保存"}</Button>
							</DialogFooter>
						</form>
					</div>
				</ScrollArea>
			</DialogContent>
		</Dialog>
	);
}

function ProxyEditor({
	accounts,
	entry,
	groups,
	loading,
	onCancel,
	onSave,
	route,
}: {
	accounts: CodexAccount[];
	entry: Exclude<EditableProxy, null>;
	groups: AccountGroup[];
	loading: boolean;
	onCancel: () => void;
	onSave: (value: AuthProxyAccountInput, target: RouteTargetSelection) => void;
	route: RouteAssignment | null;
}) {
	const initial = entry === "new" ? { name: "", accountId: "", enabled: true } : entry;
	const [name, setName] = useState(initial.name);
	const [accountId, setAccountId] = useState(initial.accountId);
	const [enabled, setEnabled] = useState(initial.enabled);
	const [target, setTarget] = useState<RouteTargetSelection>(route);

	function submit(event: FormEvent<HTMLFormElement>): void {
		event.preventDefault();
		if (name.trim() && validAccountId(accountId)) onSave({ name: name.trim(), accountId, enabled }, target);
	}

	return (
		<Dialog open onOpenChange={(open) => { if (!open && !loading) onCancel(); }}>
			<DialogContent className="flex max-h-[calc(100svh-2rem)] flex-col p-0 sm:max-w-lg" showCloseButton={!loading}>
				<ScrollArea className="min-h-0 flex-1">
					<div className="grid gap-4 p-4">
						<DialogHeader>
							<DialogTitle>{entry === "new" ? "添加下游账户" : "编辑下游账户"}</DialogTitle>
							<DialogDescription>配置 account_id、路由目标与启用状态。</DialogDescription>
						</DialogHeader>
						<form className="editor-form" onSubmit={submit}>
					<label htmlFor="proxy-name">
						<span>名称</span>
						<input autoFocus disabled={loading} id="proxy-name" maxLength={100} onChange={(event) => setName(event.target.value)} placeholder="例如：production" required type="text" value={name} />
					</label>
					<label htmlFor="proxy-account-id">
						<span>account_id</span>
						<input autoCapitalize="none" autoComplete="off" disabled={loading} id="proxy-account-id" maxLength={MAX_ACCOUNT_ID_LENGTH} onChange={(event) => setAccountId(event.target.value)} required spellCheck={false} type="text" value={accountId} />
					</label>
					<AccountTargetSelect accounts={accounts} groups={groups} loading={loading} onChange={setTarget} target={target} unassignedHint="未分配时，将继续使用来访请求中的上游凭据。" />
					<label className="switch-row"><strong>启用</strong><input checked={enabled} className="switch-control" disabled={loading} onChange={(event) => setEnabled(event.target.checked)} type="checkbox" /></label>
							<DialogFooter>
								<DialogClose asChild><Button disabled={loading} type="button" variant="outline">取消</Button></DialogClose>
								<Button disabled={loading || !name.trim() || !validAccountId(accountId)} type="submit">{loading ? <span className="spinner" /> : null}{loading ? "保存中…" : "保存"}</Button>
							</DialogFooter>
						</form>
					</div>
				</ScrollArea>
			</DialogContent>
		</Dialog>
	);
}

function AccountTargetSelect({
	accounts,
	groups,
	loading,
	onChange,
	target,
	unassignedHint,
}: {
	accounts: CodexAccount[];
	groups: AccountGroup[];
	loading: boolean;
	onChange: (target: RouteTargetSelection) => void;
	target: RouteTargetSelection;
	unassignedHint: string;
}) {
	return (
		<label className="route-target-field" htmlFor="identity-account-target">
			<span id="identity-account-target-label">账户或账户组</span>
			<Select
				disabled={loading}
				onValueChange={(value) => onChange(parseRouteTargetValue(value === UNASSIGNED_ROUTE_TARGET_VALUE ? "" : value))}
				value={routeTargetValue(target) || UNASSIGNED_ROUTE_TARGET_VALUE}
			>
				<SelectTrigger aria-labelledby="identity-account-target-label" className="w-full data-[size=default]:h-[3.15rem]" id="identity-account-target">
					<SelectValue />
				</SelectTrigger>
				<SelectContent position="popper">
					<SelectGroup>
						<SelectItem value={UNASSIGNED_ROUTE_TARGET_VALUE}>未分配</SelectItem>
					</SelectGroup>
					{groups.length || accounts.length ? <SelectSeparator /> : null}
					{groups.length ? (
						<SelectGroup>
							<SelectLabel>账户组</SelectLabel>
							{groups.map((group) => <SelectItem key={group.id} value={`group:${group.id}`}>{group.name}</SelectItem>)}
						</SelectGroup>
					) : null}
					{accounts.length ? (
						<SelectGroup>
							<SelectLabel>单个 Codex 账户</SelectLabel>
							{accounts.map((account) => (
								<SelectItem disabled={!account.enabled} key={account.id} value={`account:${account.id}`}>
									{account.name}{account.enabled ? "" : "（已禁用）"}
								</SelectItem>
							))}
						</SelectGroup>
					) : null}
				</SelectContent>
			</Select>
			<small>{groups.length === 0 && accounts.length === 0 ? "请先在 Codex 账户页添加账户或账户组。" : unassignedHint}</small>
		</label>
	);
}

function LoginView({ error, loading, onSubmit }: { error: string | null; loading: boolean; onSubmit: (secret: string) => void }) {
	const [secret, setSecret] = useState("");
	const [visible, setVisible] = useState(false);
	function submit(event: FormEvent<HTMLFormElement>): void { event.preventDefault(); if (secret && !loading) onSubmit(secret); }
	return <div className="auth-shell"><aside className="auth-aside"><div className="auth-aside-title"><span>Codex</span><span>Router</span></div></aside><div className="auth-main"><main className="auth-card"><div className="auth-mobile-brand"><ProductMark compact /><strong>Codex Router</strong></div><h1>登录管理面板</h1>{error ? <div className="inline-alert error-alert">{error}</div> : null}<form className="auth-form" onSubmit={submit}><label htmlFor="admin-secret">管理密钥</label><div className="input-with-action"><input autoFocus disabled={loading} id="admin-secret" onChange={(event) => setSecret(event.target.value)} required type={visible ? "text" : "password"} value={secret} /><button aria-label={visible ? "隐藏" : "显示"} className="input-action" onClick={() => setVisible((value) => !value)} type="button"><EyeIcon off={visible} /></button></div><button className="button button-primary auth-submit" disabled={loading || !secret}>{loading ? <span className="spinner" /> : null}{loading ? "登录中…" : "登录"}</button></form></main></div></div>;
}

function LoadingView() {
	return <div className="loading-screen"><ProductMark /><span className="spinner" /><strong>加载中…</strong></div>;
}

function InvalidPath() {
	return <main className="invalid-path"><ProductMark /><h1>页面不存在</h1><p>请使用配置中的管理路径访问控制台。</p></main>;
}

function StatusToast({ notice, onClose }: { notice: Notice; onClose: () => void }) {
	return <div className={`status-toast ${notice.tone}`} role="status"><span className="toast-icon"><NoticeIcon success={notice.tone === "success"} /></span><p>{notice.text}</p><button aria-label="关闭通知" onClick={onClose} type="button"><CloseIcon /></button></div>;
}

function routeTargetName(route: RouteAssignment, accounts: CodexAccount[], groups: AccountGroup[]): string {
	return route.targetType === "account"
		? `单账户 · ${accounts.find((entry) => entry.id === route.targetId)?.name ?? "账户已移除"}`
		: groups.find((entry) => entry.id === route.targetId)?.name ?? "账户组已移除";
}

function consumerRoute(routes: RouteAssignment[], kind: RouteConsumerType, id: string): RouteAssignment | null {
	return routes.find((route) => route.consumerType === kind && route.consumerId === id) ?? null;
}

function replaceConsumerRoute(
	routes: RouteAssignment[],
	consumerType: RouteConsumerType,
	consumerId: string,
	target: RouteTargetSelection,
): RouteAssignment[] {
	const retained = routes.filter((route) => route.consumerType !== consumerType || route.consumerId !== consumerId);
	return target ? [...retained, { consumerType, consumerId, ...target }] : retained;
}

function routeTargetValue(target: RouteTargetSelection): string {
	return target ? `${target.targetType}:${target.targetId}` : "";
}

function parseRouteTargetValue(value: string): RouteTargetSelection {
	const separator = value.indexOf(":");
	if (separator < 1) return null;
	const targetType = value.slice(0, separator);
	const targetId = value.slice(separator + 1);
	if (!targetId || (targetType !== "account" && targetType !== "group")) return null;
	return { targetType, targetId };
}

function invalidAdminResponse(): AdminApiError {
	return new AdminApiError("管理服务返回了无法识别的数据。", 502, "invalid_admin_response");
}

function managementBasePath(pathname: string): string | null {
	if (!MANAGEMENT_PATH_PATTERN.test(pathname)) return null;
	return pathname.endsWith("/") ? pathname.slice(0, -1) : pathname;
}

function managementPageFromSearch(search: string): ManagementPage {
	const page = new URLSearchParams(search).get("page");
	if (page === "routing") return "account";
	return page === "usage" || page === "pricing" || page === "api-keys" || page === "accounts" || page === "account" ? page : "overview";
}

function managementPageTitle(page: ManagementPage): string {
	if (page === "usage") return "用量分析";
	if (page === "pricing") return "模型价格";
	if (page === "api-keys") return "API Keys";
	if (page === "accounts") return "下游账户";
	if (page === "account") return "Codex 账户";
	return "运行概览";
}

function usageIdentityValue(identity: UsageIdentityFilter | null): string {
	return identity ? `${identity.identityType}:${encodeURIComponent(identity.identityId)}` : "";
}

function parseUsageIdentityValue(value: string): UsageIdentityFilter | null {
	if (!value) return null;
	const separator = value.indexOf(":");
	if (separator < 1) return null;
	const kind = value.slice(0, separator);
	if (!isUsageIdentityType(kind)) return null;
	try {
		const identityId = decodeURIComponent(value.slice(separator + 1));
		return identityId ? { identityType: kind, identityId } : null;
	} catch {
		return null;
	}
}

function isUsageIdentityType(value: string): value is UsageIdentityType {
	return value === "api_key" || value === "auth_proxy" || value === "codex_account" || value === "account_group";
}

function usageIdentityLabel(value: "api_key" | "auth_proxy"): string {
	return value === "api_key" ? "API Key" : "下游账户";
}

function statusLabel(value: "completed" | "incomplete" | "failed"): string {
	return value === "completed" ? "已完成" : value === "incomplete" ? "未完整完成" : "失败";
}

function rangeLabel(value: UsageRange): string {
	return value === "cycle" ? "当前周期" : value === "24h" ? "最近 24 小时" : value === "7d" ? "最近 7 天" : value === "30d" ? "最近 30 天" : "全部时间";
}

function errorMessage(error: unknown, fallback: string): string {
	return error instanceof AdminApiError || error instanceof Error ? error.message : fallback;
}

function withSetValue(current: ReadonlySet<string>, value: string, add: boolean): ReadonlySet<string> {
	const next = new Set(current);
	if (add) next.add(value); else next.delete(value);
	return next;
}

function omitKey<T>(record: Record<string, T>, key: string): Record<string, T> {
	const next = { ...record };
	delete next[key];
	return next;
}

function formatCount(value: number): string {
	return INTEGER_FORMAT.format(Math.max(0, Number.isFinite(value) ? value : 0));
}

function formatTokens(value: number): string {
	const normalized = Math.max(0, Number.isFinite(value) ? value : 0);
	return normalized < 10_000 ? INTEGER_FORMAT.format(normalized) : COMPACT_NUMBER_FORMAT.format(normalized);
}

function formatDate(value: number): string {
	return new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(new Date(value));
}

function formatCompactDate(value: number): string {
	return new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false }).format(new Date(value));
}

function validApiKey(value: string): boolean {
	return value.length >= MIN_API_KEY_LENGTH && value.length <= MAX_API_KEY_LENGTH && /[A-Za-z]/.test(value) && /[0-9]/.test(value) && /[^A-Za-z0-9\s]/.test(value);
}

function validAccountId(value: string): boolean {
	return value.length > 0 && value.length <= MAX_ACCOUNT_ID_LENGTH && Array.from(value).every((character) => { const code = character.charCodeAt(0); return code >= 0x21 && code <= 0x7e; });
}

function maskApiKey(value: string): string {
	return value.length < 9 ? "••••••" : `${value.slice(0, 3)}••••••••${value.slice(-4)}`;
}

function generateApiKey(): string {
	const alphabet = "abcdefghijklmnopqrstuvwxyz0123456789";
	const bytes = new Uint8Array(32);
	for (;;) {
		let value = "";
		while (value.length < GENERATED_API_KEY_LENGTH) {
			crypto.getRandomValues(bytes);
			for (const byte of bytes) {
				if (byte < 252) value += alphabet[byte % alphabet.length];
				if (value.length === GENERATED_API_KEY_LENGTH) break;
			}
		}
		if (/[a-z]/.test(value) && /[0-9]/.test(value)) return `sk-${value}`;
	}
}

function PlusIcon() {
	return <svg className="icon" aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.8" viewBox="0 0 24 24"><path d="M12 5v14" /><path d="M5 12h14" /></svg>;
}

function RefreshIcon({ spinning }: { spinning: boolean }) {
	return <svg className={`icon${spinning ? " icon-spinning" : ""}`} aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24"><path d="M20 11a8 8 0 1 0-2.3 5.7" /><path d="M20 4v7h-7" /></svg>;
}

function CloseIcon() {
	return <svg className="icon" aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.8" viewBox="0 0 24 24"><path d="m6 6 12 12" /><path d="M18 6 6 18" /></svg>;
}

function EyeIcon({ off }: { off: boolean }) {
	return <svg className="icon" aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">{off ? <><path d="m3 3 18 18" /><path d="M10.6 6.15A10.6 10.6 0 0 1 12 6c6.5 0 10 6 10 6a16.8 16.8 0 0 1-3 3.8" /><path d="M6.6 6.6C3.5 8.4 2 12 2 12s3.5 6 10 6a10.7 10.7 0 0 0 3.4-.55" /></> : <><path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12Z" /><circle cx="12" cy="12" r="2.5" /></>}</svg>;
}

function CopyIcon() {
	return <svg className="icon" aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24"><rect height="13" rx="2" width="13" x="8" y="8" /><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3" /></svg>;
}

function NoticeIcon({ success }: { success: boolean }) {
	return <svg className="icon" aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">{success ? <path d="m5 12 4.2 4.2L19 6.5" /> : <><path d="M12 9v4" /><path d="M12 17h.01" /><path d="M10.3 3.9 2.4 18a2 2 0 0 0 1.75 3h15.7a2 2 0 0 0 1.75-3L13.7 3.9a2 2 0 0 0-3.4 0Z" /></>}</svg>;
}

export default App;
