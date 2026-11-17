import {
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import {
	Layers3Icon,
	RefreshCwIcon,
	TriangleAlertIcon,
} from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyHeader } from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { UsageDashboard, UsageRange } from "./admin-api";
import { ProductMark } from "./ManagementShell";
import { CONSOLE_API_BASE, signOut } from "./session-api";
import QuotaTimeline, { type QuotaTimelineWindow } from "./QuotaTimeline";
import {
	CostActivityCard,
	ModelTokenDonut,
	TokenActivityCard,
} from "./UsageVisuals";
import { formatCost } from "./usage-format";
import "./App.css";
import "./AccountUsage.css";

const REFRESH_INTERVAL_MS = 5 * 60 * 1_000;
const CLOCK_INTERVAL_MS = 60 * 1_000;
const INTEGER_FORMAT = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 });
const COMPACT_FORMAT = new Intl.NumberFormat("zh-CN", {
	notation: "compact",
	maximumFractionDigits: 2,
});
const RANGE_OPTIONS: ReadonlyArray<{ value: UsageRange; label: string }> = [
	{ value: "24h", label: "24 小时" },
	{ value: "7d", label: "7 天" },
	{ value: "30d", label: "30 天" },
	{ value: "all", label: "全部" },
];

interface PublicAccountDashboard {
	account: {
		identityType: "api_key" | "auth_proxy";
	};
	usage: UsageDashboard;
	quota: PublicQuotaCollection | null;
}

interface PublicQuotaSnapshot {
	accountId: string;
	accountName: string;
	sampledAt: number | null;
	planType: string | null;
	windows: QuotaTimelineWindow[];
}

interface PublicQuotaCollection {
	group: { id: string; name: string } | null;
	accounts: PublicQuotaSnapshot[];
}

interface AccountUsageProps {
	/** Returns to the sign-in screen, optionally explaining why. */
	onSignedOut: (error?: string | null) => void;
}

