import { useState, type MouseEvent, type ReactNode } from "react";
import {
	ChartNoAxesColumnIcon,
	CircleDollarSignIcon,
	GaugeIcon,
	HomeIcon,
	Layers3Icon,
	LogOutIcon,
	MenuIcon,
	ScrollTextIcon,
	Settings2Icon,
	UserRoundIcon,
	UsersIcon,
} from "lucide-react";

import { NAV_GROUPS, PAGE_META, pageHref, type ManagementPage } from "@/admin/pages";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";

const PAGE_ICONS: Record<ManagementPage, ReactNode> = {
	overview: <HomeIcon aria-hidden="true" />,
	logs: <ScrollTextIcon aria-hidden="true" />,
	usage: <ChartNoAxesColumnIcon aria-hidden="true" />,
	performance: <GaugeIcon aria-hidden="true" />,
	codex: <UserRoundIcon aria-hidden="true" />,
	groups: <Layers3Icon aria-hidden="true" />,
	users: <UsersIcon aria-hidden="true" />,
	pricing: <CircleDollarSignIcon aria-hidden="true" />,
	settings: <Settings2Icon aria-hidden="true" />,
};

interface ManagementShellProps {
	activePage: ManagementPage;
	basePath: string;
	children: ReactNode;
	onLogout: () => void;
	onNavigate: (page: ManagementPage) => void;
	pageAction?: ReactNode;
}

/** Figma "Sidebar v2" + page heading around every management page. */
export default function ManagementShell({ activePage, basePath, children, onLogout, onNavigate, pageAction }: ManagementShellProps) {
	const [mobileOpen, setMobileOpen] = useState(false);
	const meta = PAGE_META[activePage];

	function navigate(event: MouseEvent<HTMLAnchorElement>, page: ManagementPage): void {
		if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
		event.preventDefault();
		setMobileOpen(false);
		onNavigate(page);
	}

	return (
		<div className="management-shell">
			<aside className="management-sidebar" id="management-sidebar-desktop">
				<SidebarContent activePage={activePage} basePath={basePath} navigate={navigate} onLogout={onLogout} />
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
					<SidebarContent activePage={activePage} basePath={basePath} navigate={navigate} onLogout={onLogout} />
				</SheetContent>
			</Sheet>

			<section className="panel-workspace">
				<ScrollArea className="min-h-0 flex-1" data-console-scroll="">
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
									<MenuIcon aria-hidden="true" />
								</Button>
								<div>
									{meta.eyebrow ? <p className="page-eyebrow">{meta.eyebrow}</p> : null}
									<h1 id="dashboard-title">{meta.title}</h1>
									<p className="dashboard-description">{meta.description}</p>
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
				<a aria-label="Codex Router 首页" className="sidebar-brand" href={pageHref(basePath, "overview")} onClick={(event) => navigate(event, "overview")}>
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
						{NAV_GROUPS.map((group) => (
							<div className="sidebar-nav-group" key={group.label}>
								<span className="sidebar-group-label">{group.label}</span>
								<div className="sidebar-nav-items">
									{group.pages.map((page) => {
										const active = activePage === page;
										return (
											<Button asChild className="h-auto min-h-[2.55rem] w-full justify-start" key={page} variant={active ? "secondary" : "ghost"}>
												<a aria-current={active ? "page" : undefined} href={pageHref(basePath, page)} onClick={(event) => navigate(event, page)}>
													<span className="sidebar-nav-icon">{PAGE_ICONS[page]}</span>
													<span className="sidebar-nav-label">{PAGE_META[page].label}</span>
												</a>
											</Button>
										);
									})}
								</div>
							</div>
						))}
					</div>
				</ScrollArea>
			</nav>

			<div className="sidebar-footer">
				<Button className="min-h-11 w-full justify-start [&_svg]:size-[1.1rem]" onClick={onLogout} title="退出" type="button" variant="ghost">
					<LogOutIcon aria-hidden="true" />
					<span>退出管理</span>
				</Button>
			</div>
		</>
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
