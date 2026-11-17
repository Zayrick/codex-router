import { useEffect, useState, type ReactNode } from "react";
import {
	ArrowRightIcon,
	BellRingIcon,
	CheckCircle2Icon,
	CircleAlertIcon,
	CoinsIcon,
	EyeIcon,
	EyeOffIcon,
	GaugeIcon,
	Globe2Icon,
	InfoIcon,
	MessageSquareMoreIcon,
	PencilIcon,
	RadioTowerIcon,
	SaveIcon,
	SmartphoneIcon,
	UsersIcon,
	ZapIcon,
} from "lucide-react";

import type { NotificationChannel } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { useAnalytics } from "@/admin/hooks";
import { StatusDot, Tag } from "@/components/app/badges";
import { planLabel } from "@/components/app/viz";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Field, FieldContent, FieldDescription, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput, InputGroupText } from "@/components/ui/input-group";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { formatCost, formatCount } from "@/lib/format";
import { cn } from "@/lib/utils";

import {
	isHttpsUrl,
	parseMultiplier,
	sectionLabel,
	settingsChanges,
	toDraft,
	toSettings,
	validMultiplier,
	validationError,
	type SettingsDraft,
	type SettingsSection,
} from "./settings-model";

const SECTIONS: { id: SettingsSection; label: string; icon: ReactNode }[] = [
	{ id: "public", label: "用户端", icon: <Globe2Icon /> },
	{ id: "billing", label: "计费", icon: <CoinsIcon /> },
	{ id: "events", label: "通知事件", icon: <BellRingIcon /> },
	{ id: "recipients", label: "接收账户", icon: <UsersIcon /> },
	{ id: "channels", label: "通知通道", icon: <SmartphoneIcon /> },
];

const sectionId = (section: SettingsSection) => `settings-${section}`;

