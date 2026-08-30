import {
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import type { UsageDashboard, UsageRange } from "./admin-api";
import { ProductMark } from "./ManagementShell";
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
	{ value: "cycle", label: "当前周期" },
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
	quota: PublicQuotaSnapshot | null;
}

interface PublicQuotaSnapshot {
	sampledAt: number;
	planType: string | null;
	windows: QuotaTimelineWindow[];
}

function AccountUsage() {
	const [range, setRange] = useState<UsageRange>("cycle");
	const [snapshot, setSnapshot] = useState<PublicAccountDashboard | null>(null);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const [now, setNow] = useState(() => Date.now());
	const requestRef = useRef(0);

	const load = useCallback(async (nextRange: UsageRange, signal?: AbortSignal) => {
		const requestId = ++requestRef.current;
		setLoading(true);
		try {
			const query = new URLSearchParams({ range: nextRange });
			const init: RequestInit = {
				cache: "no-store",
				headers: { accept: "application/json" },
			};
			if (signal) init.signal = signal;
			const response = await fetch(`${window.location.pathname}/data?${query}`, init);
			if (!response.ok) {
				throw new Error(response.status === 404 ? "not-found" : `HTTP ${response.status}`);
			}
			const next = parseDashboard(await response.json());
			if (requestId !== requestRef.current) return;
			setSnapshot(next);
			setError(null);
			setNow(Date.now());
		} catch (cause) {
			if (cause instanceof DOMException && cause.name === "AbortError") return;
			if (requestId !== requestRef.current) return;
			setError(
				cause instanceof Error && cause.message === "not-found"
					? "这个账户不存在、已停用，或访问凭证已经变更。"
					: "暂时无法读取账户用量，请稍后重试。",
			);
		} finally {
			if (requestId === requestRef.current) setLoading(false);
		}
	}, []);

	useEffect(() => {
		const controller = new AbortController();
		const initialLoad = window.setTimeout(() => {
			void load(range, controller.signal);
		}, 0);
		return () => {
			controller.abort();
			window.clearTimeout(initialLoad);
		};
	}, [load, range]);

	useEffect(() => {
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
	}, [load, range]);

	const usage = snapshot?.usage ?? null;
	const totals = usage?.totals ?? null;

	return (
		<main className="public-account-page">
			<div className="public-account-shell">
				<header className="public-account-header">
					<div className="public-account-brand">
						<ProductMark compact />
						<div><strong>Codex Router</strong><span>账户用量</span></div>
					</div>
					<div className="public-account-heading">
						<span className="public-account-kind">
							{snapshot?.account.identityType === "auth_proxy" ? "ACCOUNT ID" : "API KEY"}
						</span>
						<h1>用量信息</h1>
						<p>查看额度周期、Token 活动与模型消耗分布。</p>
					</div>
				</header>

				{error ? <div className="public-account-alert" role="alert">{error}</div> : null}
				{loading && !snapshot ? <AccountPageSkeleton /> : null}

				{snapshot && usage && totals ? (
					<div className={loading ? "public-account-content is-refreshing" : "public-account-content"}>
						<section className="public-account-range-bar" aria-label="统计时间范围">
							<div className="public-account-range-copy">
								<strong>统计范围</strong>
								<span>{formatDateRange(usage.startAt, usage.endAt)}</span>
							</div>
							<div className="public-account-range-actions">
								<div className="public-account-range-options" role="group" aria-label="选择统计时间范围">
									{RANGE_OPTIONS.map((option) => (
										<button
											aria-pressed={range === option.value}
											className={range === option.value ? "active" : ""}
											disabled={loading}
											key={option.value}
											onClick={() => setRange(option.value)}
											type="button"
										>
											{option.label}
										</button>
									))}
								</div>
								<button
									aria-label="刷新账户用量"
									className="public-account-refresh"
									disabled={loading}
									onClick={() => void load(range)}
									title="刷新账户用量"
									type="button"
								>
									<RefreshIcon spinning={loading} />
								</button>
							</div>
						</section>

						<div className="public-account-overview">
							<aside className="public-account-metrics" aria-label="账户用量指标">
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
							</aside>

							<section className="public-account-visuals" aria-label="账户用量图表">
								<div className="public-account-activity-stack activity-card-grid-stacked">
									<TokenActivityCard now={now} usage={usage} />
									<CostActivityCard now={now} usage={usage} />
								</div>
								<ModelTokenDonut usage={usage} />
							</section>
						</div>

						<section className="public-account-quota" aria-label="账户额度时间条">
							{snapshot.quota && snapshot.quota.windows.length > 0 ? (
								<QuotaTimeline
									className="public-account-quota-timeline"
									now={now}
									planType={snapshot.quota.planType}
									sampledAt={snapshot.quota.sampledAt}
									windows={snapshot.quota.windows}
								/>
							) : (
								<div className="public-account-quota-empty">额度时间条尚未完成首次同步。</div>
							)}
						</section>
					</div>
				) : null}
			</div>
		</main>
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

function AccountPageSkeleton() {
	return (
		<div className="public-account-skeleton" role="status" aria-label="正在读取账户用量">
			<div className="skeleton-range" />
			<div className="skeleton-content"><span /><span /><span /><span /></div>
			<div className="skeleton-quota" />
		</div>
	);
}

function RefreshIcon({ spinning }: { spinning: boolean }) {
	return (
		<svg className={spinning ? "is-spinning" : ""} aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">
			<path d="M20 6v5h-5" /><path d="M4 18v-5h5" />
			<path d="M18.5 9A7 7 0 0 0 6.2 6.2L4 9" /><path d="M5.5 15a7 7 0 0 0 12.3 2.8L20 15" />
		</svg>
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
