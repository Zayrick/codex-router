// Data the console loads before its first render, and error text helpers.

import { AdminApiError, type AdminApiClient, type AdminSettings, type AdminState } from "@/admin-api";

import type { PricingState } from "./context";

export interface InitialData {
	state: AdminState;
	settings: AdminSettings;
	pricing: PricingState;
}


export function errorMessage(error: unknown, fallback: string): string {
	return error instanceof AdminApiError || error instanceof Error ? error.message || fallback : fallback;
}

/** Loads the data the console needs before its first render. */
export async function loadInitialData(api: AdminApiClient): Promise<InitialData> {
	const [state, settings, pricing] = await Promise.all([api.getState(), api.getSettings(), api.getPricing()]);
	return { state, settings, pricing: { prices: pricing.prices, usedModels: pricing.usedModels, sync: pricing.sync } };
}