export function SettingsView() {
	const { settings, saveSettings, notify, state } = useAdmin();
	const saved = toDraft(settings);
	const [draft, setDraft] = useState<SettingsDraft>(saved);
	const [saving, setSaving] = useState(false);
	const [active, setActive] = useState<SettingsSection>("public");
	const changes = settingsChanges(saved, draft, state.codexAccounts);
	const dirtySections = new Set(changes.map((change) => change.section));

	// Adopt saved settings (after a save or a deleted account) when nothing is pending.
	const savedKey = JSON.stringify(saved);
	useEffect(() => {
		setDraft(JSON.parse(savedKey) as SettingsDraft);
		// eslint-disable-next-line react-hooks/exhaustive-deps -- reset only when the saved value changes.
	}, [savedKey]);

	useEffect(() => {
		const root = document.querySelector("[data-console-scroll] [data-slot=scroll-area-viewport]");
		const observer = new IntersectionObserver(
			(entries) => {
				const visible = entries.filter((entry) => entry.isIntersecting).sort((left, right) => left.boundingClientRect.top - right.boundingClientRect.top);
				const first = visible[0]?.target.id.replace("settings-", "");
				if (first) setActive(first as SettingsSection);
			},
			{ root, rootMargin: "-80px 0px -55% 0px" },
		);
		for (const section of SECTIONS) {
			const element = document.getElementById(sectionId(section.id));
			if (element) observer.observe(element);
		}
		return () => observer.disconnect();
	}, []);

	function patch(value: Partial<SettingsDraft>) {
		setDraft((current) => ({ ...current, ...value }));
	}

	async function save() {
		const error = validationError(draft);
		if (error) {
			notify(error, "error");
			return;
		}
		setSaving(true);
		await saveSettings(toSettings(draft));
		setSaving(false);
	}

	function jump(section: SettingsSection) {
		setActive(section);
		document.getElementById(sectionId(section))?.scrollIntoView({ behavior: "smooth", block: "start" });
	}

	const enabledEvents = [draft.resetWatchEnabled, draft.quotaResetEnabled, draft.usageWarningEnabled].filter(Boolean).length;
	const selectedAccounts = draft.allAccounts ? state.codexAccounts.length : draft.accountIds.filter((id) => state.codexAccounts.some((account) => account.id === id)).length;
	const enabledChannels = [draft.barkEnabled, draft.dingtalkEnabled].filter(Boolean).length;
	const navAside: Record<SettingsSection, string> = {
		public: draft.showQuota ? "显示额度" : "隐藏额度",
		billing: `${draft.fastCostMultiplier || "—"}×`,
		events: `${enabledEvents} / 3`,
		recipients: draft.allAccounts ? "全部" : `${selectedAccounts} / ${state.codexAccounts.length}`,
		channels: `${enabledChannels} / 2`,
	};

	return (
		<div className="grid items-start gap-4 lg:grid-cols-[270px_minmax(0,1fr)]">
			<div className="flex flex-col gap-4 lg:sticky lg:top-4">
				<Card className="gap-0.5 p-2">
					<nav aria-label="设置分区" className="flex flex-col gap-0.5">
						{SECTIONS.map((section) => (
							<Button
								aria-current={active === section.id ? "true" : undefined}
								className={cn(
									"h-9 justify-start gap-2 px-3 text-[13px] font-medium text-muted-foreground [&_svg]:size-4",
									active === section.id && "bg-surface-hover font-bold text-foreground",
								)}
								key={section.id}
								onClick={() => jump(section.id)}
								type="button"
								variant="ghost"
							>
								{section.icon}
								<span className="flex-1 text-left">{section.label}</span>
								{dirtySections.has(section.id) ? <span aria-label="有未保存的修改" className="size-1.5 rounded-full bg-[#f59e0b]" /> : null}
								<span className="text-[11.5px] font-normal text-faint">{navAside[section.id]}</span>
							</Button>
						))}
					</nav>
				</Card>
				<div className="flex flex-col gap-2 rounded-xl bg-surface-subtle p-4">
					<span className="flex items-center gap-1.5 text-[12.5px] font-bold">
						<InfoIcon className="size-3.5" />
						说明
					</span>
					<p className="text-xs leading-4 text-muted-foreground">修改需点击「保存设置」后生效。Fast 计费倍率对历史用量同样生效。</p>
				</div>
			</div>

			<div className="flex min-w-0 flex-col gap-4">
				{changes.length ? (
					<div className="sticky top-0 z-10 flex flex-wrap items-center gap-3 rounded-xl bg-primary px-4 py-2 text-primary-foreground shadow-md">
						<PencilIcon aria-hidden="true" className="size-3.5" />
						<div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 text-[12.5px] leading-5">
							<span className="font-bold">{changes.length} 项未保存的修改</span>
							<span className="truncate opacity-75">
								{changes
									.slice(0, 2)
									.map((change) => `${sectionLabel(change.section)} · ${change.text}`)
									.join("；")}
								{changes.length > 2 ? " 等" : ""}
							</span>
						</div>
						<Button
							className="text-primary-foreground hover:bg-primary-foreground/10 hover:text-primary-foreground"
							disabled={saving}
							onClick={() => setDraft(saved)}
							size="sm"
							type="button"
							variant="ghost"
						>
							放弃修改
						</Button>
						<Button disabled={saving} onClick={() => void save()} size="sm" type="button" variant="secondary">
							{saving ? <Spinner data-icon="inline-start" /> : <SaveIcon data-icon="inline-start" />}
							保存设置
						</Button>
					</div>
				) : null}

				<Section description="控制下游用户在用户端可见的信息。" icon={<Globe2Icon />} id="public" title="用户端">
					<SwitchRow
						checked={draft.showQuota}
						description="显示分组内所有 Codex 账户的额度时间轴。"
						icon={<GaugeIcon />}
						id="settings-show-quota"
						label="显示账户额度"
						onChange={(showQuota) => patch({ showQuota })}
					/>
				</Section>

				<BillingSection draft={draft} onChange={(fastCostMultiplier) => patch({ fastCostMultiplier })} saved={saved.fastCostMultiplier} />

				<Section icon={<BellRingIcon />} id="events" title="通知事件">
					<div className="grid gap-4 sm:grid-cols-3">
						<EventCard
							checked={draft.resetWatchEnabled}
							description="轮询重置预测 API，额度即将提前重置时提醒。"
							icon={<RadioTowerIcon />}
							label="重置预测"
							onChange={(resetWatchEnabled) => patch({ resetWatchEnabled })}
						/>
						<EventCard
							checked={draft.quotaResetEnabled}
							description="额度窗口重置后通知。"
							icon={<CheckCircle2Icon />}
							label="额度重置"
							onChange={(quotaResetEnabled) => patch({ quotaResetEnabled })}
						/>
						<EventCard
							checked={draft.usageWarningEnabled}
							description="预计在重置前耗尽时通知。"
							icon={<GaugeIcon />}
							label="用量过高"
							onChange={(usageWarningEnabled) => patch({ usageWarningEnabled })}
						/>
					</div>
					<Field>
						<FieldLabel className="text-[12.5px]" htmlFor="settings-reset-watch-url">
							重置预测 API 地址
						</FieldLabel>
						<Input
							aria-invalid={!isHttpsUrl(draft.resetWatchApiUrl) || undefined}
							id="settings-reset-watch-url"
							onChange={(event) => patch({ resetWatchApiUrl: event.target.value })}
							placeholder="https://"
							spellCheck={false}
							type="url"
							value={draft.resetWatchApiUrl}
						/>
						<UrlHint valid={isHttpsUrl(draft.resetWatchApiUrl)} validText="HTTPS 地址有效 · 仅「重置预测」事件使用" />
					</Field>
				</Section>

				<Section
					aside={<Tag>已选 {draft.allAccounts ? "全部" : `${selectedAccounts} / ${state.codexAccounts.length}`}</Tag>}
					description="「额度重置」「用量过高」仅对选中账户发送。"
					icon={<UsersIcon />}
					id="recipients"
					title="接收账户"
				>
					<SwitchRow
						checked={draft.allAccounts}
						description="新账户自动纳入。关闭后可逐个选择。"
						icon={<UsersIcon />}
						id="settings-all-accounts"
						label="所有账户"
						onChange={(allAccounts) => patch({ allAccounts })}
					/>
					{state.codexAccounts.length === 0 ? (
						<p className="text-xs text-muted-foreground">还没有 Codex 账户。</p>
					) : (
						<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
							{state.codexAccounts.map((account) => {
								const checked = draft.allAccounts || draft.accountIds.includes(account.id);
								return (
									<label
										className={cn(
											"flex cursor-pointer items-center gap-3 rounded-lg border p-3 transition-colors has-disabled:cursor-not-allowed",
											checked && !draft.allAccounts && "border-primary-border bg-surface-subtle",
											!account.enabled && "opacity-70",
										)}
										key={account.id}
									>
										<Checkbox
											checked={checked}
											disabled={draft.allAccounts}
											onCheckedChange={(value) =>
												patch({
													accountIds: value === true ? [...draft.accountIds, account.id] : draft.accountIds.filter((id) => id !== account.id),
												})
											}
										/>
										<span className="flex min-w-0 flex-1 flex-col gap-0.5">
											<span className={cn("truncate text-[13px] leading-5 font-medium", !account.enabled && "text-faint")}>
												{account.name}
												{account.enabled ? "" : "（停用）"}
											</span>
											{account.oauth?.email ? <span className="truncate font-mono text-[11px] leading-4 text-faint">{account.oauth.email}</span> : null}
										</span>
										{account.subscription?.planType ? <Tag className="bg-primary-soft">{planLabel(account.subscription.planType)}</Tag> : null}
									</label>
								);
							})}
						</div>
					)}
				</Section>

				<Section description="启用的通道会同时收到所有已开启事件。" icon={<SmartphoneIcon />} id="channels" title="通知通道">
					<div className="grid gap-4 md:grid-cols-2">
						<ChannelCard
							channel="bark"
							dirty={dirtySections.has("channels")}
							enabled={draft.barkEnabled}
							icon={<SmartphoneIcon />}
							onEnabledChange={(barkEnabled) => patch({ barkEnabled })}
							savedEnabled={saved.barkEnabled && Boolean(saved.barkPushUrl)}
							title="Bark"
						>
							<SecretField
								id="settings-bark-url"
								label="推送链接"
								onChange={(barkPushUrl) => patch({ barkPushUrl })}
								placeholder="https://api.day.app/…"
								value={draft.barkPushUrl}
							/>
						</ChannelCard>
						<ChannelCard
							channel="dingtalk"
							dirty={dirtySections.has("channels")}
							enabled={draft.dingtalkEnabled}
							icon={<MessageSquareMoreIcon />}
							onEnabledChange={(dingtalkEnabled) => patch({ dingtalkEnabled })}
							savedEnabled={saved.dingtalkEnabled && Boolean(saved.dingtalkWebhookUrl && saved.dingtalkSecret)}
							title="钉钉机器人"
						>
							<SecretField
								id="settings-dingtalk-webhook"
								label="Webhook"
								onChange={(dingtalkWebhookUrl) => patch({ dingtalkWebhookUrl })}
								placeholder="https://oapi.dingtalk.com/robot/send?access_token=…"
								value={draft.dingtalkWebhookUrl}
							/>
							<SecretField
								id="settings-dingtalk-secret"
								label="加签密钥"
								onChange={(dingtalkSecret) => patch({ dingtalkSecret })}
								placeholder="SEC…"
								value={draft.dingtalkSecret}
							/>
						</ChannelCard>
					</div>
				</Section>
			</div>
		</div>
	);
}

