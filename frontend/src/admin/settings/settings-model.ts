import type { AdminSettings, CodexAccount } from "@/admin-api";

export const MAX_FAST_COST_MULTIPLIER = 100;

export type SettingsSection = "public" | "upstream" | "billing" | "events" | "recipients" | "channels";

/** Editable copy: the multiplier stays a string while typing. */
export interface SettingsDraft {
	showQuota: boolean;
	codexClientVersion: string;
	fastCostMultiplier: string;
	resetWatchEnabled: boolean;
	quotaResetEnabled: boolean;
	usageWarningEnabled: boolean;
	allAccounts: boolean;
	accountIds: string[];
	resetWatchApiUrl: string;
	barkEnabled: boolean;
	barkPushUrl: string;
	dingtalkEnabled: boolean;
	dingtalkWebhookUrl: string;
	dingtalkSecret: string;
}

export function toDraft(settings: AdminSettings): SettingsDraft {
	const notifications = settings.notifications;
	return {
		showQuota: settings.publicAccount.showQuota,
		codexClientVersion: settings.upstream.codexClientVersion,
		fastCostMultiplier: String(settings.usage.fastCostMultiplier),
		resetWatchEnabled: notifications.resetWatchEnabled,
		quotaResetEnabled: notifications.quotaResetEnabled,
		usageWarningEnabled: notifications.usageWarningEnabled,
		allAccounts: notifications.allAccounts,
		accountIds: [...notifications.accountIds],
		resetWatchApiUrl: notifications.resetWatchApiUrl,
		barkEnabled: notifications.bark.enabled,
		barkPushUrl: notifications.bark.pushUrl,
		dingtalkEnabled: notifications.dingtalk.enabled,
		dingtalkWebhookUrl: notifications.dingtalk.webhookUrl,
		dingtalkSecret: notifications.dingtalk.secret,
	};
}

export function toSettings(draft: SettingsDraft): AdminSettings {
	return {
		publicAccount: { showQuota: draft.showQuota },
		upstream: { codexClientVersion: draft.codexClientVersion.trim() },
		usage: { fastCostMultiplier: Number(draft.fastCostMultiplier) },
		notifications: {
			resetWatchEnabled: draft.resetWatchEnabled,
			quotaResetEnabled: draft.quotaResetEnabled,
			usageWarningEnabled: draft.usageWarningEnabled,
			allAccounts: draft.allAccounts,
			accountIds: draft.allAccounts ? [] : draft.accountIds,
			resetWatchApiUrl: draft.resetWatchApiUrl.trim(),
			bark: { enabled: draft.barkEnabled, pushUrl: draft.barkPushUrl.trim() },
			dingtalk: { enabled: draft.dingtalkEnabled, webhookUrl: draft.dingtalkWebhookUrl.trim(), secret: draft.dingtalkSecret.trim() },
		},
	};
}

export function parseMultiplier(value: string): number {
	return value.trim() ? Number(value) : Number.NaN;
}

export function validMultiplier(value: string): boolean {
	const parsed = parseMultiplier(value);
	return Number.isFinite(parsed) && parsed > 0 && parsed <= MAX_FAST_COST_MULTIPLIER;
}

export function validClientVersion(value: string): boolean {
	const version = value.trim();
	return version.length <= 64 && /^\d+\.\d+\.\d+$/.test(version);
}

export function isHttpsUrl(value: string): boolean {
	try {
		const url = new URL(value.trim());
		return url.protocol === "https:" && Boolean(url.hostname);
	} catch {
		return false;
	}
}

