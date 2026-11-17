import {
	ArrowRightIcon,
	CheckCircle2Icon,
	CircleAlertIcon,
	CircleDollarSignIcon,
	CircleXIcon,
	Layers3Icon,
	ServerIcon,
	TimerIcon,
	TriangleAlertIcon,
	UsersIcon,
} from "lucide-react";
import type { ReactNode } from "react";

import { useAdmin } from "@/admin/context";
import type { ManagementPage } from "@/admin/pages";
import { SectionCard } from "@/components/app/section-card";
import { Tag } from "@/components/app/badges";
import { planLabel } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { formatDateTime, formatClock, formatPercentValue, formatSpan, formatTokens, HOUR_MS } from "@/lib/format";
import { cn } from "@/lib/utils";

import { shortAccountName, windowLabel, type OverviewData } from "./data";

type Tone = "danger" | "warning" | "neutral";

interface AttentionItem {
	key: string;
	tone: Tone;
	icon: ReactNode;
	title: ReactNode;
	detail: ReactNode;
	action: string;
	page: ManagementPage;
	params?: Record<string, string>;
	/** Lower sorts first. */
	rank: number;
}

const TONE_ORDER: Record<Tone, number> = { danger: 0, warning: 1, neutral: 2 };

/** Figma "card · 需要处理": quota exhaustion, 429s, unpriced models, routing gaps, disabled accounts. */
export function AttentionCard({ data }: { data: OverviewData }) {
	const { quota, state, now, navigate } = useAdmin();
	const items: AttentionItem[] = [];

	const groupNames = (accountId: string) =>
		state.accountGroups.filter((group) => group.accountIds.includes(accountId)).map((group) => group.name);

	for (const account of quota?.accounts ?? []) {
		if (!account.enabled) continue;
		const name = shortAccountName(account.accountName);
		for (const window of account.windows) {
			if (window.category !== "codex" || !window.exhaustsBeforeReset || window.exhaustAt === null) continue;
			const soon = window.exhaustAt - now < 2 * HOUR_MS;
			const pools = groupNames(account.accountId);
			const others = state.accountGroups
				.filter((group) => pools.includes(group.name))
				.flatMap((group) => group.accountIds)
				.filter((id) => id !== account.accountId)
				.map((id) => state.codexAccounts.find((entry) => entry.id === id))
				.filter((entry) => entry?.enabled)
				.map((entry) => shortAccountName(entry?.name ?? ""));
			items.push({
				key: `quota:${account.accountId}:${window.id}`,
				tone: soon ? "danger" : "warning",
				icon: soon ? <TimerIcon /> : <TriangleAlertIcon />,
				title: soon
					? `${name} ${windowLabel(window)}额度约 ${formatSpan(Math.max(0, window.exhaustAt - now))}后耗尽`
					: `${name} ${windowLabel(window)}额度将早于重置耗尽`,
				detail: soon
					? `已用 ${formatPercentValue(window.usedPercent)}，${window.resetAt ? `${formatClock(window.resetAt, false)} 重置` : "重置时间未知"}${others.length && pools.length ? `；${pools[0]}将转向 ${[...new Set(others)].join("、")}` : ""}`
					: `预计 ${formatDateTime(window.exhaustAt)} 耗尽，${formatDateTime(window.resetAt)} 才重置`,
				action: "查看额度",
				page: "codex",
				rank: window.exhaustAt,
			});
		}
	}

	for (const upstream of data.today?.upstreamAccounts ?? []) {
		if (upstream.rateLimited === 0) continue;
		const hour = data.lastHour?.upstreamAccounts.find((entry) => entry.codexAccountId === upstream.codexAccountId)?.rateLimited ?? 0;
		const week = data.week?.upstreamAccounts.find((entry) => entry.codexAccountId === upstream.codexAccountId)?.rateLimited;
		items.push({
			key: `429:${upstream.codexAccountId}`,
			tone: hour > 0 ? "danger" : "warning",
			icon: <CircleAlertIcon />,
			title: `${shortAccountName(upstream.codexAccountName)} 出现 429`,
			detail: `今日 ${upstream.rateLimited} 次 · 近 1 小时 ${hour} 次${week === undefined ? "" : ` · 7 天 ${week} 次`}`,
			action: "查看日志",
			page: "logs",
			params: { upstreamType: "codex_account", upstreamId: upstream.codexAccountId, status: "failed" },
			rank: Number.MAX_SAFE_INTEGER - hour,
		});
	}

	for (const model of data.week?.unpricedModels ?? []) {
		items.push({
			key: `price:${model.model}`,
			tone: "warning",
			icon: <CircleDollarSignIcon />,
			title: `${model.model} 未配置价格`,
			detail: `7 天 ${formatTokens(model.totalTokens)} Token 未计入成本`,
			action: "配置价格",
			page: "pricing",
			rank: Number.MAX_SAFE_INTEGER,
		});
	}

	for (const user of state.users.filter((entry) => entry.enabled && !state.routes.some((route) => route.userId === entry.id))) {
		const hasKey = user.credentials.some((credential) => credential.enabled && credential.type === "api_key");
		items.push({
			key: `route:user:${user.id}`,
			tone: "neutral",
			icon: <UsersIcon />,
			title: `${user.name}未分配路由`,
			detail: hasKey ? "API Key 请求将不可用" : "请求将透传客户端自带的上游凭据",
			action: "分配路由",
			page: "users",
			rank: Number.MAX_SAFE_INTEGER,
		});
	}

	for (const account of state.codexAccounts) {
		const expired = account.oauth && account.oauth.expiresAt > 0 && account.oauth.expiresAt <= now;
		if (account.enabled && !expired) continue;
		const plan = planLabel(account.subscription?.planType);
		items.push({
			key: `account:${account.id}`,
			tone: expired && account.enabled ? "danger" : "neutral",
			icon: <CircleXIcon />,
			title: `${account.name} ${account.enabled ? "凭据已过期" : "已停用"}`,
			detail: account.enabled ? "需要重新登录才能参与路由" : `${plan ? `${plan} 账户，` : ""}不参与路由`,
			action: "Codex 账户",
			page: "codex",
			rank: Number.MAX_SAFE_INTEGER,
		});
	}

	items.sort((left, right) => TONE_ORDER[left.tone] - TONE_ORDER[right.tone] || left.rank - right.rank);

	return (
		<SectionCard
			actions={items.length ? <Tag tone={items.some((item) => item.tone === "danger") ? "danger" : "warning"}>{items.length} 项</Tag> : null}
			title="需要处理"
		>
			{!data.today && !quota ? (
				<div className="flex flex-col gap-3">
					{[0, 1, 2, 3].map((index) => (
						<Skeleton className="h-10" key={index} />
					))}
				</div>
			) : items.length === 0 ? (
				<Empty className="py-6">
					<EmptyHeader>
						<EmptyMedia variant="icon">
							<CheckCircle2Icon />
						</EmptyMedia>
						<EmptyTitle>一切正常</EmptyTitle>
						<EmptyDescription>没有需要处理的额度、限流或配置问题。</EmptyDescription>
					</EmptyHeader>
				</Empty>
			) : (
				<ul className="flex flex-col divide-y divide-border/70">
					{items.map((item) => (
						<li className="flex items-start gap-2 py-2" key={item.key}>
							<span
								className={cn(
									"flex shrink-0 items-center justify-center rounded-md p-1 [&_svg]:size-3.5",
									item.tone === "danger" && "bg-danger-soft text-danger",
									item.tone === "warning" && "bg-warning-soft text-warning",
									item.tone === "neutral" && "bg-surface-subtle text-muted-foreground",
								)}
							>
								{item.icon}
							</span>
							<div className="flex min-w-0 flex-1 flex-col gap-0.5">
								<div className="flex items-center gap-2">
									<span className="min-w-0 flex-1 text-[12.5px] leading-5 font-semibold">{item.title}</span>
									<Button className="h-auto shrink-0 gap-0.5 px-0 text-[11.5px] text-foreground" onClick={() => navigate(item.page, item.params)} size="xs" type="button" variant="link">
										{item.action}
										<ArrowRightIcon className="size-3" />
									</Button>
								</div>
								<span className="text-[11.5px] leading-4 text-muted-foreground">{item.detail}</span>
							</div>
						</li>
					))}
				</ul>
			)}
		</SectionCard>
	);
}

