import { useState, type FormEvent } from "react";
import {
	CopyIcon,
	ExternalLinkIcon,
	InfoIcon,
	RefreshCwIcon,
	TriangleAlertIcon,
	UsersIcon,
} from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardAction,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import {
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import {
	Empty,
	EmptyDescription,
	EmptyHeader,
	EmptyMedia,
	EmptyTitle,
} from "@/components/ui/empty";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import type {
	CodexAccount,
	CodexAccountDeviceAuthorization,
	CodexAccountUpdate,
	SubscriptionInfo,
	SubscriptionMetadata,
} from "./admin-api";
import DeleteConfirmationDialog from "./DeleteConfirmationDialog";
import QuotaTimeline from "./QuotaTimeline";

interface CodexAccountsProps {
	accounts: CodexAccount[];
	busyAccounts: ReadonlySet<string>;
	loginFlow: CodexAccountDeviceAuthorization | null;
	loginLoading: boolean;
	loginError: string | null;
	now: number;
	subscriptionErrors: Readonly<Record<string, string>>;
	subscriptionLoading: ReadonlySet<string>;
	subscriptions: Readonly<Record<string, SubscriptionInfo>>;
	onCancelLogin: () => void;
	onDelete: (account: CodexAccount) => void;
	onRefresh: (account: CodexAccount) => void;
	onStartLogin: () => void;
	onUpdate: (account: CodexAccount, value: CodexAccountUpdate) => void;
}

export default function CodexAccounts({
	accounts,
	busyAccounts,
	loginFlow,
	loginLoading,
	loginError,
	now,
	subscriptionErrors,
	subscriptionLoading,
	subscriptions,
	onCancelLogin,
	onDelete,
	onRefresh,
	onStartLogin,
	onUpdate,
}: CodexAccountsProps) {
	const [editing, setEditing] = useState<CodexAccount | null>(null);

	return (
		<>
			{accounts.length === 0 ? (
				<Empty className="unified-empty-state border" aria-label="Codex 账户">
					<EmptyHeader>
						<EmptyMedia variant="icon"><UsersIcon /></EmptyMedia>
						<EmptyTitle>暂无 Codex 账户</EmptyTitle>
						<EmptyDescription>添加账户后即可查看订阅额度并参与请求调度。</EmptyDescription>
					</EmptyHeader>
				</Empty>
			) : (
				<section className="codex-account-list" aria-label="Codex 账户">
					{accounts.map((account) => (
						<CodexAccountCard
							account={account}
							busy={busyAccounts.has(account.id)}
							error={subscriptionErrors[account.id] ?? null}
							key={account.id}
							loading={subscriptionLoading.has(account.id)}
							now={now}
							onDelete={() => onDelete(account)}
							onEdit={() => setEditing(account)}
							onRefresh={() => onRefresh(account)}
							onToggle={() => onUpdate(account, { name: account.name, enabled: !account.enabled })}
							subscription={subscriptions[account.id] ?? null}
						/>
					))}
				</section>
			)}

			{editing ? (
				<AccountNameDialog
					account={editing}
					busy={busyAccounts.has(editing.id)}
					onCancel={() => setEditing(null)}
					onSave={(name) => {
						onUpdate(editing, { name, enabled: editing.enabled });
						setEditing(null);
					}}
				/>
			) : null}

			{loginFlow || loginLoading || loginError ? (
				<DeviceLoginDialog
					error={loginError}
					flow={loginFlow}
					loading={loginLoading}
					onCancel={onCancelLogin}
					onRetry={onStartLogin}
				/>
			) : null}
		</>
	);
}

function CodexAccountCard({
	account,
	busy,
	error,
	loading,
	now,
	onDelete,
	onEdit,
	onRefresh,
	onToggle,
	subscription,
}: {
	account: CodexAccount;
	busy: boolean;
	error: string | null;
	loading: boolean;
	now: number;
	onDelete: () => void;
	onEdit: () => void;
	onRefresh: () => void;
	onToggle: () => void;
	subscription: SubscriptionInfo | null;
}) {
	const metadata = subscription ?? account.subscription;
	const plan = metadata?.planType;
	const email = account.oauth?.email;

	return (
		<Card className={`account-card codex-account-card gap-0${account.enabled ? "" : " is-disabled"}`} size="sm">
				<CardHeader className="codex-account-card-header border-b max-[46rem]:grid-cols-1">
					<CardTitle className="truncate" title={email ?? account.name}>{email || account.name}</CardTitle>
					<CardDescription className="codex-account-subtitle">
						{email ? <small>{account.name}</small> : null}
						{email ? <i aria-hidden="true">·</i> : null}
						<small>{formatPlan(plan)}</small>
						<AccountDetailsInfo account={account} metadata={metadata} now={now} subscription={subscription} />
					</CardDescription>
					<CardAction className="account-header-actions codex-account-header-actions max-[46rem]:col-start-1 max-[46rem]:row-auto max-[46rem]:mt-2 max-[46rem]:justify-self-stretch">
						<Switch
							aria-label={`${account.enabled ? "禁用" : "启用"}${account.name}`}
							checked={account.enabled}
							disabled={busy}
							onCheckedChange={() => onToggle()}
							title={account.enabled ? "禁用账户" : "启用账户"}
						/>
						<Button aria-label="刷新额度" disabled={loading || busy || !account.enabled} onClick={onRefresh} size="icon-sm" title="刷新额度" type="button" variant="outline">
							<RefreshCwIcon className={loading ? "animate-spin" : undefined} />
						</Button>
						<Button disabled={busy} onClick={onEdit} size="sm" type="button" variant="outline">重命名</Button>
						<DeleteConfirmationDialog
							description="相关路由也会被移除。此操作无法撤销。"
							onConfirm={onDelete}
							title={`删除 Codex 账户“${email || account.name}”？`}
							trigger={<Button disabled={busy} size="sm" type="button" variant="destructive">删除</Button>}
						/>
					</CardAction>
				</CardHeader>

			<CardContent className="account-quota-section grid gap-0 p-0" aria-label={`${email || account.name} 的账户配额`}>
				{loading && !subscription ? (
					<div className="center-state account-quota-loading" role="status">
						<Spinner />
						<span>正在读取配额时间轴…</span>
					</div>
				) : null}
				{error ? (
					<Alert className="rounded-none border-x-0 border-t-0" variant="destructive">
						<TriangleAlertIcon />
						<AlertDescription>{error}</AlertDescription>
					</Alert>
				) : null}
				{subscription?.windows.length ? (
					<QuotaTimeline
						className={loading ? "account-quota-timeline is-refreshing rounded-none ring-0" : "account-quota-timeline rounded-none ring-0"}
						now={now}
						planType={plan}
						sampledAt={subscription.fetchedAt}
						windows={subscription.windows}
					/>
				) : !loading && !error ? (
					<Empty className="account-quota-empty rounded-none">
						<EmptyHeader><EmptyDescription>暂无额度数据</EmptyDescription></EmptyHeader>
					</Empty>
				) : null}
			</CardContent>
		</Card>
	);
}

function AccountDetailsInfo({
	account,
	metadata,
	now,
	subscription,
}: {
	account: CodexAccount;
	metadata: SubscriptionInfo | SubscriptionMetadata | null;
	now: number;
	subscription: SubscriptionInfo | null;
}) {
	const credits = subscription?.rateLimitResetCredits;
	const availableCredits = credits?.availableCount ?? null;
	const applicableCredits = credits?.applicableAvailableCount ?? null;
	const resetCredits = availableCredits === null
		? "暂无数据"
		: applicableCredits === null
			? String(Math.max(0, availableCredits))
			: `${Math.max(0, availableCredits)} · 可用 ${Math.max(0, applicableCredits)}`;

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button aria-label="查看账户详情" size="icon-xs" type="button" variant="ghost">
					<InfoIcon />
				</Button>
			</TooltipTrigger>
			<TooltipContent align="start" className="grid w-[min(19rem,calc(100vw-5rem))] gap-2" side="bottom" sideOffset={6}>
				<strong>账户详情</strong>
				<AccountDetailRow label="Account ID" value={account.oauth?.accountId ?? "未知"} />
				<AccountDetailRow label="开始时间" value={formatTimestamp(metadata?.subscriptionActiveStart)} />
				<AccountDetailRow
					danger={isExpired(metadata?.subscriptionActiveUntil, now)}
					label="到期时间"
					value={formatTimestamp(metadata?.subscriptionActiveUntil)}
				/>
				<AccountDetailRow
					danger={isExpired(account.oauth?.expiresAt, now)}
					label="Token 到期时间"
					value={formatTimestamp(account.oauth?.expiresAt, "未知")}
				/>
				<AccountDetailRow label="重置积分" value={resetCredits} />
				<AccountDetailRow label="用量更新时间" value={formatTimestamp(subscription?.fetchedAt)} />
			</TooltipContent>
		</Tooltip>
	);
}

