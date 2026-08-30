import {
	useEffect,
	useState,
	type MouseEvent,
	type ReactNode,
} from "react";
import type {
	OAuthStatus,
	SubscriptionInfo,
	SubscriptionMetadata,
} from "./admin-api";

export type ManagementPage =
	| "overview"
	| "usage"
	| "api-keys"
	| "accounts"
	| "account";

type ShellIconName =
	| "home"
	| "usage"
	| "key"
	| "accounts"
	| "account"
	| "info"
	| "logout"
	| "menu";

interface ManagementShellProps {
	activePage: ManagementPage;
	basePath: string;
	children: ReactNode;
	mainAccount: OAuthStatus | null;
	mainAccountSubscription: SubscriptionInfo | SubscriptionMetadata | null;
	now: number;
	onLogout: () => void;
	onNavigate: (page: ManagementPage) => void;
}

const PAGE_COPY: Record<ManagementPage, { title: string; description: string }> = {
	overview: {
		title: "运行概览",
		description: "快速检查账户连接、调用身份与近期请求状态。",
	},
	usage: {
		title: "用量分析",
		description: "按时间、API Key 或下游 account id 查看完整 Token 消耗。",
	},
	"api-keys": {
		title: "API Keys",
		description: "创建和维护下游客户端访问 Codex Router 的密钥。",
	},
	accounts: {
		title: "下游账户",
		description: "管理 auth proxy account id、独立登录与启停状态。",
	},
	account: {
		title: "主账户",
		description: "维护 Codex OAuth 登录，并查看订阅计划与额度窗口。",
	},
};

export default function ManagementShell({
	activePage,
	basePath,
	children,
	mainAccount,
	mainAccountSubscription,
	now,
	onLogout,
	onNavigate,
}: ManagementShellProps) {
	const [mobileOpen, setMobileOpen] = useState(false);
	const pageCopy = PAGE_COPY[activePage];

	useEffect(() => {
		if (!mobileOpen) return;
		const closeOnEscape = (event: KeyboardEvent) => {
			if (event.key === "Escape") setMobileOpen(false);
		};
		window.addEventListener("keydown", closeOnEscape);
		return () => window.removeEventListener("keydown", closeOnEscape);
	}, [mobileOpen]);

	function navigate(event: MouseEvent<HTMLAnchorElement>, page: ManagementPage): void {
		if (
			event.button !== 0 ||
			event.metaKey ||
			event.ctrlKey ||
			event.shiftKey ||
			event.altKey
		) {
			return;
		}
		event.preventDefault();
		setMobileOpen(false);
		onNavigate(page);
	}

	const shellClassName = mobileOpen
		? "management-shell mobile-sidebar-open"
		: "management-shell";

	return (
		<div className={shellClassName}>
			<button
				aria-label="关闭导航"
				className="sidebar-backdrop"
				onClick={() => setMobileOpen(false)}
				type="button"
			/>

			<aside className="management-sidebar" id="management-sidebar">
				<header className="sidebar-header">
					<a
						className="sidebar-brand"
						href={managementPageHref(basePath, "overview")}
						onClick={(event) => navigate(event, "overview")}
						aria-label="Codex Router 首页"
					>
						<ProductMark />
						<span className="sidebar-brand-copy">
							<strong>Codex Router</strong>
							<small>Management</small>
						</span>
					</a>
				</header>

				<nav className="sidebar-navigation" aria-label="主导航">
					<NavGroup label="运行">
						<NavItem activePage={activePage} basePath={basePath} icon="home" label="概览" onNavigate={navigate} page="overview" />
						<NavItem activePage={activePage} basePath={basePath} icon="usage" label="用量分析" onNavigate={navigate} page="usage" />
					</NavGroup>
					<NavGroup label="调用身份">
						<NavItem activePage={activePage} basePath={basePath} icon="key" label="API Keys" onNavigate={navigate} page="api-keys" />
						<NavItem activePage={activePage} basePath={basePath} icon="accounts" label="下游账户" onNavigate={navigate} page="accounts" />
					</NavGroup>
					<NavGroup label="账户">
						<NavItem activePage={activePage} basePath={basePath} icon="account" label="主账户" onNavigate={navigate} page="account" />
					</NavGroup>
				</nav>

				<div className="sidebar-footer">
					<div className="sidebar-service-state">
						<span className="sidebar-footer-copy">
							<strong title={mainAccount?.email ?? undefined}>
								{mainAccount ? mainAccount.email ?? "未提供邮箱" : "尚未登陆"}
							</strong>
							{mainAccount ? (
								<span className="sidebar-account-plan">
									<small>{formatPlanType(mainAccountSubscription?.planType)}</small>
									<AccountPlanInfo
										now={now}
										oauth={mainAccount}
										subscription={mainAccountSubscription}
									/>
								</span>
							) : (
								<small>前往主帐户页面登陆</small>
							)}
						</span>
					</div>
					<button className="sidebar-logout" onClick={onLogout} type="button" title="退出">
						<ShellIcon name="logout" />
						<span>退出管理</span>
					</button>
				</div>
			</aside>

			<section className="panel-workspace">
				<main className="panel-main">
					<section className="dashboard-heading" aria-labelledby="dashboard-title">
						<div className="dashboard-heading-copy">
							<button
								aria-controls="management-sidebar"
								aria-expanded={mobileOpen}
								aria-label="打开导航"
								className="workspace-menu-button"
								onClick={() => setMobileOpen(true)}
								type="button"
							>
								<ShellIcon name="menu" />
							</button>
							<div>
								<p className="page-eyebrow">{pageEyebrow(activePage)}</p>
								<h1 id="dashboard-title">{pageCopy.title}</h1>
								<p className="dashboard-description">{pageCopy.description}</p>
							</div>
						</div>
					</section>

					<div className={`dashboard-content page-${activePage}`}>{children}</div>
				</main>
			</section>
		</div>
	);
}

