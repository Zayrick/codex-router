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

const REASONING_EFFORT_LABELS: Readonly<Record<string, string>> = {
	"": "未记录",
	default: "默认",
	none: "关闭",
	minimal: "极低",
	low: "低",
	medium: "中",
	high: "高",
	xhigh: "超高",
	max: "最高",
};

const SERVICE_TIER_LABELS: Readonly<Record<string, string>> = {
	"": "未记录",
	default: "标准",
	priority: "Fast",
	flex: "Flex",
};

export function reasoningEffortLabel(value: string): string {
	return REASONING_EFFORT_LABELS[value] ?? value;
}

export function serviceTierLabel(value: string): string {
	return SERVICE_TIER_LABELS[value] ?? value;
}
