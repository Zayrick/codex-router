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

const SERVICE_TIER_LABELS: Readonly<Record<string, string>> = {
	"": "未记录",
	auto: "自动",
	default: "标准",
	priority: "Fast",
	flex: "Flex",
};

/** Effort values are shown as sent upstream; only missing history gets a label. */
export function reasoningEffortLabel(value: string): string {
	return value || "未记录";
}

export function serviceTierLabel(value: string): string {
	return SERVICE_TIER_LABELS[value] ?? value;
}
