// Color scales for day × hour heatmaps.

/** Empty cell (no requests) and not-yet-reached cell styles. */
export const EMPTY_HEAT_COLOR = "var(--surface-subtle)";
const EMPTY_CELL = EMPTY_HEAT_COLOR;

/** Blue intensity ramp for counts (requests, tokens). */
export const HEAT_BLUE = ["#dfe6ff", "#a9bdff", "#7b9bff", "#4f7cff"] as const;
/** Teal intensity ramp for cost. */
export const HEAT_TEAL = ["#d3f2ee", "#99e2d9", "#4cc9b9", "#14b8a6"] as const;
/** Success-rate buckets: ≥ 99 %, 95–99 %, 80–95 %, < 80 %. */
export const HEAT_HEALTH = ["#0e9f6e", "#7fcfb3", "#f59e0b", "#c65746"] as const;

/** Color for a count relative to `max` on a ramp; 0 is the empty color. */
export function rampColor(value: number, max: number, ramp: readonly string[]): string {
	if (value <= 0 || max <= 0) return EMPTY_CELL;
	const index = Math.min(ramp.length - 1, Math.floor((value / max) * ramp.length));
	return ramp[Math.max(0, index)] ?? EMPTY_CELL;
}

/** Color for a success ratio (0–1); `null` means no requests. */
export function healthColor(ratio: number | null): string {
	if (ratio === null) return EMPTY_CELL;
	if (ratio >= 0.99) return HEAT_HEALTH[0];
	if (ratio >= 0.95) return HEAT_HEALTH[1];
	if (ratio >= 0.8) return HEAT_HEALTH[2];
	return HEAT_HEALTH[3];
}

