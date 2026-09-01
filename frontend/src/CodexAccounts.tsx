import { useId, useState, type FormEvent } from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
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
				<section className="card empty-state unified-empty-state codex-accounts-empty" aria-label="Codex 账户">
					<strong>暂无 Codex 账户</strong>
				</section>
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
		<article className={`card account-card codex-account-card${account.enabled ? "" : " is-disabled"}`}>
			<div className="account-summary">
				<header className="card-header codex-account-card-header">
					<div className="codex-account-identity">
						<strong title={email ?? account.name}>{email || account.name}</strong>
						<span className="codex-account-subtitle">
							{email ? <small>{account.name}</small> : null}
							{email ? <i aria-hidden="true">·</i> : null}
							<small>{formatPlan(plan)}</small>
							<AccountDetailsInfo account={account} metadata={metadata} now={now} subscription={subscription} />
						</span>
					</div>
					<div className="account-header-actions codex-account-header-actions">
						<Switch
							aria-label={`${account.enabled ? "禁用" : "启用"}${account.name}`}
							checked={account.enabled}
							disabled={busy}
							onCheckedChange={() => onToggle()}
							title={account.enabled ? "禁用账户" : "启用账户"}
						/>
						<button aria-label="刷新额度" className="icon-button" disabled={loading || busy || !account.enabled} onClick={onRefresh} title="刷新额度" type="button">
							<RefreshIcon spinning={loading} />
						</button>
						<button className="button button-secondary account-header-button" disabled={busy} onClick={onEdit} type="button">重命名</button>
						<DeleteConfirmationDialog
							description="相关路由也会被移除。此操作无法撤销。"
							onConfirm={onDelete}
							title={`删除 Codex 账户“${email || account.name}”？`}
							trigger={<button className="button button-danger-quiet account-header-button" disabled={busy} type="button">删除</button>}
						/>
					</div>
				</header>
			</div>

			<div className="account-quota-section" aria-label={`${email || account.name} 的账户配额`}>
				{loading && !subscription ? (
					<div className="center-state account-quota-loading" role="status">
						<span className="spinner" aria-hidden="true" />
						<span>正在读取配额时间轴…</span>
					</div>
				) : null}
				{error ? (
					<div className="inline-alert error-alert account-quota-alert" role="alert">
						<AlertIcon />
						<span>{error}</span>
					</div>
				) : null}
				{subscription?.windows.length ? (
					<QuotaTimeline
						className={loading ? "account-quota-timeline is-refreshing" : "account-quota-timeline"}
						now={now}
						planType={plan}
						sampledAt={subscription.fetchedAt}
						windows={subscription.windows}
					/>
				) : !loading && !error ? (
					<p className="muted-message account-quota-empty">暂无额度数据</p>
				) : null}
			</div>
		</article>
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
	const detailsId = useId();
	const credits = subscription?.rateLimitResetCredits;
	const availableCredits = credits?.availableCount ?? null;
	const applicableCredits = credits?.applicableAvailableCount ?? null;
	const resetCredits = availableCredits === null
		? "暂无数据"
		: applicableCredits === null
			? String(Math.max(0, availableCredits))
			: `${Math.max(0, availableCredits)} · 可用 ${Math.max(0, applicableCredits)}`;

	return (
		<span className="plan-info account-plan-info">
			<button aria-describedby={detailsId} aria-label="查看账户详情" className="plan-info-button" type="button">
				<InfoIcon />
			</button>
			<span className="plan-tooltip" id={detailsId} role="tooltip">
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
			</span>
		</span>
	);
}

function AccountDetailRow({ danger = false, label, value }: { danger?: boolean; label: string; value: string }) {
	return (
		<span className="plan-tooltip-row">
			<span>{label}</span>
			<b className={danger ? "danger-text" : undefined}>{value}</b>
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
				<form className="editor-form" onSubmit={submit}>
					<label htmlFor="codex-account-name"><span>显示名称</span><input autoFocus disabled={busy} id="codex-account-name" maxLength={100} onChange={(event) => setName(event.target.value)} required type="text" value={name} /></label>
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
				{loading && !flow ? <div className="center-state"><span className="spinner" /><span>获取登录码…</span></div> : null}
				{flow ? (
					<div className="device-login-content">
						<button className="device-code-button" onClick={() => void copyCode()} type="button">
							<code>{flow.authorization.userCode}</code><span>{copied ? "已复制" : "点击复制"}</span>
						</button>
						<Button asChild><a href={flow.authorization.verificationUri} rel="noreferrer" target="_blank">打开登录页面 <ExternalIcon /></a></Button>
						<small>等待授权…</small>
					</div>
				) : null}
				{error ? <div className="inline-alert error-alert"><span>{error}</span></div> : null}
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

function RefreshIcon({ spinning }: { spinning: boolean }) {
	return <svg className={`icon${spinning ? " icon-spinning" : ""}`} aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24"><path d="M20 11a8 8 0 1 0-2.3 5.7" /><path d="M20 4v7h-7" /></svg>;
}

function InfoIcon() {
	return <svg className="icon" aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" /><path d="M12 11v5" /><path d="M12 8h.01" /></svg>;
}

function AlertIcon() {
	return <svg className="icon" aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24"><path d="M12 9v4" /><path d="M12 17h.01" /><path d="M10.3 3.9 2.4 18a2 2 0 0 0 1.75 3h15.7a2 2 0 0 0 1.75-3L13.7 3.9a2 2 0 0 0-3.4 0Z" /></svg>;
}

function ExternalIcon() {
	return <svg className="icon" aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" viewBox="0 0 24 24"><path d="M15 3h6v6" /><path d="m10 14 11-11" /><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" /></svg>;
}
