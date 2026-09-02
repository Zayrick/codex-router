import { useMemo, useState, type FormEvent, type ReactNode } from "react";
import {
	BellRingIcon,
	CheckCircle2Icon,
	EyeIcon,
	EyeOffIcon,
	GaugeIcon,
	Globe2Icon,
	MessageSquareMoreIcon,
	RadioTowerIcon,
	SaveIcon,
	SmartphoneIcon,
	TriangleAlertIcon,
	UsersIcon,
} from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardAction,
	CardContent,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
	Field,
	FieldContent,
	FieldDescription,
	FieldGroup,
	FieldLabel,
	FieldTitle,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
	InputGroup,
	InputGroupAddon,
	InputGroupButton,
	InputGroupInput,
} from "@/components/ui/input-group";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import type { AdminSettings, CodexAccount, ModelPrice } from "./admin-api";
import ModelPricingCard from "./ModelPricingCard";

interface SettingsPageProps {
	accounts: CodexAccount[];
	error: string | null;
	loading: boolean;
	onSave: (settings: AdminSettings) => void;
	settings: AdminSettings | null;
	saving: boolean;
	pricing: {
		error: string | null;
		loading: boolean;
		onSave: (prices: ModelPrice[]) => void;
		onSync: () => void;
		prices: ModelPrice[];
		saving: boolean;
		syncing: boolean;
		usedModels: string[];
	};
}

