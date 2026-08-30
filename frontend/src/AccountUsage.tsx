import {
	useCallback,
	useEffect,
	useRef,
	useState,
	type FormEvent,
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
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [now, setNow] = useState(() => Date.now());
	const requestRef = useRef(0);
	const credentialRef = useRef<string | null>(null);

	const load = useCallback(async (
		nextRange: UsageRange,
		nextCredential: string | null = credentialRef.current,
	) => {
		if (!nextCredential) return;
		const requestId = ++requestRef.current;
		setLoading(true);
		try {
			const response = await fetch("/account/data", {
				headers: { accept: "application/json" },
				method: "POST",
				body: new URLSearchParams({
					credential: nextCredential,
					range: nextRange,
				}),
			});
			if (!response.ok) {
				throw new Error(response.status === 404 ? "not-found" : `HTTP ${response.status}`);
			}
			const next = parseDashboard(await response.json());
			if (requestId !== requestRef.current) return;
			credentialRef.current = nextCredential;
			setSnapshot(next);
			setError(null);
			setNow(Date.now());
		} catch (cause) {
			if (requestId !== requestRef.current) return;
			setError(
				cause instanceof Error && cause.message === "not-found"
					? "输入的 API Key 或 account id 不存在，或已停用。"
					: "暂时无法读取账户用量，请稍后重试。",
			);
		} finally {
			if (requestId === requestRef.current) setLoading(false);
		}
	}, []);

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

	function lookup(credential: string): void {
		setError(null);
		void load("cycle", credential);
	}

	function changeRange(nextRange: UsageRange): void {
		if (nextRange === range) return;
		setRange(nextRange);
		void load(nextRange);
	}

	function clearAccount(): void {
		credentialRef.current = null;
		setRange("cycle");
		setSnapshot(null);
		setError(null);
	}

	if (!snapshot) {
		return <AccountLookupView error={error} loading={loading} onSubmit={lookup} />;
	}

	const usage = snapshot.usage;
	const totals = usage.totals;

	return (
		<main className="public-account-page">
			<div className="public-account-shell">
				<header className="public-account-header">
					<div className="public-account-brand">
						<ProductMark compact />
						<div><strong>Codex Router</strong><span>账户用量</span></div>
						<button
							className="button button-secondary public-account-change"
							disabled={loading}
							onClick={clearAccount}
							type="button"
						>
							更换凭据
						</button>
					</div>
					<div className="public-account-heading">
						<span className="public-account-kind">
							{snapshot.account.identityType === "auth_proxy" ? "ACCOUNT ID" : "API KEY"}
						</span>
						<h1>用量信息</h1>
						<p>查看额度周期、Token 活动与模型消耗分布。</p>
					</div>
				</header>

				{error ? <div className="public-account-alert" role="alert">{error}</div> : null}

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
										onClick={() => changeRange(option.value)}
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
			</div>
		</main>
	);
}

interface AccountLookupViewProps {
	error: string | null;
	loading: boolean;
	onSubmit: (credential: string) => void;
}

function AccountLookupView({ error, loading, onSubmit }: AccountLookupViewProps) {
	const [credential, setCredential] = useState("");
	const [visible, setVisible] = useState(false);

	function submit(event: FormEvent<HTMLFormElement>): void {
		event.preventDefault();
		if (!credential || loading) return;
		onSubmit(credential);
	}

	return (
		<div className="auth-shell">
			<aside className="auth-aside" aria-label="Codex Router">
				<div className="auth-aside-title" aria-hidden="true">
					<span>Codex</span>
					<span>Router</span>
				</div>
			</aside>

			<div className="auth-main">
				<main className="auth-card">
					<div className="auth-mobile-brand"><ProductMark compact /><strong>Codex Router</strong></div>
					<p className="auth-eyebrow">账户用量</p>
					<h1>查看用量信息</h1>
					<p className="auth-description">输入 API Key 或 account id，查看对应账户的额度与 Token 消耗。</p>

					{error ? (
						<div className="inline-alert error-alert" role="alert">
							<LookupIcon name="alert" />
							<span>{error}</span>
						</div>
					) : null}

					<form className="auth-form" onSubmit={submit}>
						<label htmlFor="account-credential">API Key 或 account id</label>
						<div className="input-with-action">
							<input
								id="account-credential"
								autoCapitalize="none"
								autoComplete="off"
								autoCorrect="off"
								autoFocus
								disabled={loading}
								maxLength={512}
								onChange={(event) => setCredential(event.target.value)}
								placeholder="输入 API Key 或 account id"
								required
								spellCheck={false}
								type={visible ? "text" : "password"}
								value={credential}
							/>
							<button
								aria-label={visible ? "隐藏访问凭据" : "显示访问凭据"}
								className="input-action"
								disabled={loading}
								onClick={() => setVisible((value) => !value)}
								type="button"
							>
								<LookupIcon name={visible ? "eye-off" : "eye"} />
							</button>
						</div>
						<button className="button button-primary auth-submit" disabled={loading}>
							{loading ? <span className="spinner" aria-hidden="true" /> : null}
							{loading ? "查询中…" : "查看账户用量"}
						</button>
					</form>
					<p className="auth-footnote">访问凭据仅用于本次用量查询</p>
				</main>
			</div>
		</div>
	);
}

function LookupIcon({ name }: { name: "alert" | "eye" | "eye-off" }) {
	return (
		<svg className="icon" aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24">
			{name === "alert" ? (
				<><path d="M12 9v4" /><path d="M12 17h.01" /><path d="M10.3 3.9 2.4 18a2 2 0 0 0 1.75 3h15.7a2 2 0 0 0 1.75-3L13.7 3.9a2 2 0 0 0-3.4 0Z" /></>
			) : name === "eye" ? (
				<><path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12Z" /><circle cx="12" cy="12" r="2.5" /></>
			) : (
				<><path d="m3 3 18 18" /><path d="M10.6 6.15A10.6 10.6 0 0 1 12 6c6.5 0 10 6 10 6a16.8 16.8 0 0 1-3 3.8" /><path d="M6.6 6.6C3.5 8.4 2 12 2 12s3.5 6 10 6a10.7 10.7 0 0 0 3.4-.55" /></>
			)}
		</svg>
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