function Section({
	id,
	icon,
	title,
	description,
	aside,
	children,
}: {
	id: SettingsSection;
	icon: ReactNode;
	title: string;
	description?: string;
	aside?: ReactNode;
	children: ReactNode;
}) {
	return (
		<Card className="scroll-mt-16 gap-4 p-6" id={sectionId(id)}>
			<div className="flex items-center gap-3">
				<div className="shrink-0 rounded-lg bg-surface-subtle p-2 [&_svg]:size-4">{icon}</div>
				<div className="flex min-w-0 flex-1 flex-col gap-1">
					<h2 className="text-[15px] leading-5 font-bold tracking-[-0.01em]">{title}</h2>
					{description ? <p className="text-[12.5px] leading-5 text-muted-foreground">{description}</p> : null}
				</div>
				{aside}
			</div>
			{children}
		</Card>
	);
}

/** Figma Settings/Switch Row. */
function SwitchRow({
	id,
	icon,
	label,
	description,
	checked,
	onChange,
}: {
	id: string;
	icon: ReactNode;
	label: string;
	description: string;
	checked: boolean;
	onChange: (value: boolean) => void;
}) {
	return (
		<Field className="rounded-lg border px-4 py-3" orientation="horizontal">
			<FieldContent>
				<FieldLabel className="gap-1.5 text-[13px] [&_svg]:size-3.5" htmlFor={id}>
					{icon}
					{label}
				</FieldLabel>
				<FieldDescription className="text-xs">{description}</FieldDescription>
			</FieldContent>
			<Switch checked={checked} id={id} onCheckedChange={onChange} />
		</Field>
	);
}