export default function SettingsPage({
	accounts,
	error,
	loading,
	onSave,
	settings,
	saving,
	pricing,
}: SettingsPageProps) {
	const [draft, setDraft] = useState<AdminSettings | null>(settings ? cloneSettings(settings) : null);
	const [secretVisible, setSecretVisible] = useState(false);

	const validationError = useMemo(() => draft ? validateSettings(draft) : null, [draft]);
	const changed = Boolean(draft && settings && JSON.stringify(draft) !== JSON.stringify(settings));

	function submit(event: FormEvent<HTMLFormElement>): void {
		event.preventDefault();
		if (!draft || validationError || saving) return;
		onSave(normalizeSettings(draft));
	}

	if (!draft) {
		return (
		<Card className="min-h-40 flex-row items-center justify-center gap-2 text-muted-foreground">
			<Spinner />
			<span>{loading ? "正在读取设置…" : "设置暂不可用"}</span>
		</Card>
		);
	}

	const accountEventsEnabled =
		draft.notifications.quotaResetEnabled || draft.notifications.usageWarningEnabled;

	return (
		<div className="settings-page">
		<form className="settings-form" onSubmit={submit}>
			<div className="settings-actions">
				<Button disabled={!changed || Boolean(validationError) || saving} type="submit">
					{saving ? <Spinner /> : <SaveIcon data-icon="inline-start" />}
					{saving ? "保存中…" : "保存设置"}
				</Button>
			</div>

			{error ? (
				<Alert variant="destructive">
					<TriangleAlertIcon />
					<AlertDescription>{error}</AlertDescription>
				</Alert>
			) : null}
			{validationError ? (
				<Alert variant="destructive">
					<TriangleAlertIcon />
					<AlertDescription>{validationError}</AlertDescription>
				</Alert>
			) : null}

			<Card>
				<CardHeader>
					<CardTitle className="settings-card-title"><Globe2Icon />用户端</CardTitle>
				</CardHeader>
				<CardContent>
					<Field className="settings-switch-row" orientation="horizontal">
						<FieldContent>
							<FieldTitle><GaugeIcon />显示账户额度</FieldTitle>
							<FieldDescription>显示分组内所有 Codex 账户的额度时间轴。</FieldDescription>
						</FieldContent>
						<Switch
							aria-label="用户端显示账户额度"
							checked={draft.publicAccount.showQuota}
							disabled={saving}
							onCheckedChange={(showQuota) => setDraft({ ...draft, publicAccount: { showQuota } })}
						/>
					</Field>
				</CardContent>
			</Card>

			<Card>
				<CardHeader className="border-b">
					<CardTitle className="settings-card-title"><BellRingIcon />通知</CardTitle>
				</CardHeader>
				<CardContent className="settings-notification-content">
					<section className="settings-section" aria-labelledby="notification-events-title">
						<div className="settings-section-heading">
							<h3 id="notification-events-title">通知事件</h3>
						</div>
						<div className="settings-event-grid">
							<NotificationEvent
								checked={draft.notifications.resetWatchEnabled}
								disabled={saving}
								icon={<RadioTowerIcon />}
								label="重置预测"
								onChange={(resetWatchEnabled) => setDraft({ ...draft, notifications: { ...draft.notifications, resetWatchEnabled } })}
							/>
							<NotificationEvent
								checked={draft.notifications.quotaResetEnabled}
								disabled={saving}
								icon={<CheckCircle2Icon />}
								label="额度重置"
								onChange={(quotaResetEnabled) => setDraft({ ...draft, notifications: { ...draft.notifications, quotaResetEnabled } })}
							/>
							<NotificationEvent
								checked={draft.notifications.usageWarningEnabled}
								disabled={saving}
								icon={<GaugeIcon />}
								label="用量过高"
								onChange={(usageWarningEnabled) => setDraft({ ...draft, notifications: { ...draft.notifications, usageWarningEnabled } })}
							/>
						</div>
						<Field>
							<FieldLabel htmlFor="reset-watch-api-url">重置预测 API 地址</FieldLabel>
							<Input
								disabled={saving}
								id="reset-watch-api-url"
								onChange={(event) => setDraft({ ...draft, notifications: { ...draft.notifications, resetWatchApiUrl: event.target.value } })}
								placeholder="https://example.com/api/status"
								spellCheck={false}
								type="url"
								value={draft.notifications.resetWatchApiUrl}
							/>
						</Field>
					</section>

					<Separator />

					<section className="settings-section" aria-labelledby="notification-accounts-title">
						<div className="settings-section-heading">
							<h3 id="notification-accounts-title">接收账户</h3>
						</div>
						<Field className="settings-switch-row" data-disabled={!accountEventsEnabled} orientation="horizontal">
							<FieldContent>
								<FieldTitle><UsersIcon />所有账户</FieldTitle>
								<FieldDescription>新账户自动纳入。</FieldDescription>
							</FieldContent>
							<Switch
								checked={draft.notifications.allAccounts}
								disabled={saving || !accountEventsEnabled}
								onCheckedChange={(allAccounts) => setDraft({
									...draft,
									notifications: {
										...draft.notifications,
										allAccounts,
										accountIds: !allAccounts && draft.notifications.accountIds.length === 0
											? accounts.map((account) => account.id)
											: draft.notifications.accountIds,
									},
								})}
							/>
						</Field>

						{accounts.length ? (
							<FieldGroup className="settings-account-grid" data-slot="checkbox-group">
								{accounts.map((account) => {
									const checked = draft.notifications.accountIds.includes(account.id);
									return (
										<FieldLabel htmlFor={`notification-account-${account.id}`} key={account.id}>
											<Field orientation="horizontal">
												<Checkbox
													checked={draft.notifications.allAccounts || checked}
													disabled={saving || !accountEventsEnabled || draft.notifications.allAccounts}
													id={`notification-account-${account.id}`}
													onCheckedChange={(value) => {
														const accountIds = value
															? [...draft.notifications.accountIds, account.id]
															: draft.notifications.accountIds.filter((id) => id !== account.id);
														setDraft({ ...draft, notifications: { ...draft.notifications, accountIds } });
													}}
												/>
												<FieldTitle>{account.name}{account.enabled ? "" : "（停用）"}</FieldTitle>
											</Field>
										</FieldLabel>
									);
								})}
							</FieldGroup>
						) : (
							<Alert><UsersIcon /><AlertDescription>暂无 Codex 账户。添加账户后可在此选择通知范围。</AlertDescription></Alert>
						)}
					</section>

					<Separator />

					<section className="settings-section" aria-labelledby="notification-channels-title">
						<div className="settings-section-heading">
							<h3 id="notification-channels-title">通知通道</h3>
						</div>
						<div className="settings-channel-grid">
							<Card className="settings-channel-card" size="sm">
								<CardHeader>
									<CardTitle className="settings-channel-title"><SmartphoneIcon />Bark</CardTitle>
									<CardAction><Switch aria-label="启用 Bark 通知" checked={draft.notifications.bark.enabled} disabled={saving} onCheckedChange={(enabled) => setDraft({ ...draft, notifications: { ...draft.notifications, bark: { ...draft.notifications.bark, enabled } } })} /></CardAction>
								</CardHeader>
								<CardContent>
									<Field data-disabled={!draft.notifications.bark.enabled}>
										<FieldLabel htmlFor="bark-push-url">推送链接</FieldLabel>
										<Input disabled={saving} id="bark-push-url" onChange={(event) => setDraft({ ...draft, notifications: { ...draft.notifications, bark: { ...draft.notifications.bark, pushUrl: event.target.value } } })} placeholder="https://api.day.app/device-key" spellCheck={false} type="url" value={draft.notifications.bark.pushUrl} />
									</Field>
								</CardContent>
							</Card>

							<Card className="settings-channel-card" size="sm">
								<CardHeader>
									<CardTitle className="settings-channel-title"><MessageSquareMoreIcon />钉钉机器人</CardTitle>
									<CardAction><Switch aria-label="启用钉钉通知" checked={draft.notifications.dingtalk.enabled} disabled={saving} onCheckedChange={(enabled) => setDraft({ ...draft, notifications: { ...draft.notifications, dingtalk: { ...draft.notifications.dingtalk, enabled } } })} /></CardAction>
								</CardHeader>
								<CardContent>
									<FieldGroup>
										<Field data-disabled={!draft.notifications.dingtalk.enabled}>
											<FieldLabel htmlFor="dingtalk-webhook-url">Webhook</FieldLabel>
											<Input disabled={saving} id="dingtalk-webhook-url" onChange={(event) => setDraft({ ...draft, notifications: { ...draft.notifications, dingtalk: { ...draft.notifications.dingtalk, webhookUrl: event.target.value } } })} placeholder="https://oapi.dingtalk.com/robot/send?access_token=…" spellCheck={false} type="url" value={draft.notifications.dingtalk.webhookUrl} />
										</Field>
										<Field data-disabled={!draft.notifications.dingtalk.enabled}>
											<FieldLabel htmlFor="dingtalk-secret">加签密钥</FieldLabel>
											<InputGroup>
												<InputGroupInput autoComplete="off" disabled={saving} id="dingtalk-secret" onChange={(event) => setDraft({ ...draft, notifications: { ...draft.notifications, dingtalk: { ...draft.notifications.dingtalk, secret: event.target.value } } })} placeholder="SEC…" spellCheck={false} type={secretVisible ? "text" : "password"} value={draft.notifications.dingtalk.secret} />
												<InputGroupAddon align="inline-end"><InputGroupButton aria-label={secretVisible ? "隐藏钉钉密钥" : "显示钉钉密钥"} onClick={() => setSecretVisible((visible) => !visible)} size="icon-xs">{secretVisible ? <EyeOffIcon /> : <EyeIcon />}</InputGroupButton></InputGroupAddon>
											</InputGroup>
										</Field>
									</FieldGroup>
								</CardContent>
							</Card>
						</div>
					</section>
				</CardContent>
			</Card>
		</form>

		<ModelPricingCard
			error={pricing.error}
			loading={pricing.loading}
			onSave={pricing.onSave}
			onSync={pricing.onSync}
			prices={pricing.prices}
			saving={pricing.saving}
			syncing={pricing.syncing}
			usedModels={pricing.usedModels}
		/>
	</div>
	);
}

