// Unsaved price edits, shared by the pricing page and its heading actions
// (rendered separately by the shell).

import { useSyncExternalStore } from "react";

import type { ModelPrice } from "@/admin-api";

export const PRICE_FIELDS = ["input", "output", "cacheRead", "cacheWrite", "multiplier"] as const;
export type PriceField = (typeof PRICE_FIELDS)[number];

export const PRICE_FIELD_LABELS: Record<PriceField, string> = {
	input: "普通输入",
	output: "输出",
	cacheRead: "缓存读取",
	cacheWrite: "缓存写入",
	multiplier: "倍率",
};

/** A price row being edited; values stay strings while typing. */
export type DraftRow = { model: string } & Record<PriceField, string>;

let draft: DraftRow[] | null = null;
const listeners = new Set<() => void>();

function emit() {
	for (const listener of listeners) listener();
}

export function setDraft(next: DraftRow[] | null) {
	draft = next;
	emit();
}

export function usePriceDraft(): DraftRow[] | null {
	return useSyncExternalStore(
		(listener) => {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		() => draft,
	);
}

export function toRow(price: ModelPrice): DraftRow {
	return {
		model: price.model,
		input: String(price.input),
		output: String(price.output),
		cacheRead: String(price.cacheRead),
		cacheWrite: String(price.cacheWrite),
		multiplier: String(price.multiplier),
	};
}

export function parseValue(value: string): number {
	return value.trim() === "" ? Number.NaN : Number(value);
}

export function validRow(row: DraftRow): boolean {
	return row.model.trim().length > 0 && PRICE_FIELDS.every((field) => {
		const value = parseValue(row[field]);
		return Number.isFinite(value) && value >= 0;
	});
}

export function toPrice(row: DraftRow): ModelPrice {
	return {
		model: row.model.trim(),
		input: parseValue(row.input),
		output: parseValue(row.output),
		cacheRead: parseValue(row.cacheRead),
		cacheWrite: parseValue(row.cacheWrite),
		multiplier: parseValue(row.multiplier),
	};
}

export function sameModel(left: string, right: string): boolean {
	return left.trim().toLowerCase() === right.trim().toLowerCase();
}

export function samePrice(left: ModelPrice, right: ModelPrice): boolean {
	return sameModel(left.model, right.model) && PRICE_FIELDS.every((field) => left[field] === right[field]);
}

export interface PriceChange {
	model: string;
	kind: "added" | "removed" | "changed";
	fields: { field: PriceField; from: number; to: number }[];
}

/** Differences between the saved prices and the draft. */
export function priceChanges(base: ModelPrice[], rows: DraftRow[] | null): PriceChange[] {
	if (!rows) return [];
	const changes: PriceChange[] = [];
	for (const row of rows) {
		const saved = base.find((price) => sameModel(price.model, row.model));
		if (!saved) {
			changes.push({ model: row.model, kind: "added", fields: [] });
			continue;
		}
		const fields = PRICE_FIELDS.flatMap((field) => {
			const to = parseValue(row[field]);
			return to === saved[field] ? [] : [{ field, from: saved[field], to }];
		});
		if (fields.length) changes.push({ model: saved.model, kind: "changed", fields });
	}
	for (const price of base) {
		if (!rows.some((row) => sameModel(row.model, price.model))) changes.push({ model: price.model, kind: "removed", fields: [] });
	}
	return changes;
}

/** Cost in USD of the given token counts at `price` (Fast surcharge excluded). */
export function estimateCost(
	tokens: { inputTokens: number; cachedInputTokens: number; cacheCreationInputTokens: number; outputTokens: number },
	price: ModelPrice,
): number {
	const regular = Math.max(0, tokens.inputTokens - tokens.cachedInputTokens - tokens.cacheCreationInputTokens);
	const unscaled =
		regular * price.input + tokens.cachedInputTokens * price.cacheRead + tokens.cacheCreationInputTokens * price.cacheWrite + tokens.outputTokens * price.output;
	return (unscaled / 1_000_000) * price.multiplier;
}