function AccountDetailRow({ danger = false, label, value }: { danger?: boolean; label: string; value: string }) {
	return (
		<span className="grid grid-cols-[5.25rem_minmax(0,1fr)] items-start gap-3 leading-relaxed">
			<span className="opacity-70">{label}</span>
			<b className={danger ? "text-destructive text-right" : "text-right"}>{value}</b>
		</span>
	);
}

function AccountNameDialog({
	account,
	busy,
	onCancel,
	onSave,
}: {
	account: CodexAccount;
	busy: boolean;
	onCancel: () => void;
	onSave: (name: string) => void;
}) {
	const [name, setName] = useState(account.name);

	function submit(event: FormEvent<HTMLFormElement>): void {
		event.preventDefault();
		const value = name.trim();
		if (value) onSave(value);
	}

	return (
		<Dialog open onOpenChange={(open) => { if (!open && !busy) onCancel(); }}>
			<DialogContent showCloseButton={!busy}>
				<DialogHeader>
					<DialogTitle>重命名账户</DialogTitle>
					<DialogDescription>修改该 Codex 账户在管理面板中的显示名称。</DialogDescription>
				</DialogHeader>
				<form className="grid gap-4" onSubmit={submit}>
					<FieldGroup>
						<Field>
							<FieldLabel htmlFor="codex-account-name">显示名称</FieldLabel>
							<Input autoFocus disabled={busy} id="codex-account-name" maxLength={100} onChange={(event) => setName(event.target.value)} required type="text" value={name} />
						</Field>
					</FieldGroup>
					<DialogFooter>
						<DialogClose asChild><Button disabled={busy} type="button" variant="outline">取消</Button></DialogClose>
						<Button disabled={busy || !name.trim()} type="submit">保存</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}

function DeviceLoginDialog({
	error,
	flow,
	loading,
	onCancel,
	onRetry,
}: {
	error: string | null;
	flow: CodexAccountDeviceAuthorization | null;
	loading: boolean;
	onCancel: () => void;
	onRetry: () => void;
}) {
	const [copiedCode, setCopiedCode] = useState<string | null>(null);
	const copied = Boolean(flow && copiedCode === flow.authorization.userCode);

	async function copyCode(): Promise<void> {
		if (!flow) return;
		await navigator.clipboard.writeText(flow.authorization.userCode);
		setCopiedCode(flow.authorization.userCode);
	}

	return (
		<Dialog open onOpenChange={(open) => { if (!open) onCancel(); }}>
			<DialogContent className="sm:max-w-lg">
				<DialogHeader>
					<DialogTitle>登录新账户</DialogTitle>
					<DialogDescription>打开 OpenAI 登录页并输入设备码以完成授权。</DialogDescription>
				</DialogHeader>
				{loading && !flow ? <div className="center-state"><Spinner /><span>获取登录码…</span></div> : null}
				{flow ? (
					<div className="device-login-content">
						<Button className="device-code-button h-auto w-full border-dashed" onClick={() => void copyCode()} type="button" variant="outline">
							<code>{flow.authorization.userCode}</code>
							<span>{copied ? "已复制" : "点击复制"}</span>
							<CopyIcon className="sr-only" />
						</Button>
						<Button asChild><a href={flow.authorization.verificationUri} rel="noreferrer" target="_blank">打开登录页面 <ExternalLinkIcon data-icon="inline-end" /></a></Button>
						<small>等待授权…</small>
					</div>
				) : null}
				{error ? <Alert variant="destructive"><TriangleAlertIcon /><AlertDescription>{error}</AlertDescription></Alert> : null}
				{error ? <DialogFooter><DialogClose asChild><Button type="button" variant="outline">取消</Button></DialogClose><Button disabled={loading} onClick={onRetry} type="button">重新获取</Button></DialogFooter> : null}
			</DialogContent>
		</Dialog>
	);
}

function formatPlan(value: string | null | undefined): string {
	if (!value) return "未知套餐";
	return value.replace(/[_-]+/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function isExpired(value: number | null | undefined, now: number): boolean {
	return typeof value === "number" && Number.isFinite(value) && value > 0 && value <= now;
}

function formatTimestamp(value: number | null | undefined, fallback = "暂无数据"): string {
	if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return fallback;
	return new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(new Date(value));
}
