import { useState } from "react";

import { useAdmin } from "@/admin/context";

import { priceChanges, setDraft, toPrice, usePriceDraft, validRow } from "./store";

/** Save the draft; returns whether it was written. */
export function useSavePrices() {
	const { pricing, savePricing, notify } = useAdmin();
	const draft = usePriceDraft();
	const [saving, setSaving] = useState(false);
	const dirty = priceChanges(pricing.prices, draft).length > 0;

	async function save() {
		if (!draft || !dirty || saving) return;
		const invalid = draft.find((row) => !validRow(row));
		if (invalid) {
			notify(`${invalid.model || "新模型"} 的价格无效：单价需为不小于 0 的数字。`, "error");
			return;
		}
		const names = new Set<string>();
		for (const row of draft) {
			const key = row.model.trim().toLowerCase();
			if (names.has(key)) {
				notify(`模型 ${row.model} 重复。`, "error");
				return;
			}
			names.add(key);
		}
		setSaving(true);
		const saved = await savePricing(draft.map(toPrice));
		setSaving(false);
		if (saved) setDraft(null);
	}

	return { dirty, saving, save };
}