function BillingSection({ draft, saved, onChange }: { draft: SettingsDraft; saved: string; onChange: (value: string) => void }) {
	const { navigate } = useAdmin();
	const analytics = useAnalytics({ range: "7d" });
	const current = parseMultiplier(saved);
	const next = parseMultiplier(draft.fastCostMultiplier);
	const changed = current !== next;
	const valid = validMultiplier(draft.fastCostMultiplier);
	const base = analytics.data?.totals.fastBaseCostUsd ?? null;
	const currentSurcharge = base === null ? null : base * (current - 1);
	const nextSurcharge = base === null || !valid ? null : base * (next - 1);
	return (
		<Section description="模型单价在「模型价格」页维护。" icon={<CoinsIcon />} id="billing" title="计费">
			<div className={cn("flex flex-wrap items-center gap-4 rounded-lg border px-4 py-3", changed && "border-warning-border bg-warning-soft/50")}>
				<div className="flex min-w-60 flex-1 flex-col gap-1">
					<label className="flex items-center gap-1.5 text-[13px] leading-5 font-medium" htmlFor="settings-fast-multiplier">
						<ZapIcon className="size-3.5" />
						Fast 计费倍率
					</label>
					<p className="text-xs leading-4 text-muted-foreground">Fast 请求按单价 × 倍率计费，对历史用量同样生效。范围 0–100。</p>
				</div>
				<div className="flex flex-col items-end gap-1">
					<InputGroup className={cn("h-8 w-28", changed && "border-[1.5px] border-warning", !valid && "border-destructive")}>
						<InputGroupInput
							aria-invalid={!valid || undefined}
							className="font-mono text-[13px]"
							id="settings-fast-multiplier"
							inputMode="decimal"
							onChange={(event) => onChange(event.target.value)}
							value={draft.fastCostMultiplier}
						/>
						<InputGroupAddon align="inline-end">
							<InputGroupText>×</InputGroupText>
						</InputGroupAddon>
					</InputGroup>
					{changed ? <span className="text-[11px] leading-4 text-warning">已修改 · 原 {saved}×</span> : null}
				</div>
			</div>
			<div className="flex flex-wrap items-center gap-6 rounded-lg bg-surface-subtle px-4 py-3">
				<Impact label="近 7 天 Fast 请求" value={analytics.data ? formatCount(analytics.data.totals.fastRequests) : "—"} />
				<Impact label={`当前倍率加成 (${saved}×)`} value={formatCost(currentSurcharge)} />
				{changed ? (
					<>
						<Impact label={`保存后加成 (${draft.fastCostMultiplier || "—"}×)`} tone="warning" value={formatCost(nextSurcharge)} />
						<Impact
							label="7 天成本变化"
							tone="warning"
							value={nextSurcharge === null || currentSurcharge === null ? "—" : formatCost(nextSurcharge - currentSurcharge, { signed: true })}
						/>
					</>
				) : null}
				<span className="flex-1" />
				<Button className="h-auto p-0 text-xs" onClick={() => navigate("pricing")} type="button" variant="link">
					管理模型价格
					<ArrowRightIcon data-icon="inline-end" />
				</Button>
			</div>
		</Section>
	);
}