function AccountPlanInfo({
	now,
	oauth,
	subscription,
}: {
	now: number;
	oauth: OAuthStatus;
	subscription: SubscriptionInfo | SubscriptionMetadata | null;
}) {
	const info = isSubscriptionInfo(subscription) ? subscription : null;
	const credits = info?.rateLimitResetCredits;
	const availableCredits = credits?.availableCount ?? null;
	const applicableCredits = credits?.applicableAvailableCount ?? null;
	const resetCredits =
		availableCredits === null
			? "暂无数据"
			: applicableCredits === null
				? String(Math.max(0, availableCredits))
				: `${Math.max(0, availableCredits)} · 可用 ${Math.max(0, applicableCredits)}`;

	return (
		<span className="plan-info sidebar-plan-info">
			<button
				aria-describedby="sidebar-account-plan-details"
				aria-label="查看套餐详情"
				className="plan-info-button"
				type="button"
			>
				<ShellIcon name="info" />
			</button>
			<span
				className="plan-tooltip"
				id="sidebar-account-plan-details"
				role="tooltip"
			>
				<strong>套餐详情</strong>
				<PlanTooltipRow
					label="开始时间"
					value={formatTimestamp(subscription?.subscriptionActiveStart)}
				/>
				<PlanTooltipRow
					danger={timestampExpired(subscription?.subscriptionActiveUntil, now)}
					label="到期时间"
					value={formatTimestamp(subscription?.subscriptionActiveUntil)}
				/>
				<PlanTooltipRow
					danger={timestampExpired(oauth.expiresAt, now)}
					label="Token 到期时间"
					value={formatTimestamp(oauth.expiresAt, "未知")}
				/>
				<PlanTooltipRow label="重置积分" value={resetCredits} />
				<PlanTooltipRow
					label="用量更新时间"
					value={formatTimestamp(info?.fetchedAt)}
				/>
			</span>
		</span>
	);
}

function PlanTooltipRow({
	danger = false,
	label,
	value,
}: {
	danger?: boolean;
	label: string;
	value: string;
}) {
	return (
		<span className="plan-tooltip-row">
			<span>{label}</span>
			<b className={danger ? "danger-text" : undefined}>{value}</b>
		</span>
	);
}

function NavGroup({ label, children }: { label: string; children: ReactNode }) {
	return (
		<div className="sidebar-nav-group">
			<span className="sidebar-group-label">{label}</span>
			<div className="sidebar-nav-items">{children}</div>
		</div>
	);
}

