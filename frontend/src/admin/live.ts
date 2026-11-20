// The console's live stream: one EventSource per admin session that delivers
// in-flight connections and data versions, so views refetch only on change.

import { useEffect, useRef, useState } from "react";

import type { LiveSnapshot } from "@/admin-api";

/** How long a broken stream may take to reconnect before views fall back to polling. */
const OFFLINE_AFTER_MS = 5_000;
/** Retry delay after the server refused the stream (signed out, restarting). */
const RETRY_REFUSED_MS = 10_000;

export interface LiveStream {
	snapshot: LiveSnapshot | null;
	/** False until the first snapshot and while reconnecting failed for a while. */
	connected: boolean;
}

/**
 * Subscribes to `url`. The server ends each stream after a minute so the
 * session is re-checked; EventSource reconnects on its own. When the server
 * refuses the stream, `onRefused` runs (to detect an expired session) and the
 * stream is retried later.
 */
export function useLiveStream(url: string, onRefused: () => void): LiveStream {
	const [snapshot, setSnapshot] = useState<LiveSnapshot | null>(null);
	const [connected, setConnected] = useState(false);
	const refusedRef = useRef(onRefused);

	useEffect(() => {
		refusedRef.current = onRefused;
	});

	useEffect(() => {
		let source: EventSource | null = null;
		let offlineTimer: number | null = null;
		let retryTimer: number | null = null;
		let closed = false;

		const clearOffline = () => {
			if (offlineTimer !== null) window.clearTimeout(offlineTimer);
			offlineTimer = null;
		};
		const open = () => {
			retryTimer = null;
			source = new EventSource(url, { withCredentials: true });
			source.addEventListener("snapshot", (event) => {
				try {
					const next = JSON.parse((event as MessageEvent<string>).data) as LiveSnapshot;
					clearOffline();
					setSnapshot(next);
					setConnected(true);
				} catch {
					// Ignore a malformed frame; the next snapshot replaces it.
				}
			});
			source.addEventListener("error", () => {
				if (closed || !source) return;
				if (source.readyState === EventSource.CLOSED) {
					source = null;
					clearOffline();
					setConnected(false);
					refusedRef.current();
					retryTimer = window.setTimeout(open, RETRY_REFUSED_MS);
				} else if (offlineTimer === null) {
					// Reconnecting; only report offline if it does not come back soon.
					offlineTimer = window.setTimeout(() => {
						offlineTimer = null;
						setConnected(false);
					}, OFFLINE_AFTER_MS);
				}
			});
		};
		open();
		return () => {
			closed = true;
			source?.close();
			clearOffline();
			if (retryTimer !== null) window.clearTimeout(retryTimer);
		};
	}, [url]);

	return { snapshot, connected };
}

/**
 * Counts changes of `value`, at most one per `intervalMs` (trailing changes
 * still count). The first value is the baseline and does not count.
 */
export function useThrottledTick(value: number | undefined, intervalMs: number): number {
	const [tick, setTick] = useState(0);
	const seen = useRef<number | undefined>(undefined);
	const lastFired = useRef(0);
	const timer = useRef<number | null>(null);

	useEffect(() => {
		if (value === undefined) return;
		if (seen.current === undefined) {
			seen.current = value;
			return;
		}
		if (seen.current === value) return;
		seen.current = value;
		if (timer.current !== null) return;
		const wait = Math.max(0, lastFired.current + intervalMs - Date.now());
		timer.current = window.setTimeout(() => {
			timer.current = null;
			lastFired.current = Date.now();
			setTick((current) => current + 1);
		}, wait);
	}, [value, intervalMs]);

	useEffect(
		() => () => {
			if (timer.current !== null) window.clearTimeout(timer.current);
			timer.current = null;
		},
		[],
	);
	return tick;
}