/** First validation problem of the draft, if any. */
export function validationError(draft: SettingsDraft): string | null {
	if (!validClientVersion(draft.codexClientVersion)) return "Codex 客户端版本需使用 x.y.z 格式，例如 0.168.0。";
	if (!validMultiplier(draft.fastCostMultiplier)) return `Fast 计费倍率必须大于 0 且不超过 ${MAX_FAST_COST_MULTIPLIER}。`;
	if (!isHttpsUrl(draft.resetWatchApiUrl)) return "重置预测 API 地址必须是有效的 HTTPS 链接。";
	if (draft.barkPushUrl.trim() && !isHttpsUrl(draft.barkPushUrl)) return "Bark 推送链接必须使用 HTTPS。";
	if (draft.barkEnabled && !isHttpsUrl(draft.barkPushUrl)) return "启用 Bark 前，请填写有效的 HTTPS 推送链接。";
	const webhook = draft.dingtalkWebhookUrl.trim();
	const secret = draft.dingtalkSecret.trim();
	if (Boolean(webhook) !== Boolean(secret) || (webhook && !isHttpsUrl(webhook))) return "钉钉 Webhook 与加签密钥需要同时填写，Webhook 需使用 HTTPS。";
	if (draft.dingtalkEnabled && !webhook) return "启用钉钉前，请填写 Webhook 和加签密钥。";
	return null;
}

export interface SettingsChange {
	section: SettingsSection;
	text: string;
}

const SECTION_LABELS: Record<SettingsSection, string> = {
	public: "用户端",
	upstream: "上游",
	billing: "计费",
	events: "通知事件",
	recipients: "接收账户",
	channels: "通知通道",
};

export function sectionLabel(section: SettingsSection): string {
	return SECTION_LABELS[section];
}

function onOff(value: boolean): string {
	return value ? "开启" : "关闭";
}

/** Human-readable differences between saved settings and the draft. */
export function settingsChanges(saved: SettingsDraft, draft: SettingsDraft, accounts: CodexAccount[]): SettingsChange[] {
	const changes: SettingsChange[] = [];
	if (saved.showQuota !== draft.showQuota) changes.push({ section: "public", text: `显示账户额度 ${onOff(draft.showQuota)}` });
	if (saved.codexClientVersion.trim() !== draft.codexClientVersion.trim()) {
		changes.push({ section: "upstream", text: `客户端版本 ${saved.codexClientVersion} → ${draft.codexClientVersion.trim() || "—"}` });
	}
	if (parseMultiplier(saved.fastCostMultiplier) !== parseMultiplier(draft.fastCostMultiplier)) {
		changes.push({ section: "billing", text: `Fast 计费倍率 ${saved.fastCostMultiplier}× → ${draft.fastCostMultiplier || "—"}×` });
	}
	const events: [keyof SettingsDraft, string][] = [
		["resetWatchEnabled", "重置预测"],
		["quotaResetEnabled", "额度重置"],
		["usageWarningEnabled", "用量过高"],
	];
	for (const [key, label] of events) {
		if (saved[key] !== draft[key]) changes.push({ section: "events", text: `${label} ${onOff(Boolean(draft[key]))}` });
	}
	if (saved.resetWatchApiUrl.trim() !== draft.resetWatchApiUrl.trim()) changes.push({ section: "events", text: "重置预测 API 地址" });
	if (saved.allAccounts !== draft.allAccounts) changes.push({ section: "recipients", text: `所有账户 ${onOff(draft.allAccounts)}` });
	else if (!draft.allAccounts) {
		const added = draft.accountIds.filter((id) => !saved.accountIds.includes(id));
		const removed = saved.accountIds.filter((id) => !draft.accountIds.includes(id));
		const name = (id: string) => accounts.find((account) => account.id === id)?.name ?? id;
		if (added.length || removed.length) {
			changes.push({
				section: "recipients",
				text: [added.length ? `加入 ${added.map(name).join("、")}` : "", removed.length ? `移除 ${removed.map(name).join("、")}` : ""].filter(Boolean).join("，"),
			});
		}
	}
	if (saved.barkEnabled !== draft.barkEnabled) changes.push({ section: "channels", text: `Bark ${onOff(draft.barkEnabled)}` });
	if (saved.barkPushUrl.trim() !== draft.barkPushUrl.trim()) changes.push({ section: "channels", text: "Bark 推送链接" });
	if (saved.dingtalkEnabled !== draft.dingtalkEnabled) changes.push({ section: "channels", text: `钉钉机器人 ${onOff(draft.dingtalkEnabled)}` });
	if (saved.dingtalkWebhookUrl.trim() !== draft.dingtalkWebhookUrl.trim() || saved.dingtalkSecret.trim() !== draft.dingtalkSecret.trim()) {
		changes.push({ section: "channels", text: "钉钉 Webhook / 密钥" });
	}
	return changes;
}
