const COST_FORMAT = new Intl.NumberFormat("en-US", {
	style: "currency",
	currency: "USD",
	minimumFractionDigits: 2,
	maximumFractionDigits: 4,
});

export function formatCost(value: number): string {
	const normalized = Math.max(0, Number.isFinite(value) ? value : 0);
	return normalized > 0 && normalized < 0.01 ? `$${normalized.toFixed(4)}` : COST_FORMAT.format(normalized);
}
