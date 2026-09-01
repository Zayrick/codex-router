import {
	useCallback,
	useEffect,
	useRef,
	useState,
	type FormEvent,
} from "react";
import {
	EyeIcon,
	EyeOffIcon,
	RefreshCwIcon,
	TriangleAlertIcon,
} from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyHeader } from "@/components/ui/empty";
import { Field, FieldLabel } from "@/components/ui/field";
import {
	InputGroup,
	InputGroupAddon,
	InputGroupButton,
	InputGroupInput,
} from "@/components/ui/input-group";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
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
		return <ScrollArea className="h-svh"><AccountLookupView error={error} loading={loading} onSubmit={lookup} /></ScrollArea>;
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
							onClick={clearAccount}
							size="sm"
							type="button"
							variant="outline"
						>
							更换凭据
						</Button>
					</div>
					<div className="public-account-heading">
						<Badge className="public-account-kind" variant="outline">
							{snapshot.account.identityType === "auth_proxy" ? "ACCOUNT ID" : "API KEY"}
						</Badge>
						<h1>用量信息</h1>
						<p>查看额度周期、Token 活动与模型消耗分布。</p>
					</div>
				</header>

				{error ? <Alert className="public-account-alert" variant="destructive"><TriangleAlertIcon /><AlertDescription>{error}</AlertDescription></Alert> : null}

				<div className={loading ? "public-account-content is-refreshing" : "public-account-content"}>
					<Card className="public-account-range-bar" aria-label="统计时间范围">
						<div className="public-account-range-copy">
							<strong>统计范围</strong>
							<span>{formatDateRange(usage.startAt, usage.endAt)}</span>
						</div>
						<div className="public-account-range-actions">
							<ScrollArea className="public-account-range-scroll" aria-label="选择统计时间范围" scrollbars="horizontal">
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
						<Card className="public-account-metrics" aria-label="账户用量指标">
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
							<Empty className="public-account-quota-empty border"><EmptyHeader><EmptyDescription>额度时间条尚未完成首次同步。</EmptyDescription></EmptyHeader></Empty>
						)}
					</section>
				</div>
			</div>
			</main>
		</ScrollArea>
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
						<Alert className="mt-4" variant="destructive">
							<TriangleAlertIcon />
							<AlertDescription>{error}</AlertDescription>
						</Alert>
					) : null}

					<form className="auth-form" onSubmit={submit}>
						<Field>
							<FieldLabel htmlFor="account-credential">API Key 或 account id</FieldLabel>
							<InputGroup className="h-10">
								<InputGroupInput
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
								<InputGroupAddon align="inline-end">
									<InputGroupButton aria-label={visible ? "隐藏访问凭据" : "显示访问凭据"} disabled={loading} onClick={() => setVisible((value) => !value)} size="icon-sm">
										{visible ? <EyeOffIcon /> : <EyeIcon />}
									</InputGroupButton>
								</InputGroupAddon>
							</InputGroup>
						</Field>
						<Button className="auth-submit" disabled={loading} size="lg" type="submit">
							{loading ? <Spinner /> : null}
							{loading ? "查询中…" : "查看账户用量"}
						</Button>
					</form>
					<p className="auth-footnote">访问凭据仅用于本次用量查询</p>
				</main>
			</div>
		</div>
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
