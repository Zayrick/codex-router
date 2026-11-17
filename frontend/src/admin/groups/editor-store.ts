import { useSyncExternalStore } from "react";

import type { AccountGroup } from "@/admin-api";

/** Group being edited; shared so the page heading action can open the editor. */
let current: AccountGroup | "new" | null = null;
const listeners = new Set<() => void>();

export function openGroupEditor(entry: AccountGroup | "new" | null): void {
	current = entry;
	for (const listener of listeners) listener();
}

export function useGroupEditor(): AccountGroup | "new" | null {
	return useSyncExternalStore(
		(listener) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		() => current,
	);
}