function Impact({ label, value, tone }: { label: string; value: string; tone?: "warning" }) {
	return (
		<div className="flex flex-col gap-0.5">
			<span className={cn("text-[15px] leading-5 font-bold tabular-nums", tone === "warning" && "text-warning")}>{value}</span>
			<span className="text-[11px] leading-4 text-faint">{label}</span>
		</div>
	);
}

function EventCard({
	icon,
	label,
	description,
	checked,
	onChange,
}: {
	icon: ReactNode;
	label: string;
	description: string;
	checked: boolean;
	onChange: (value: boolean) => void;
}) {
	return (
		<Card className="gap-3 rounded-lg p-4 shadow-none ring-border">
			<div className="flex items-center justify-between">
				<div className="rounded-lg bg-primary-soft p-2 [&_svg]:size-4">{icon}</div>
				<Switch aria-label={label} checked={checked} onCheckedChange={onChange} />
			</div>
			<div className="flex flex-col gap-1">
				<span className="text-[13px] leading-5 font-bold">{label}</span>
				<span className="text-xs leading-4 text-muted-foreground">{description}</span>
			</div>
		</Card>
	);
}

function UrlHint({ valid, validText }: { valid: boolean; validText: string }) {
	return valid ? (
		<span className="flex items-center gap-1 text-[11.5px] leading-4 text-success">
			<CheckCircle2Icon className="size-3" />
			{validText}
		</span>
	) : (
		<span className="flex items-center gap-1 text-[11.5px] leading-4 text-danger">
			<CircleAlertIcon className="size-3" />
			需要有效的 HTTPS 地址
		</span>
	);
}

function ChannelCard({
	channel,
	icon,
	title,
	enabled,
	savedEnabled,
	dirty,
	onEnabledChange,
	children,
}: {
	channel: NotificationChannel;
	icon: ReactNode;
	title: string;
	enabled: boolean;
	savedEnabled: boolean;
	dirty: boolean;
	onEnabledChange: (value: boolean) => void;
	children: ReactNode;
}) {
	const { testNotification } = useAdmin();
	const [testing, setTesting] = useState(false);
	const reason = dirty ? "保存修改后再测试" : !savedEnabled ? "启用并保存后可测试" : null;
	async function test() {
		setTesting(true);
		await testNotification(channel);
		setTesting(false);
	}
	return (
		<Card className="gap-4 rounded-lg p-4 shadow-none ring-border">
			<div className="flex items-center gap-2">
				<span className="[&_svg]:size-4">{icon}</span>
				<span className="text-[13.5px] leading-5 font-semibold">{title}</span>
				<Tag tone={enabled ? "success" : "neutral"}>
					<StatusDot tone={enabled ? "success" : "neutral"}>{enabled ? "已启用" : "未启用"}</StatusDot>
				</Tag>
				<span className="flex-1" />
				<Switch aria-label={`启用 ${title}`} checked={enabled} onCheckedChange={onEnabledChange} />
			</div>
			{children}
			<div className="mt-auto flex items-center gap-2 border-t pt-3">
				<Tooltip>
					<TooltipTrigger asChild>
						<span tabIndex={reason ? 0 : -1}>
							<Button disabled={Boolean(reason) || testing} onClick={() => void test()} size="sm" type="button" variant="outline">
								{testing ? <Spinner data-icon="inline-start" /> : <BellRingIcon data-icon="inline-start" />}
								测试通知
							</Button>
						</span>
					</TooltipTrigger>
					{reason ? <TooltipContent>{reason}</TooltipContent> : null}
				</Tooltip>
			</div>
		</Card>
	);
}

function SecretField({ id, label, value, placeholder, onChange }: { id: string; label: string; value: string; placeholder: string; onChange: (value: string) => void }) {
	const [visible, setVisible] = useState(false);
	return (
		<Field>
			<FieldLabel className="text-[12.5px]" htmlFor={id}>
				{label}
			</FieldLabel>
			<InputGroup>
				<InputGroupInput
					autoComplete="off"
					id={id}
					onChange={(event) => onChange(event.target.value)}
					placeholder={placeholder}
					spellCheck={false}
					type={visible ? "text" : "password"}
					value={value}
				/>
				<InputGroupAddon align="inline-end">
					<InputGroupButton aria-label={visible ? "隐藏" : "显示"} onClick={() => setVisible((current) => !current)} size="icon-xs">
						{visible ? <EyeOffIcon /> : <EyeIcon />}
					</InputGroupButton>
				</InputGroupAddon>
			</InputGroup>
		</Field>
	);
}