/** Figma "card · 资源概况": counts per resource with their state split. */
export function ResourcesCard() {
	const { state, pricing, navigate } = useAdmin();
	const routedTo = (id: string) => state.routes.some((route) => route.userId === id);
	const usedGroups = new Set(state.routes.filter((route) => route.targetType === "group").map((route) => route.targetId));
	const priced = new Set(pricing.prices.map((price) => price.model.toLowerCase()));
	const usedModels = pricing.usedModels;
	const unpriced = usedModels.filter((model) => !priced.has(model.toLowerCase())).length;

	const codexEnabled = state.codexAccounts.filter((account) => account.enabled).length;
	const usersEnabled = state.users.filter((user) => user.enabled).length;
	const usersUnrouted = state.users.filter((user) => user.enabled && !routedTo(user.id)).length;

	const rows: { key: string; icon: ReactNode; label: string; count: number; parts: { text: string; tone: "success" | "warning" | "muted" }[]; page: ManagementPage }[] = [
		{
			key: "codex",
			icon: <ServerIcon />,
			label: "Codex 账户",
			count: state.codexAccounts.length,
			parts: [
				{ text: `${codexEnabled} 启用`, tone: "success" },
				{ text: `${state.codexAccounts.length - codexEnabled} 停用`, tone: "muted" },
			],
			page: "codex",
		},
		{
			key: "groups",
			icon: <Layers3Icon />,
			label: "账户组",
			count: state.accountGroups.length,
			parts: [
				{ text: `${usedGroups.size} 使用中`, tone: "success" },
				{ text: `${state.accountGroups.filter((group) => !usedGroups.has(group.id)).length} 未分配用户`, tone: "warning" },
			],
			page: "groups",
		},
		{
			key: "users",
			icon: <UsersIcon />,
			label: "用户",
			count: state.users.length,
			parts: [
				{ text: `${usersEnabled} 启用`, tone: "success" },
				{ text: `${state.users.length - usersEnabled} 停用`, tone: "muted" },
				{ text: `${usersUnrouted} 未分配`, tone: "warning" },
			],
			page: "users",
		},
		{
			key: "pricing",
			icon: <CircleDollarSignIcon />,
			label: "模型价格",
			count: usedModels.length,
			parts: [
				{ text: `${usedModels.length - unpriced} 已定价`, tone: "success" },
				{ text: `${unpriced} 未定价`, tone: "warning" },
			],
			page: "pricing",
		},
	];

	return (
		<SectionCard title="资源概况">
			<ul className="flex flex-col divide-y divide-border/70">
				{rows.map((row) => (
					<li key={row.key}>
						<button className="flex w-full items-center gap-4 rounded-md py-2 text-left transition-colors hover:bg-surface-subtle/60" onClick={() => navigate(row.page)} type="button">
							<span className="flex rounded-md bg-surface-subtle p-2 text-muted-foreground [&_svg]:size-4">{row.icon}</span>
							<span className="flex min-w-0 flex-1 flex-col gap-1">
								<span className="flex items-baseline gap-2">
									<span className="text-[12.5px] leading-5 font-medium">{row.label}</span>
									<span className="text-[15px] leading-5 font-bold tabular-nums">{row.count}</span>
								</span>
								<span className="flex flex-wrap gap-2 text-[11px] leading-4 tabular-nums">
									{row.parts
										.filter((part, index) => index === 0 || !part.text.startsWith("0 "))
										.map((part) => (
											<span className={part.tone === "success" ? "text-success" : part.tone === "warning" ? "text-warning" : "text-faint"} key={part.text}>
												{part.text}
											</span>
										))}
								</span>
							</span>
							<ArrowRightIcon className="size-3.5 text-muted-foreground" />
						</button>
					</li>
				))}
			</ul>
		</SectionCard>
	);
}