function NotificationEvent({
	checked,
	disabled,
	icon,
	label,
	onChange,
}: {
	checked: boolean;
	disabled: boolean;
	icon: ReactNode;
	label: string;
	onChange: (checked: boolean) => void;
}) {
	return (
		<Field className="settings-event-card" orientation="horizontal">
			<div className="settings-event-icon" aria-hidden="true">{icon}</div>
			<FieldTitle>{label}</FieldTitle>
			<Switch aria-label={`启用${label}通知`} checked={checked} disabled={disabled} onCheckedChange={onChange} />
		</Field>
	);
}

function validateSettings(settings: AdminSettings): string | null {
	if (!isHttpsUrl(settings.notifications.resetWatchApiUrl)) {
		return "重置预测 API 地址必须是有效的 HTTPS 链接。";
	}
	if (settings.notifications.bark.enabled && !isHttpsUrl(settings.notifications.bark.pushUrl)) {
		return "启用 Bark 前，请填写有效的 HTTPS 推送链接。";
	}
	const dingtalk = settings.notifications.dingtalk;
	if (dingtalk.enabled && (!isHttpsUrl(dingtalk.webhookUrl) || !dingtalk.secret.trim())) {
		return "启用钉钉前，请填写有效的 Webhook 和加签密钥。";
	}
	if (
		(Boolean(dingtalk.webhookUrl.trim()) !== Boolean(dingtalk.secret.trim()))
		|| (dingtalk.webhookUrl.trim() && !isHttpsUrl(dingtalk.webhookUrl))
	) {
		return "钉钉 Webhook 与加签密钥需要同时填写。";
	}
	if (settings.notifications.bark.pushUrl.trim() && !isHttpsUrl(settings.notifications.bark.pushUrl)) {
		return "Bark 推送链接必须使用 HTTPS。";
	}
	return null;
}

function normalizeSettings(settings: AdminSettings): AdminSettings {
	return {
		...settings,
		notifications: {
			...settings.notifications,
			accountIds: settings.notifications.allAccounts ? [] : settings.notifications.accountIds,
			resetWatchApiUrl: settings.notifications.resetWatchApiUrl.trim(),
			bark: {
				...settings.notifications.bark,
				pushUrl: settings.notifications.bark.pushUrl.trim(),
			},
			dingtalk: {
				...settings.notifications.dingtalk,
				webhookUrl: settings.notifications.dingtalk.webhookUrl.trim(),
				secret: settings.notifications.dingtalk.secret.trim(),
			},
		},
	};
}

function isHttpsUrl(value: string): boolean {
	try {
		const url = new URL(value.trim());
		return url.protocol === "https:" && Boolean(url.hostname);
	} catch {
		return false;
	}
}

function cloneSettings(settings: AdminSettings): AdminSettings {
	return {
		publicAccount: { ...settings.publicAccount },
		notifications: {
			...settings.notifications,
			accountIds: [...settings.notifications.accountIds],
			bark: { ...settings.notifications.bark },
			dingtalk: { ...settings.notifications.dingtalk },
		},
	};
}
