import {
	useCallback,
	useEffect,
	useState,
} from "react";
import QuotaTimeline, { type QuotaTimelineWindow } from "./QuotaTimeline";

const REFRESH_INTERVAL_MS = 5 * 60 * 1_000;
const CLOCK_INTERVAL_MS = 1_000;
const HOUR_MS = 60 * 60 * 1_000;
const DAY_MS = 24 * HOUR_MS;
const MOCK_USAGE = import.meta.env.DEV && new URLSearchParams(window.location.search).has("mock");

type UsageWindow = QuotaTimelineWindow;

interface UsageSnapshot {
	sampledAt: number;
	planType?: string | null;
	windows: UsageWindow[];
}

function StatusUsage() {
	const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const [now, setNow] = useState(() => Date.now());

	const load = useCallback(async (signal?: AbortSignal) => {
		try {
			if (MOCK_USAGE) {
				setSnapshot(mockSnapshot(Date.now()));
				setError(null);
				return;
			}
			const init: RequestInit = {
				headers: { accept: "application/json" },
			};
			if (signal) init.signal = signal;
			const response = await fetch("/status/usage/data", init);
			if (!response.ok) throw new Error(`HTTP ${response.status}`);
			const value: unknown = await response.json();
			const next = parseSnapshot(value);
			setSnapshot(next);
			setError(null);
		} catch (cause) {
			if (cause instanceof DOMException && cause.name === "AbortError") return;
			setError("暂时无法读取用量缓存，将在下个刷新周期重试。");
		} finally {
			setLoading(false);
		}
	}, []);

	useEffect(() => {
		const controller = new AbortController();
		const initialLoad = window.setTimeout(() => {
			void load(controller.signal);
		}, 0);
		const refresh = window.setInterval(() => {
			if (!document.hidden) void load();
		}, REFRESH_INTERVAL_MS);
		const clock = window.setInterval(() => setNow(Date.now()), CLOCK_INTERVAL_MS);
		const onVisibilityChange = () => {
			if (!document.hidden) {
				setNow(Date.now());
				void load();
			}
		};
		document.addEventListener("visibilitychange", onVisibilityChange);
		return () => {
			controller.abort();
			window.clearTimeout(initialLoad);
			window.clearInterval(refresh);
			window.clearInterval(clock);
			document.removeEventListener("visibilitychange", onVisibilityChange);
		};
	}, [load]);

	return (
		<main className="usage-page">
			<section className="usage-shell" aria-labelledby="usage-title">
				<header className="usage-header">
					<div>
						<p className="usage-eyebrow">CODEX ROUTER STATUS</p>
						<h1 id="usage-title">配额窗口</h1>
					</div>
				</header>

				{error ? <div className="usage-message usage-error" role="status">{error}</div> : null}
				{loading && !snapshot ? <LoadingTimeline /> : null}
				{!loading && !snapshot ? (
					<div className="usage-message" role="status">暂无用量缓存，定时任务完成首次采样后会自动显示。</div>
				) : null}

				{snapshot ? (
					<QuotaTimeline
						now={now}
						planType={snapshot.planType}
						sampledAt={snapshot.sampledAt}
						windows={snapshot.windows}
					/>
				) : null}
			</section>
		</main>
	);
}

function LoadingTimeline() {
	return <div className="timeline-loading" role="status"><span /><span /><span /></div>;
}

function parseSnapshot(value: unknown): UsageSnapshot | null {
	if (typeof value !== "object" || value === null || Array.isArray(value)) {
		throw new Error("Invalid usage response");
	}
	const snapshot = value as Record<string, unknown>;
	if (snapshot.snapshot === null) return null;
	if (validTimestamp(snapshot.sampledAt) === null || !Array.isArray(snapshot.windows)) {
		throw new Error("Invalid usage snapshot");
	}
	return value as UsageSnapshot;
}

function validTimestamp(value: unknown): number | null {
	return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}

function mockSnapshot(now: number): UsageSnapshot {
	return {
		sampledAt: now,
		planType: "pro",
		windows: [
			{
				id: "mock-codex-five-hour",
				category: "codex",
				name: "Codex",
				kind: "five_hour",
				usedPercent: 32,
				remainingPercent: 68,
				limitWindowSeconds: 5 * 60 * 60,
				resetAt: now + 2.25 * HOUR_MS,
			},
			{
				id: "mock-codex-weekly",
				category: "codex",
				name: "Codex",
				kind: "weekly",
				usedPercent: 59,
				remainingPercent: 41,
				limitWindowSeconds: 7 * 24 * 60 * 60,
				resetAt: now + 3.8 * DAY_MS,
			},
			{
				id: "mock-spark-weekly",
				category: "additional",
				name: "GPT-5.3-Codex-Spark",
				kind: "weekly",
				usedPercent: 77,
				remainingPercent: 23,
				limitWindowSeconds: 7 * 24 * 60 * 60,
				resetAt: now + 5.4 * DAY_MS,
			},
			{
				id: "mock-review-weekly",
				category: "code_review",
				name: "Code Review",
				kind: "weekly",
				usedPercent: 16,
				remainingPercent: 84,
				limitWindowSeconds: 7 * 24 * 60 * 60,
				resetAt: now + 1.6 * DAY_MS,
			},
		],
	};
}

export default StatusUsage;
