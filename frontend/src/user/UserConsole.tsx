// Signed-in API key / downstream account console: topbar with 用量 / 调用日志
// tabs (Figma "User v2 / 用量" and "User v2 / 调用日志").

import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { ChartNoAxesColumnIcon, KeyRoundIcon, Layers3Icon, LogOutIcon, ScrollTextIcon, UserRoundIcon } from "lucide-react";
import { toast } from "sonner";

import { AdminSessionExpiredError } from "@/admin-api";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ProductMark } from "@/ManagementShell";
import { signOut } from "@/session-api";
import { UserApiClient, type UserProfile } from "@/user-api";

import UserLogs from "./UserLogs";
import UserUsage from "./UserUsage";

type UserTab = "usage" | "logs";

const TAB_COPY: Record<UserTab, { title: string; description: (name: string) => string }> = {
	usage: { title: "用量信息", description: (name) => `${name} 的用量、延迟与所在账户池额度。` },
	logs: { title: "调用日志", description: (name) => `逐条查看 ${name} 的请求。` },
};

function tabFromSearch(): UserTab {
	return new URLSearchParams(window.location.search).get("tab") === "logs" ? "logs" : "usage";
}

export default function UserConsole({ onSignedOut }: { onSignedOut: (error?: string | null) => void }) {
	const api = useMemo(() => new UserApiClient(), []);
	const [tab, setTab] = useState<UserTab>(tabFromSearch);
	const [logParams, setLogParams] = useState(() => new URLSearchParams(window.location.search));
	const [profile, setProfile] = useState<UserProfile | null>(null);
	const [now, setNow] = useState(() => Date.now());

	const expired = useCallback(() => onSignedOut(null), [onSignedOut]);

	useEffect(() => {
		let active = true;
		const load = () =>
			api
				.getProfile()
				.then((next) => {
					if (active) setProfile(next);
				})
				.catch((error: unknown) => {
					if (!active) return;
					if (error instanceof AdminSessionExpiredError) expired();
					else toast.error(error instanceof Error ? error.message : "读取账户信息失败。");
				});
		void load();
		const refresh = window.setInterval(() => {
			if (!document.hidden) void load();
		}, 60_000);
		const tick = window.setInterval(() => {
			if (!document.hidden) setNow(Date.now());
		}, 30_000);
		const onPopState = () => {
			setTab(tabFromSearch());
			setLogParams(new URLSearchParams(window.location.search));
		};
		window.addEventListener("popstate", onPopState);
		return () => {
			active = false;
			window.clearInterval(refresh);
			window.clearInterval(tick);
			window.removeEventListener("popstate", onPopState);
		};
	}, [api, expired]);

	useEffect(() => {
		document.title = `${TAB_COPY[tab].title} · Codex Router`;
	}, [tab]);

	function changeTab(next: string, params: Record<string, string> = {}) {
		const value: UserTab = next === "logs" ? "logs" : "usage";
		const url = new URL(window.location.href);
		url.search = "";
		if (value === "logs") url.searchParams.set("tab", "logs");
		for (const [key, item] of Object.entries(params)) url.searchParams.set(key, item);
		window.history.pushState(null, "", url);
		setLogParams(new URLSearchParams(url.search));
		setTab(value);
	}

	async function handleSignOut() {
		try {
			await signOut();
		} catch (error) {
			if (!(error instanceof AdminSessionExpiredError)) {
				toast.error(error instanceof Error ? error.message : "退出登录失败。");
				return;
			}
		}
		onSignedOut(null);
	}

	const identity = profile?.identity;
	const kindLabel = identity?.identityType === "auth_proxy" ? "下游账户" : "API Key";
	const name = identity?.name || kindLabel;
	const routeLabel = routeSummary(profile);

	return (
		<Tabs className="h-svh gap-0" onValueChange={(next) => changeTab(next)} value={tab}>
			<header className="shrink-0 border-b bg-card">
				<div className="mx-auto flex h-16 max-w-[1128px] items-center justify-between gap-4 px-4 lg:px-0">
					<div className="flex h-full min-w-0 items-center gap-8">
						<div className="flex items-center gap-2.5">
							<ProductMark compact />
							<div className="flex flex-col max-sm:hidden">
								<strong className="text-sm leading-5 font-bold tracking-[-0.02em]">Codex Router</strong>
								<span className="text-[11px] leading-4 text-faint">账户用量</span>
							</div>
						</div>
						<TabsList className="h-full gap-6 p-0" variant="line">
							<TabButton icon={<ChartNoAxesColumnIcon />} label="用量" value="usage" />
							<TabButton icon={<ScrollTextIcon />} label="调用日志" value="logs" />
						</TabsList>
					</div>
					<div className="flex items-center gap-3">
						{identity ? (
							<span className="flex h-8 items-center gap-2 rounded-full border bg-surface-subtle pr-3 pl-2.5 text-[13px] max-md:hidden">
								{identity.identityType === "auth_proxy" ? <UserRoundIcon className="size-3.5" /> : <KeyRoundIcon className="size-3.5" />}
								<span className="font-semibold">{identity.name}</span>
								<span className="text-[12.5px] text-muted-foreground">· {kindLabel}</span>
							</span>
						) : null}
						<Button onClick={() => void handleSignOut()} size="sm" type="button" variant="outline">
							<LogOutIcon data-icon="inline-start" />
							退出登录
						</Button>
					</div>
				</div>
			</header>

			<ScrollArea className="min-h-0 flex-1">
				<main className="mx-auto flex max-w-[1128px] flex-col px-4 pt-10 pb-16 lg:px-0">
					<section className="flex flex-wrap items-end justify-between gap-4 pb-6" aria-labelledby="user-title">
						<div className="flex min-w-0 flex-col">
							<p className="pb-2 text-[9.76px] leading-4 font-extrabold tracking-[0.12em] text-faint uppercase">
								{kindLabel} · 仅显示当前凭据的数据
							</p>
							<h1 className="text-[38.4px] leading-[52px] font-bold tracking-[-0.045em]" id="user-title">
								{TAB_COPY[tab].title}
							</h1>
							<p className="pt-2 text-[14px] leading-5 text-muted-foreground">
								{profile ? TAB_COPY[tab].description(name) : <Skeleton className="h-5 w-64" />}
							</p>
						</div>
						{identity ? (
							<div className="flex flex-wrap items-center gap-2">
								<span className="flex items-center rounded-md border bg-card px-2.5 py-1.5 font-mono text-xs text-muted-foreground">
									{identity.identityType === "api_key" ? "sk-" : ""}••••••••{identity.credentialHint}
								</span>
								{routeLabel ? (
									<span className="flex items-center gap-1.5 rounded-md border bg-card px-2.5 py-1.5 text-xs">
										<Layers3Icon className="size-3.25 text-muted-foreground" />
										<span className="text-muted-foreground">路由</span>
										<span className="font-semibold">{routeLabel}</span>
									</span>
								) : null}
							</div>
						) : null}
					</section>
					<TabsContent value="usage">
						<UserUsage
							api={api}
							now={now}
							onOpenLogs={(params) => changeTab("logs", params)}
							onSessionExpired={expired}
							profile={profile}
						/>
					</TabsContent>
					<TabsContent value="logs">
						<UserLogs api={api} initialParams={logParams} now={now} onSessionExpired={expired} />
					</TabsContent>
				</main>
			</ScrollArea>
		</Tabs>
	);
}

function TabButton({ value, label, icon }: { value: UserTab; label: string; icon: ReactNode }) {
	return (
		<TabsTrigger
			className="h-full flex-none gap-2 rounded-none border-0 px-1 text-sm font-medium text-muted-foreground after:bg-foreground data-[state=active]:font-bold data-[state=active]:text-foreground [&_svg]:size-4"
			value={value}
		>
			{icon}
			{label}
		</TabsTrigger>
	);
}

function routeSummary(profile: UserProfile | null): string | null {
	if (!profile?.route) return profile ? "未分配" : null;
	if (profile.group) {
		const count = profile.quota?.accounts.length;
		return count ? `${profile.group.name} · ${count} 个账户` : profile.group.name;
	}
	return profile.quota?.accounts[0]?.accountName ?? "单个账户";
}
