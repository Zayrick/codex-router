import { useCallback, useEffect, useRef, useState } from "react";

import { AdminSessionExpiredError } from "@/admin-api";

export interface Resource<T> {
	data: T | null;
	error: string | null;
	/** True while a request is in flight (including background refreshes). */
	loading: boolean;
	reload: () => void;
}

/**
 * Loads data with abort-on-change. Keeps the previous data while reloading so
 * views do not flash; `refreshMs` polls while the tab is visible, and a
 * changed `refreshKey` (for example a pushed data version) reloads once.
 */
export function useResource<T>(
	load: (signal: AbortSignal) => Promise<T>,
	deps: readonly unknown[],
	options: { refreshMs?: number; refreshKey?: unknown; onSessionExpired?: () => void; enabled?: boolean } = {},
): Resource<T> {
	const [data, setData] = useState<T | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [loading, setLoading] = useState(false);
	const [version, setVersion] = useState(0);
	const loadRef = useRef(load);
	const expiredRef = useRef(options.onSessionExpired);
	const enabled = options.enabled ?? true;

	useEffect(() => {
		loadRef.current = load;
		expiredRef.current = options.onSessionExpired;
	});

	useEffect(() => {
		if (!enabled) return;
		const controller = new AbortController();
		setLoading(true);
		loadRef
			.current(controller.signal)
			.then((value) => {
				if (controller.signal.aborted) return;
				setData(value);
				setError(null);
			})
			.catch((reason: unknown) => {
				if (controller.signal.aborted) return;
				if (reason instanceof AdminSessionExpiredError) {
					expiredRef.current?.();
					return;
				}
				setError(reason instanceof Error && reason.message ? reason.message : "读取数据失败。");
			})
			.finally(() => {
				if (!controller.signal.aborted) setLoading(false);
			});
		return () => controller.abort();
		// eslint-disable-next-line react-hooks/exhaustive-deps -- callers list the inputs of `load`.
	}, [...deps, version, enabled]);

	useEffect(() => {
		if (!options.refreshMs || !enabled) return;
		const timer = window.setInterval(() => {
			if (!document.hidden) setVersion((value) => value + 1);
		}, options.refreshMs);
		return () => window.clearInterval(timer);
	}, [options.refreshMs, enabled]);

	const [seenKey, setSeenKey] = useState(options.refreshKey);
	if (seenKey !== options.refreshKey) {
		setSeenKey(options.refreshKey);
		setVersion((value) => value + 1);
	}

	const reload = useCallback(() => setVersion((value) => value + 1), []);
	return { data, error, loading, reload };
}
