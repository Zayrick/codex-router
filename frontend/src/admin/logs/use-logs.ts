import { useCallback, useMemo, useState } from "react";

import { AdminSessionExpiredError, type EventPageQuery, type SessionGroup, type UsageEvent, type UsageEventPage, type UsageSessionKind } from "@/admin-api";
import { useResource } from "@/admin/use-resource";

export const PAGE_SIZE = 50;

export interface LogsData {
	page: UsageEventPage | null;
	error: string | null;
	loading: boolean;
	reload: () => void;
	events: UsageEvent[];
	sessions: SessionGroup[];
	sessionCounts: UsageEventPage["sessionCounts"];
	loadMore: () => void;
	loadingMore: boolean;
	canLoadMore: boolean;
}

/**
 * First page (refreshed by `refreshMs`) plus pages appended with
 * offset = loaded count. Appended pages reset whenever the query changes.
 */
export function useLogs(
	getEvents: (query: EventPageQuery, signal: AbortSignal) => Promise<UsageEventPage>,
	query: EventPageQuery,
	options: { refreshMs?: number; onSessionExpired: () => void; onError?: (message: string) => void },
): LogsData {
	const key = JSON.stringify(query);
	const first = useResource((signal) => getEvents({ ...query, offset: 0, limit: PAGE_SIZE }, signal), [key], {
		onSessionExpired: options.onSessionExpired,
		...(options.refreshMs ? { refreshMs: options.refreshMs } : {}),
	});
	const [more, setMore] = useState<{ key: string; pages: UsageEventPage[] }>({ key, pages: [] });
	const [loadingMore, setLoadingMore] = useState(false);
	const pages = useMemo(() => (more.key === key ? more.pages : []), [key, more]);

	const events = useMemo(() => dedupe([first.data?.events ?? [], ...pages.map((page) => page.events)].flat(), (event) => String(event.id)), [first.data, pages]);
	const sessions = useMemo(
		() => dedupe([first.data?.sessions ?? [], ...pages.map((page) => page.sessions ?? [])].flat(), (group) => group.sessionId || `user:${group.userId}`),
		[first.data, pages],
	);
	const sessionCounts = useMemo(() => Object.assign({}, ...pages.map((page) => page.sessionCounts), first.data?.sessionCounts ?? {}) as UsageEventPage["sessionCounts"], [first.data, pages]);

	const grouped = query.group === "session";
	const loaded = grouped ? sessions.length : events.length;
	const total = first.data?.total ?? 0;

	const { onSessionExpired, onError } = options;
	const loadMore = useCallback(() => {
		if (loadingMore) return;
		setLoadingMore(true);
		const controller = new AbortController();
		getEvents({ ...query, offset: loaded, limit: PAGE_SIZE }, controller.signal)
			.then((page) => setMore((current) => ({ key, pages: [...(current.key === key ? current.pages : []), page] })))
			.catch((error: unknown) => {
				if (error instanceof AdminSessionExpiredError) onSessionExpired();
				else onError?.(error instanceof Error ? error.message : "加载更多失败。");
			})
			.finally(() => setLoadingMore(false));
	}, [getEvents, key, loaded, loadingMore, onError, onSessionExpired, query]);

	return {
		page: first.data,
		error: first.error,
		loading: first.loading,
		reload: first.reload,
		events,
		sessions,
		sessionCounts,
		loadMore,
		loadingMore,
		canLoadMore: loaded < total,
	};
}

function dedupe<T>(items: T[], key: (item: T) => string): T[] {
	const seen = new Set<string>();
	return items.filter((item) => {
		const id = key(item);
		if (seen.has(id)) return false;
		seen.add(id);
		return true;
	});
}

const KIND_ORDER: UsageSessionKind[] = ["main", "subagent", "fork", "compaction"];
const KIND_LABEL: Record<string, string> = { main: "主会话", subagent: "子代理", fork: "分叉", compaction: "压缩" };

export function sessionRoot(event: UsageEvent): string {
	return event.parentSessionId || event.sessionId;
}

/** Requests of the event's session tree and their "主会话 1 · 子代理 3" breakdown. */
export function sessionSummary(event: UsageEvent, counts: UsageEventPage["sessionCounts"]): { total: number; text: string } | null {
	const root = sessionRoot(event);
	const kinds = root ? counts[root] : undefined;
	if (!kinds) return null;
	const total = Object.values(kinds).reduce((sum, value) => sum + (value ?? 0), 0);
	const text = KIND_ORDER.filter((kind) => kinds[kind])
		.map((kind) => `${KIND_LABEL[kind]} ${kinds[kind]}`)
		.join(" · ");
	return { total, text };
}

/** Models of the range from the backend, plus any still selected. */
export function useModelOptions(fromPage: string[] | undefined, selected: string[], fallback: string[]): string[] {
	return useMemo(() => [...new Set([...(fromPage ?? fallback), ...selected])].sort(), [fallback, fromPage, selected]);
}
