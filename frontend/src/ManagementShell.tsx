import { useState, type MouseEvent, type ReactNode } from "react";
import {
	ChartNoAxesColumnIcon,
	CircleDollarSignIcon,
	HomeIcon,
	KeyRoundIcon,
	LogOutIcon,
	MenuIcon,
	UserRoundIcon,
	UsersIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetTitle,
} from "@/components/ui/sheet";

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
		description: "按时间、上游路由目标与下游调用身份查看完整 Token 消耗。",
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
		<div className="management-shell">
			<aside className="management-sidebar" id="management-sidebar-desktop">
				<SidebarContent
					activePage={activePage}
					basePath={basePath}
					navigate={navigate}
					onLogout={onLogout}
				/>
			</aside>

			<Sheet onOpenChange={setMobileOpen} open={mobileOpen}>
				<SheetContent
					className="mobile-management-sidebar w-[min(17rem,calc(100vw-2rem))] gap-0 p-3"
					id="management-sidebar"
					showCloseButton={false}
					side="left"
				>
					<SheetTitle className="sr-only">管理导航</SheetTitle>
					<SheetDescription className="sr-only">前往管理面板的各个页面。</SheetDescription>
					<SidebarContent
						activePage={activePage}
						basePath={basePath}
						navigate={navigate}
						onLogout={onLogout}
					/>
				</SheetContent>
			</Sheet>

			<section className="panel-workspace">
				<ScrollArea className="min-h-0 flex-1">
					<main className="panel-main">
						<section className="dashboard-heading" aria-labelledby="dashboard-title">
							<div className="dashboard-heading-copy">
								<Button
									aria-controls="management-sidebar"
									aria-expanded={mobileOpen}
									aria-label="打开导航"
									className="hidden shrink-0 max-[60rem]:inline-flex"
									onClick={() => setMobileOpen(true)}
									size="icon-lg"
									type="button"
									variant="outline"
								>
									<ShellIcon name="menu" />
								</Button>
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

function SidebarContent({
	activePage,
	basePath,
	navigate,
	onLogout,
}: {
	activePage: ManagementPage;
	basePath: string;
	navigate: (event: MouseEvent<HTMLAnchorElement>, page: ManagementPage) => void;
	onLogout: () => void;
}) {
	return (
		<>
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
				<Button className="min-h-11 w-full justify-start [&_svg]:size-[1.1rem]" onClick={onLogout} title="退出" type="button" variant="ghost">
					<ShellIcon name="logout" />
					<span>退出管理</span>
				</Button>
			</div>
		</>
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
		<Button asChild className="h-auto min-h-[2.55rem] w-full justify-start" variant={active ? "secondary" : "ghost"}>
			<a
				aria-current={active ? "page" : undefined}
				href={managementPageHref(basePath, page)}
				onClick={(event) => onNavigate(event, page)}
			>
				<span className="sidebar-nav-icon"><ShellIcon name={icon} /></span>
				<span className="sidebar-nav-label">{label}</span>
			</a>
		</Button>
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
	switch (name) {
		case "home":
			return <HomeIcon aria-hidden="true" />;
		case "usage":
			return <ChartNoAxesColumnIcon aria-hidden="true" />;
		case "pricing":
			return <CircleDollarSignIcon aria-hidden="true" />;
		case "key":
			return <KeyRoundIcon aria-hidden="true" />;
		case "accounts":
			return <UsersIcon aria-hidden="true" />;
		case "account":
			return <UserRoundIcon aria-hidden="true" />;
		case "logout":
			return <LogOutIcon aria-hidden="true" />;
		case "menu":
			return <MenuIcon aria-hidden="true" />;
	}
}