function AccountUsage({ onSignedOut }: AccountUsageProps) {
	const [range, setRange] = useState<UsageRange>("24h");
	const [snapshot, setSnapshot] = useState<PublicAccountDashboard | null>(null);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [now, setNow] = useState(() => Date.now());
	const requestRef = useRef(0);

	const load = useCallback(async (nextRange: UsageRange) => {
		const requestId = ++requestRef.current;
		setLoading(true);
		try {
			const response = await fetch(
				`${CONSOLE_API_BASE}/account?${new URLSearchParams({ range: nextRange })}`,
				{ credentials: "same-origin", headers: { Accept: "application/json" } },
			);
			if (response.status === 401 || response.status === 403) {
				if (requestId === requestRef.current) onSignedOut("登录已失效，请重新登录。");
				return;
			}
			if (!response.ok) throw new Error(`HTTP ${response.status}`);
			const next = parseDashboard(await response.json());
			if (requestId !== requestRef.current) return;
			setSnapshot(next);
			setError(null);
			setNow(Date.now());
		} catch {
			if (requestId !== requestRef.current) return;
			setError("暂时无法读取账户用量，请稍后重试。");
		} finally {
			if (requestId === requestRef.current) setLoading(false);
		}
	}, [onSignedOut]);

	useEffect(() => {
		// Defer the first load so the effect itself never sets state synchronously.
		const timer = window.setTimeout(() => void load("24h"), 0);
		return () => window.clearTimeout(timer);
	}, [load]);

	const accountSelected = snapshot !== null;

	useEffect(() => {
		if (!accountSelected) return;
		const refresh = window.setInterval(() => {
			if (!document.hidden) void load(range);
		}, REFRESH_INTERVAL_MS);
		const clock = window.setInterval(() => setNow(Date.now()), CLOCK_INTERVAL_MS);
		const onVisibilityChange = () => {
			if (!document.hidden) {
				setNow(Date.now());
				void load(range);
			}
		};
		document.addEventListener("visibilitychange", onVisibilityChange);
		return () => {
			window.clearInterval(refresh);
			window.clearInterval(clock);
			document.removeEventListener("visibilitychange", onVisibilityChange);
		};
	}, [accountSelected, load, range]);

	function changeRange(nextRange: UsageRange): void {
		if (nextRange === range) return;
		setRange(nextRange);
		void load(nextRange);
	}

	async function handleSignOut(): Promise<void> {
		requestRef.current += 1;
		try {
			await signOut();
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : "退出登录失败，请稍后重试。");
			return;
		}
		onSignedOut(null);
	}

	if (!snapshot) {
		return error ? (
			<main className="loading-screen">
				<ProductMark />
				<Alert className="max-w-md" variant="destructive"><TriangleAlertIcon /><AlertDescription>{error}</AlertDescription></Alert>
				<div className="flex gap-2">
					<Button onClick={() => void load(range)} type="button" variant="outline">重试</Button>
					<Button onClick={() => void handleSignOut()} type="button" variant="ghost">退出登录</Button>
				</div>
			</main>
		) : (
			<div className="loading-screen"><ProductMark /><Spinner /><strong>加载中…</strong></div>
		);
	}

	const usage = snapshot.usage;
	const totals = usage.totals;

	return (
		<ScrollArea className="h-svh">
			<main className="public-account-page">
			<div className="public-account-shell">
				<header className="public-account-header">
					<div className="public-account-brand">
						<ProductMark compact />
						<div><strong>Codex Router</strong><span>账户用量</span></div>
						<Button
							disabled={loading}
							onClick={() => void handleSignOut()}
							size="sm"
							type="button"
							variant="outline"
						>
							退出登录
						</Button>
					</div>
					<div className="public-account-heading">
						<Badge className="mb-2 text-[.56rem] font-[760] tracking-[.08em]" variant="outline">
							{snapshot.account.identityType === "auth_proxy" ? "ACCOUNT ID" : "API KEY"}
						</Badge>
						<h1>用量信息</h1>
						<p>{snapshot.quota ? "查看 Token 活动、模型消耗与路由账户额度。" : "查看 Token 活动与模型消耗分布。"}</p>
					</div>
				</header>

				{error ? <Alert className="mb-3 text-[.76rem]" variant="destructive"><TriangleAlertIcon /><AlertDescription>{error}</AlertDescription></Alert> : null}

				<div className={loading ? "public-account-content is-refreshing" : "public-account-content"}>
					<Card className="min-w-0 flex-row items-center justify-between gap-3 p-3 max-[58rem]:flex-col max-[58rem]:items-start" aria-label="统计时间范围">
						<div className="public-account-range-copy">
							<strong>统计范围</strong>
							<span>{formatDateRange(usage.startAt, usage.endAt)}</span>
						</div>
						<div className="public-account-range-actions">
							<ScrollArea className="w-max min-w-0 max-w-full max-[58rem]:w-full max-[42rem]:flex-1" aria-label="选择统计时间范围" scrollbars="horizontal">
								<ToggleGroup
									className="min-w-max"
									disabled={loading}
									onValueChange={(value) => { if (value) changeRange(value as UsageRange); }}
									spacing={0}
									type="single"
									value={range}
									variant="outline"
								>
									{RANGE_OPTIONS.map((option) => (
										<ToggleGroupItem
											key={option.value}
											value={option.value}
										>
											{option.label}
										</ToggleGroupItem>
									))}
								</ToggleGroup>
							</ScrollArea>
							<Button
								aria-label="刷新账户用量"
								disabled={loading}
								onClick={() => void load(range)}
								size="icon"
								title="刷新账户用量"
								type="button"
								variant="outline"
							>
								<RefreshCwIcon className={loading ? "animate-spin" : undefined} />
							</Button>
						</div>
					</Card>

					<div className="public-account-overview">
						<Card className="public-account-metrics grid min-w-0 grid-rows-[auto_repeat(3,minmax(0,1fr))] gap-0 p-3 max-[58rem]:grid-cols-3 max-[58rem]:grid-rows-[auto_minmax(0,1fr)] max-[42rem]:grid-cols-1" aria-label="账户用量指标">
							<header><span>账户指标</span><small>{rangeLabel(usage.range)}</small></header>
							<AccountMetric label="请求次数" value={formatCount(totals.requests)} />
							<AccountMetric
								label="Token 总量"
								value={formatTokens(totals.totalTokens)}
							/>
							<AccountMetric
								label="成本"
								value={formatCost(totals.costUsd)}
							/>
						</Card>

						<section className="public-account-visuals" aria-label="账户用量图表">
							<div className="public-account-activity-stack activity-card-grid-stacked">
								<TokenActivityCard now={now} usage={usage} />
								<CostActivityCard now={now} usage={usage} />
							</div>
							<ModelTokenDonut usage={usage} />
						</section>
					</div>

					{snapshot.quota ? (
						<PublicQuotaSection now={now} quota={snapshot.quota} />
					) : null}
				</div>
			</div>
			</main>
		</ScrollArea>
	);
}