function NavItem({
	activePage,
	basePath,
	icon,
	label,
	onNavigate,
	page,
}: {
	activePage: ManagementPage;
	basePath: string;
	icon: ShellIconName;
	label: string;
	onNavigate: (event: MouseEvent<HTMLAnchorElement>, page: ManagementPage) => void;
	page: ManagementPage;
}) {
	const active = activePage === page;
	return (
		<a
			aria-current={active ? "page" : undefined}
			className={`sidebar-nav-item${active ? " active" : ""}`}
			href={managementPageHref(basePath, page)}
			onClick={(event) => onNavigate(event, page)}
		>
			<span className="sidebar-nav-icon"><ShellIcon name={icon} /></span>
			<span className="sidebar-nav-label">{label}</span>
		</a>
	);
}

export function ProductMark({ compact = false }: { compact?: boolean }) {
	return (
		<span className={`product-mark${compact ? " compact" : ""}`} aria-hidden="true">
			<svg fill="none" viewBox="0 0 32 32">
				<path d="M8.5 9.5h9a5 5 0 0 1 0 10H14" />
				<path d="m11 16-3 3 3 3" />
				<circle cx="9" cy="9.5" r="2" />
				<circle cx="23" cy="19.5" r="2" />
			</svg>
		</span>
	);
}

function pageEyebrow(page: ManagementPage): string {
	if (page === "overview") return "DASHBOARD";
	if (page === "usage") return "OBSERVABILITY";
	if (page === "account") return "CODEX ACCOUNT";
	return "IDENTITIES";
}

function managementPageHref(basePath: string, page: ManagementPage): string {
	return page === "overview" ? basePath : `${basePath}?page=${encodeURIComponent(page)}`;
}

function ShellIcon({ name }: { name: ShellIconName }) {
	return (
		<svg aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">
			{shellIconPaths(name)}
		</svg>
	);
}

function shellIconPaths(name: ShellIconName): ReactNode {
	switch (name) {
		case "home":
			return <><path d="m3 10 9-7 9 7" /><path d="M5 9v11h14V9" /><path d="M9 20v-6h6v6" /></>;
		case "usage":
			return <><path d="M4 19V9" /><path d="M10 19V5" /><path d="M16 19v-7" /><path d="M22 19H2" /></>;
		case "key":
			return <><circle cx="8" cy="15" r="4" /><path d="m11 12 8-8" /><path d="m15 8 2 2" /><path d="m17 6 2 2" /></>;
		case "accounts":
			return <><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" /><path d="M22 21v-2a4 4 0 0 0-3-3.87" /><path d="M16 3.13a4 4 0 0 1 0 7.75" /></>;
		case "account":
			return <><circle cx="12" cy="8" r="4" /><path d="M4 21a8 8 0 0 1 16 0" /></>;
		case "info":
			return <><circle cx="12" cy="12" r="9" /><path d="M12 11v5" /><path d="M12 8h.01" /></>;
		case "logout":
			return <><path d="m16 17 5-5-5-5" /><path d="M21 12H9" /><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" /></>;
		case "menu":
			return <><path d="M4 7h16" /><path d="M4 12h16" /><path d="M4 17h16" /></>;
	}
}

function isSubscriptionInfo(
	value: SubscriptionInfo | SubscriptionMetadata | null,
): value is SubscriptionInfo {
	return value !== null && "windows" in value;
}

function validTimestamp(value: number | null | undefined): value is number {
	return typeof value === "number" && Number.isFinite(value) && value > 0;
}

function timestampExpired(value: number | null | undefined, now: number): boolean {
	return validTimestamp(value) && value <= now;
}

function formatTimestamp(
	value: number | null | undefined,
	fallback = "暂无数据",
): string {
	if (!validTimestamp(value)) return fallback;
	return new Intl.DateTimeFormat("zh-CN", {
		dateStyle: "medium",
		timeStyle: "short",
	}).format(new Date(value));
}

function formatPlanType(value: string | null | undefined): string {
	const normalized = value?.trim().toLowerCase() ?? "";
	switch (normalized) {
		case "pro":
			return "Pro";
		case "prolite":
		case "pro-lite":
		case "pro_lite":
			return "Pro Lite";
		case "plus":
			return "Plus";
		case "team":
			return "Team";
		case "free":
			return "Free";
		default:
			return value || "未知";
	}
}
