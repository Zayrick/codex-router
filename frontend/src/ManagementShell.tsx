import { useEffect, useState, type MouseEvent, type ReactNode } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";

export type ManagementPage =
	| "overview"
	| "usage"
	| "pricing"
	| "api-keys"
	| "accounts"
	| "account";

type ShellIconName =
	| "home"
	| "usage"
	| "pricing"
	| "key"
	| "accounts"
	| "account"
	| "logout"
	| "menu";

interface ManagementShellProps {
	activePage: ManagementPage;
	basePath: string;
	children: ReactNode;
	onLogout: () => void;
	onNavigate: (page: ManagementPage) => void;
	pageAction?: ReactNode;
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
	pricing: {
		title: "模型价格",
		description: "配置模型 Token 单价，用于估算请求成本与用量支出。",
	},
	"api-keys": {
		title: "API Keys",
		description: "创建和维护下游客户端访问 Codex Router 的密钥。",
	},
	accounts: {
		title: "下游账户",
		description: "管理 auth proxy account id 与启停状态。",
	},
	account: {
		title: "Codex 账户",
		description: "维护 Codex OAuth 登录、订阅额度与账户组。",
	},
};

export default function ManagementShell({
	activePage,
	basePath,
	children,
	onLogout,
	onNavigate,
	pageAction,
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

	function navigate(
		event: MouseEvent<HTMLAnchorElement>,
		page: ManagementPage,
	): void {
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

	return (
		<div className={mobileOpen ? "management-shell mobile-sidebar-open" : "management-shell"}>
			<button
				aria-label="关闭导航"
				className="sidebar-backdrop"
				onClick={() => setMobileOpen(false)}
				type="button"
			/>

			<aside className="management-sidebar" id="management-sidebar">
				<header className="sidebar-header">
					<a
						aria-label="Codex Router 首页"
						className="sidebar-brand"
						href={managementPageHref(basePath, "overview")}
						onClick={(event) => navigate(event, "overview")}
					>
						<ProductMark />
						<span className="sidebar-brand-copy">
							<strong>Codex Router</strong>
							<small>Management</small>
						</span>
					</a>
				</header>

				<nav className="sidebar-navigation-shell" aria-label="主导航">
					<ScrollArea className="min-h-0 flex-1">
						<div className="sidebar-navigation">
							<NavGroup label="运行">
								<NavItem activePage={activePage} basePath={basePath} icon="home" label="概览" onNavigate={navigate} page="overview" />
								<NavItem activePage={activePage} basePath={basePath} icon="usage" label="用量分析" onNavigate={navigate} page="usage" />
								<NavItem activePage={activePage} basePath={basePath} icon="pricing" label="模型价格" onNavigate={navigate} page="pricing" />
							</NavGroup>
							<NavGroup label="调用身份">
								<NavItem activePage={activePage} basePath={basePath} icon="key" label="API Keys" onNavigate={navigate} page="api-keys" />
								<NavItem activePage={activePage} basePath={basePath} icon="accounts" label="下游账户" onNavigate={navigate} page="accounts" />
							</NavGroup>
							<NavGroup label="调度">
								<NavItem activePage={activePage} basePath={basePath} icon="account" label="Codex 账户" onNavigate={navigate} page="account" />
							</NavGroup>
						</div>
					</ScrollArea>
				</nav>

				<div className="sidebar-footer">
					<button className="sidebar-logout" onClick={onLogout} type="button" title="退出">
						<ShellIcon name="logout" />
						<span>退出管理</span>
					</button>
				</div>
			</aside>

			<section className="panel-workspace">
				<ScrollArea className="min-h-0 flex-1">
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
							{pageAction ? <div className="dashboard-heading-action">{pageAction}</div> : null}
						</section>

						<div className={`dashboard-content page-${activePage}`}>{children}</div>
					</main>
				</ScrollArea>
			</section>
		</div>
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
	if (page === "pricing") return "COST SETTINGS";
	if (page === "account") return "CODEX ACCOUNTS";
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
		case "pricing":
			return <><circle cx="12" cy="12" r="9" /><path d="M15.5 8.5h-5a2 2 0 0 0 0 4h3a2 2 0 0 1 0 4H8.5" /><path d="M12 6.5v2M12 16.5v2" /></>;
		case "key":
			return <><circle cx="8" cy="15" r="4" /><path d="m11 12 8-8" /><path d="m15 8 2 2" /><path d="m17 6 2 2" /></>;
		case "accounts":
			return <><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" /><path d="M22 21v-2a4 4 0 0 0-3-3.87" /><path d="M16 3.13a4 4 0 0 1 0 7.75" /></>;
		case "account":
			return <><circle cx="12" cy="8" r="4" /><path d="M4 21a8 8 0 0 1 16 0" /></>;
		case "logout":
			return <><path d="m16 17 5-5-5-5" /><path d="M21 12H9" /><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" /></>;
		case "menu":
			return <><path d="M4 7h16" /><path d="M4 12h16" /><path d="M4 17h16" /></>;
	}
}