function PublicQuotaSection({ now, quota }: { now: number; quota: PublicQuotaCollection }) {
	const title = quota.group?.name ?? "路由账户额度";
	return (
		<section className="public-quota-section" aria-labelledby="public-quota-title">
			<header className="public-quota-heading">
				<div className="public-quota-heading-icon"><Layers3Icon /></div>
				<h2 id="public-quota-title">{title}</h2>
				<span className="public-quota-count">{quota.accounts.length} 个账户</span>
			</header>

			{quota.accounts.length ? (
				<div className="public-quota-list">
					{quota.accounts.map((account) => (
						account.sampledAt !== null && account.windows.length ? (
							<QuotaTimeline
								accountName={account.accountName}
								className="min-w-0"
								key={account.accountId}
								now={now}
								planType={account.planType}
								sampledAt={account.sampledAt}
								showLegend={false}
								windows={account.windows}
							/>
						) : (
							<Card className="public-quota-empty" key={account.accountId}>
								<div className="public-quota-account-label">
									<span>{account.accountName.slice(0, 1).toUpperCase() || "C"}</span>
									<strong>{account.accountName}</strong>
								</div>
								<Empty className="min-h-24 rounded-none border-x-0 border-b-0 text-[.72rem]"><EmptyHeader><EmptyDescription>额度时间轴尚未完成首次同步。</EmptyDescription></EmptyHeader></Empty>
							</Card>
						)
					))}
				</div>
			) : (
				<Empty className="min-h-30 border text-[.72rem]"><EmptyHeader><EmptyDescription>当前凭据尚未分配可展示的路由账户。</EmptyDescription></EmptyHeader></Empty>
			)}
		</section>
	);
}

function AccountMetric({ label, value }: { label: string; value: string }) {
	return (
		<div className="public-account-metric">
			<span>{label}</span>
			<strong>{value}</strong>
		</div>
	);
}

function parseDashboard(value: unknown): PublicAccountDashboard {
	if (!isRecord(value) || !isRecord(value.account) || !isRecord(value.usage)) {
		throw new Error("invalid-response");
	}
	if (
		!(["api_key", "auth_proxy"] as const).includes(value.account.identityType as "api_key" | "auth_proxy") ||
		!isRecord(value.usage.totals) ||
		!Array.isArray(value.usage.series) ||
		!Array.isArray(value.usage.models)
	) {
		throw new Error("invalid-response");
	}
	return value as unknown as PublicAccountDashboard;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function formatTokens(value: number): string {
	const normalized = Math.max(0, Number.isFinite(value) ? value : 0);
	return normalized < 10_000 ? INTEGER_FORMAT.format(normalized) : COMPACT_FORMAT.format(normalized);
}

function formatCount(value: number): string {
	return INTEGER_FORMAT.format(Math.max(0, Number.isFinite(value) ? value : 0));
}

function formatDateRange(startAt: number, endAt: number): string {
	const formatter = new Intl.DateTimeFormat("zh-CN", {
		year: "numeric",
		month: "2-digit",
		day: "2-digit",
	});
	return `${formatter.format(new Date(startAt))} — ${formatter.format(new Date(endAt))}`;
}

function rangeLabel(range: string): string {
	return RANGE_OPTIONS.find((option) => option.value === range)?.label ?? "所选范围";
}

export default AccountUsage;
