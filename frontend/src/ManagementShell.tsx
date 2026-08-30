import { useEffect, useState, type ReactNode } from "react";

type ShellIconName = "home" | "logout" | "menu";

interface ManagementShellProps {
	activeApiKeys: number;
	activeProxyAccounts: number;
	children: ReactNode;
	mainAccountConnected: boolean;
	onLogout: () => void;
	requestCount: number | null;
	totalApiKeys: number;
	totalProxyAccounts: number;
	usageRangeLabel: string;
}

export default function ManagementShell({
	activeApiKeys,
	activeProxyAccounts,
	children,
	mainAccountConnected,
	onLogout,
	requestCount,
	totalApiKeys,
	totalProxyAccounts,
	usageRangeLabel,
}: ManagementShellProps) {
	const [mobileOpen, setMobileOpen] = useState(false);

	useEffect(() => {
		if (!mobileOpen) return;
		const closeOnEscape = (event: KeyboardEvent) => {
			if (event.key === "Escape") setMobileOpen(false);
		};
		window.addEventListener("keydown", closeOnEscape);
		return () => window.removeEventListener("keydown", closeOnEscape);
	}, [mobileOpen]);

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
						href={window.location.pathname}
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
					<div className="sidebar-nav-group">
						<span className="sidebar-group-label">运行</span>
						<a
							aria-current="page"
							className="sidebar-nav-item active"
							href={window.location.pathname}
							onClick={() => setMobileOpen(false)}
						>
							<span className="sidebar-nav-icon"><ShellIcon name="home" /></span>
							<span className="sidebar-nav-label">主页</span>
						</a>
					</div>
				</nav>

				<div className="sidebar-footer">
					<div className="sidebar-service-state" title="管理服务已连接">
						<span className="sidebar-footer-copy">
							<strong>管理服务</strong>
							<small>已连接</small>
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
								<h1 id="dashboard-title">运行概览</h1>
								<p className="dashboard-description">
									集中查看账户配额、Token 消耗与调用身份。
								</p>
							</div>
						</div>
						<span
							className={`account-state-pill ${mainAccountConnected ? "connected" : "pending"}`}
						>
							{mainAccountConnected ? "主账户已连接" : "等待主账户登录"}
						</span>
					</section>

					<section className="dashboard-summary" aria-label="运行摘要">
						<SummaryCard
							detail={mainAccountConnected ? "OAuth 凭据可用" : "需要完成设备授权"}
							label="主账户"
							value={mainAccountConnected ? "已连接" : "待登录"}
						/>
						<SummaryCard
							detail={`共 ${totalApiKeys} 个本地密钥`}
							label="可用 API Keys"
							value={String(activeApiKeys)}
						/>
						<SummaryCard
							detail={`共 ${totalProxyAccounts} 个代理账户`}
							label="启用代理账户"
							value={String(activeProxyAccounts)}
						/>
						<SummaryCard
							detail={`${usageRangeLabel}累计请求`}
							label="请求数"
							value={requestCount === null ? "—" : requestCount.toLocaleString("zh-CN")}
						/>
					</section>

					<div className="dashboard-content">{children}</div>
				</main>
			</section>
		</div>
	);
}

function SummaryCard({
	detail,
	label,
	value,
}: {
	detail: string;
	label: string;
	value: string;
}) {
	return (
		<article className="summary-card">
			<div className="summary-card-topline">
				<span>{label}</span>
			</div>
			<strong>{value}</strong>
			<small>{detail}</small>
		</article>
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

function ShellIcon({ name }: { name: ShellIconName }) {
	return (
		<svg
			aria-hidden="true"
			fill="none"
			stroke="currentColor"
			strokeLinecap="round"
			strokeLinejoin="round"
			strokeWidth="1.8"
			viewBox="0 0 24 24"
		>
			{shellIconPaths(name)}
		</svg>
	);
}

function shellIconPaths(name: ShellIconName): ReactNode {
	switch (name) {
		case "home":
			return <><path d="m3 10 9-7 9 7" /><path d="M5 9v11h14V9" /><path d="M9 20v-6h6v6" /></>;
		case "logout":
			return <><path d="m16 17 5-5-5-5" /><path d="M21 12H9" /><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" /></>;
		case "menu":
			return <><path d="M4 7h16" /><path d="M4 12h16" /><path d="M4 17h16" /></>;
	}
}
