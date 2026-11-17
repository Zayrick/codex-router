import { useSyncExternalStore } from "react";

// The page heading's "导出 CSV" button lives outside the logs view; the view
// publishes the export URL of its current filters here.

let current: string | null = null;
const listeners = new Set<() => void>();

export function publishExportUrl(url: string | null): void {
	if (url === current) return;
	current = url;
	for (const listener of listeners) listener();
}

export function useExportUrl(): string | null {
	return useSyncExternalStore(
		(listener) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		() => current,
	);
}
