// Dialogs shared by the console pages: identity editors, account group
// editor, Codex account rename and device login.

import { useState, type FormEvent } from "react";
import { CopyIcon, ExternalLinkIcon, EyeIcon, EyeOffIcon, TriangleAlertIcon } from "lucide-react";

import type {
	AccountGroup,
	AccountGroupStrategy,
	AuthProxyAccount,
	AuthProxyAccountInput,
	ClientApiKey,
	ClientApiKeyInput,
	CodexAccount,
	CodexAccountDeviceAuthorization,
	RouteAssignment,
} from "@/admin-api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Badge } from "@/components/ui/badge";
import {
	Field,
	FieldContent,
	FieldDescription,
	FieldGroup,
	FieldLabel,
	FieldLegend,
	FieldSeparator,
	FieldSet,
	FieldTitle,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectSeparator,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

import {
	MAX_ACCOUNT_ID_LENGTH,
	MAX_API_KEY_LENGTH,
	MIN_API_KEY_LENGTH,
	STRATEGY_OPTIONS,
	generateApiKey,
	newGroup,
	parseRouteTargetValue,
	routeTargetValue,
	strategyHint,
	validAccountId,
	validApiKey,
	type RouteTargetSelection,
} from "./routing-model";

export { default as CredentialImportDialog } from "@/CredentialImportDialog";
export { default as DeleteConfirmationDialog } from "@/DeleteConfirmationDialog";

const UNASSIGNED_ROUTE_TARGET_VALUE = "__unassigned_route_target__";

export function ApiKeyEditor({
	accounts,
	entry,
	groups,
	loading,
	onCancel,
	onSave,
	route,
}: {
	accounts: CodexAccount[];
	entry: ClientApiKey | "new";
	groups: AccountGroup[];
	loading: boolean;
	onCancel: () => void;
	onSave: (value: ClientApiKeyInput, target: RouteTargetSelection) => void;
	route: RouteAssignment | null;
}) {
	const initial = entry === "new" ? { name: "", key: generateApiKey(), enabled: true } : entry;
	const [name, setName] = useState(initial.name);
	const [key, setKey] = useState(initial.key);
	const [enabled, setEnabled] = useState(initial.enabled);
	const [visible, setVisible] = useState(entry === "new");
	const [target, setTarget] = useState<RouteTargetSelection>(route);

	function submit(event: FormEvent<HTMLFormElement>): void {
		event.preventDefault();
		if (validApiKey(key) && name.trim()) onSave({ name: name.trim(), key, enabled }, target);
	}

	return (
		<Dialog open onOpenChange={(open) => { if (!open && !loading) onCancel(); }}>
			<DialogContent className="flex h-[min(40rem,calc(100svh-2rem))] flex-col gap-0 overflow-hidden p-0 sm:max-w-lg" showCloseButton={!loading}>
				<DialogHeader className="shrink-0 border-b px-4 py-4 pr-12 text-left sm:px-6 sm:pr-14">
					<DialogTitle>{entry === "new" ? "添加 API Key" : "编辑 API Key"}</DialogTitle>
					<DialogDescription>配置 API Key、路由目标与启用状态。</DialogDescription>
				</DialogHeader>
				<form className="flex min-h-0 flex-1 flex-col overflow-hidden" onSubmit={submit}>
					<ScrollArea className="h-full min-h-0 flex-1">
						<FieldGroup className="p-4 sm:p-6">
							<Field>
								<FieldLabel htmlFor="api-key-name">名称</FieldLabel>
								<Input autoFocus disabled={loading} id="api-key-name" maxLength={100} onChange={(event) => setName(event.target.value)} placeholder="例如：my-laptop" required type="text" value={name} />
							</Field>
							<Field>
								<FieldLabel htmlFor="api-key-value">Key</FieldLabel>
								<InputGroup>
									<InputGroupInput autoComplete="off" disabled={loading} id="api-key-value" maxLength={MAX_API_KEY_LENGTH} minLength={MIN_API_KEY_LENGTH} onChange={(event) => setKey(event.target.value)} required spellCheck={false} type={visible ? "text" : "password"} value={key} />
									<InputGroupAddon align="inline-end">
										<InputGroupButton aria-label={visible ? "隐藏" : "显示"} onClick={() => setVisible((value) => !value)} size="icon-xs">
											{visible ? <EyeOffIcon /> : <EyeIcon />}
										</InputGroupButton>
									</InputGroupAddon>
								</InputGroup>
								<FieldDescription>至少 {MIN_API_KEY_LENGTH} 个字符，并包含字母、数字与符号。</FieldDescription>
							</Field>
							<AccountTargetSelect accounts={accounts} groups={groups} loading={loading} onChange={setTarget} target={target} unassignedHint="未分配时，该 API Key 的请求将不可用。" />
							<Field className="rounded-lg border p-3" orientation="horizontal">
								<FieldContent>
									<FieldLabel htmlFor="api-key-enabled">启用</FieldLabel>
									<FieldDescription>允许客户端使用这个 API Key 调用服务。</FieldDescription>
								</FieldContent>
								<Switch checked={enabled} disabled={loading} id="api-key-enabled" onCheckedChange={setEnabled} />
							</Field>
							<Button className="w-fit" disabled={loading} onClick={() => setKey(generateApiKey())} type="button" variant="outline">重新生成 Key</Button>
						</FieldGroup>
					</ScrollArea>
					<DialogFooter className="m-0 shrink-0 rounded-none px-4 py-3 sm:px-6 sm:py-4">
						<DialogClose asChild><Button disabled={loading} type="button" variant="outline">取消</Button></DialogClose>
						<Button disabled={loading || !name.trim() || !validApiKey(key)} type="submit">{loading ? <Spinner /> : null}{loading ? "保存中…" : "保存"}</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}

export function ProxyEditor({
	accounts,
	entry,
	groups,
	loading,
	onCancel,
	onSave,
	route,
}: {
	accounts: CodexAccount[];
	entry: AuthProxyAccount | "new";
	groups: AccountGroup[];
	loading: boolean;
	onCancel: () => void;
	onSave: (value: AuthProxyAccountInput, target: RouteTargetSelection) => void;
	route: RouteAssignment | null;
}) {
	const initial = entry === "new" ? { name: "", accountId: "", enabled: true } : entry;
	const [name, setName] = useState(initial.name);
	const [accountId, setAccountId] = useState(initial.accountId);
	const [enabled, setEnabled] = useState(initial.enabled);
	const [target, setTarget] = useState<RouteTargetSelection>(route);

	function submit(event: FormEvent<HTMLFormElement>): void {
		event.preventDefault();
		if (name.trim() && validAccountId(accountId)) onSave({ name: name.trim(), accountId, enabled }, target);
	}

	return (
		<Dialog open onOpenChange={(open) => { if (!open && !loading) onCancel(); }}>
			<DialogContent className="flex h-[min(36rem,calc(100svh-2rem))] flex-col gap-0 overflow-hidden p-0 sm:max-w-lg" showCloseButton={!loading}>
				<DialogHeader className="shrink-0 border-b px-4 py-4 pr-12 text-left sm:px-6 sm:pr-14">
					<DialogTitle>{entry === "new" ? "添加下游账户" : "编辑下游账户"}</DialogTitle>
					<DialogDescription>配置 account_id、路由目标与启用状态。</DialogDescription>
				</DialogHeader>
				<form className="flex min-h-0 flex-1 flex-col overflow-hidden" onSubmit={submit}>
					<ScrollArea className="h-full min-h-0 flex-1">
						<FieldGroup className="p-4 sm:p-6">
							<Field>
								<FieldLabel htmlFor="proxy-name">名称</FieldLabel>
								<Input autoFocus disabled={loading} id="proxy-name" maxLength={100} onChange={(event) => setName(event.target.value)} placeholder="例如：production" required type="text" value={name} />
							</Field>
							<Field>
								<FieldLabel htmlFor="proxy-account-id">account_id</FieldLabel>
								<Input autoCapitalize="none" autoComplete="off" disabled={loading} id="proxy-account-id" maxLength={MAX_ACCOUNT_ID_LENGTH} onChange={(event) => setAccountId(event.target.value)} required spellCheck={false} type="text" value={accountId} />
								<FieldDescription>填写下游 auth proxy 请求中使用的 account_id。</FieldDescription>
							</Field>
							<AccountTargetSelect accounts={accounts} groups={groups} loading={loading} onChange={setTarget} target={target} unassignedHint="未分配时，将继续使用来访请求中的上游凭据。" />
							<Field className="rounded-lg border p-3" orientation="horizontal">
								<FieldContent>
									<FieldLabel htmlFor="proxy-enabled">启用</FieldLabel>
									<FieldDescription>允许该 account_id 使用当前路由设置。</FieldDescription>
								</FieldContent>
								<Switch checked={enabled} disabled={loading} id="proxy-enabled" onCheckedChange={setEnabled} />
							</Field>
						</FieldGroup>
					</ScrollArea>
					<DialogFooter className="m-0 shrink-0 rounded-none px-4 py-3 sm:px-6 sm:py-4">
						<DialogClose asChild><Button disabled={loading} type="button" variant="outline">取消</Button></DialogClose>
						<Button disabled={loading || !name.trim() || !validAccountId(accountId)} type="submit">{loading ? <Spinner /> : null}{loading ? "保存中…" : "保存"}</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}

export function AccountTargetSelect({
	accounts,
	groups,
	loading,
	onChange,
	target,
	unassignedHint,
}: {
	accounts: CodexAccount[];
	groups: AccountGroup[];
	loading: boolean;
	onChange: (target: RouteTargetSelection) => void;
	target: RouteTargetSelection;
	unassignedHint: string;
}) {
	return (
		<Field className="route-target-field">
			<FieldLabel htmlFor="identity-account-target" id="identity-account-target-label">账户或账户组</FieldLabel>
			<Select
				disabled={loading}
				onValueChange={(value) => onChange(parseRouteTargetValue(value === UNASSIGNED_ROUTE_TARGET_VALUE ? "" : value))}
				value={routeTargetValue(target) || UNASSIGNED_ROUTE_TARGET_VALUE}
			>
				<SelectTrigger aria-labelledby="identity-account-target-label" className="w-full" id="identity-account-target">
					<SelectValue />
				</SelectTrigger>
				<SelectContent position="popper">
					<SelectGroup>
						<SelectItem value={UNASSIGNED_ROUTE_TARGET_VALUE}>未分配</SelectItem>
					</SelectGroup>
					{groups.length || accounts.length ? <SelectSeparator /> : null}
					{groups.length ? (
						<SelectGroup>
							<SelectLabel>账户组</SelectLabel>
							{groups.map((group) => <SelectItem key={group.id} value={`group:${group.id}`}>{group.name}</SelectItem>)}
						</SelectGroup>
					) : null}
					{accounts.length ? (
						<SelectGroup>
							<SelectLabel>单个 Codex 账户</SelectLabel>
							{accounts.map((account) => (
								<SelectItem disabled={!account.enabled} key={account.id} value={`account:${account.id}`}>
									{account.name}{account.enabled ? "" : "（已禁用）"}
								</SelectItem>
							))}
						</SelectGroup>
					) : null}
				</SelectContent>
			</Select>
			<FieldDescription>{groups.length === 0 && accounts.length === 0 ? "请先在 Codex 账户页添加账户或账户组。" : unassignedHint}</FieldDescription>
		</Field>
	);
}

export function GroupEditorDialog({
	accounts,
	entry,
	onCancel,
	onSave,
	saving,
}: {
	accounts: CodexAccount[];
	entry: AccountGroup | "new";
	onCancel: () => void;
	onSave: (group: AccountGroup) => Promise<void>;
	saving: boolean;
}) {
	const initial = entry === "new" ? newGroup() : entry;
	const [name, setName] = useState(initial.name);
	const [accountIds, setAccountIds] = useState(initial.accountIds);
	const [strategy, setStrategy] = useState<AccountGroupStrategy>(initial.strategy);
	const [sessionAffinity, setSessionAffinity] = useState(initial.sessionAffinity);
	const [ttl, setTtl] = useState(initial.sessionAffinityTtl);
	const selectedAccountCount = accounts.filter((account) => accountIds.includes(account.id)).length;

	function submit(event: FormEvent<HTMLFormElement>): void {
		event.preventDefault();
		if (!name.trim() || (strategy !== "fallback" && sessionAffinity && !ttl.trim())) return;
		void onSave({
			id: initial.id,
			name: name.trim(),
			accountIds,
			strategy,
			sessionAffinity: strategy === "fallback" ? false : sessionAffinity,
			sessionAffinityTtl: ttl.trim() || "1h",
		});
	}

	function toggleAccount(id: string): void {
		setAccountIds((current) => current.includes(id)
			? current.filter((value) => value !== id)
			: [...current, id]);
	}

	return (
		<Dialog open onOpenChange={(open) => { if (!open && !saving) onCancel(); }}>
			<DialogContent className="flex h-[min(48rem,calc(100svh-2rem))] flex-col gap-0 overflow-hidden p-0 sm:max-w-2xl" showCloseButton={!saving}>
				<DialogHeader className="shrink-0 border-b px-4 py-4 pr-12 text-left sm:px-6 sm:pr-14">
					<DialogTitle>{entry === "new" ? "添加账户组" : "编辑账户组"}</DialogTitle>
					<DialogDescription>将账户整理为一个可复用的路由池，并设置请求分配方式。</DialogDescription>
				</DialogHeader>

				<form aria-busy={saving} className="flex min-h-0 flex-1 flex-col overflow-hidden" onSubmit={submit}>
					<ScrollArea className="h-full min-h-0 flex-1">
						<FieldGroup className="p-4 sm:p-6">
							<Field>
								<FieldLabel htmlFor="group-name">组名称</FieldLabel>
								<Input
									autoFocus
									disabled={saving}
									id="group-name"
									maxLength={100}
									onChange={(event) => setName(event.target.value)}
									placeholder="例如：production-pool"
									required
									type="text"
									value={name}
								/>
								<FieldDescription>用于在 API Key 和下游账户的路由设置中识别这个组。</FieldDescription>
							</Field>

							<FieldSeparator />

							<FieldSet>
								<FieldLegend className="mb-0">组内账户</FieldLegend>
								<div className="flex flex-col items-start justify-between gap-2 sm:flex-row">
									<FieldDescription className="m-0">请求只会在所选账户之间调度。</FieldDescription>
									<Badge variant={selectedAccountCount > 0 ? "secondary" : "outline"}>
										{selectedAccountCount} / {accounts.length} 已选
									</Badge>
								</div>

								{accounts.length === 0 ? (
									<div className="rounded-lg border border-dashed p-4 text-center text-sm text-muted-foreground sm:p-6">
										暂无 Codex 账户，请先添加账户后再配置成员。
									</div>
								) : (
									<FieldGroup data-slot="checkbox-group" className="gap-2">
										{accounts.map((account) => (
											<FieldLabel
												className={cn(
													"cursor-pointer has-[:disabled]:cursor-not-allowed has-data-checked:bg-transparent dark:has-data-checked:bg-transparent",
													!account.enabled && "border-dashed",
												)}
												key={account.id}
											>
												<Field orientation="horizontal" data-disabled={saving || undefined}>
													<Checkbox
														aria-label={`选择账户 ${account.name}`}
														checked={accountIds.includes(account.id)}
														className={TRANSPARENT_CHECKBOX_CLASS}
														disabled={saving}
														onCheckedChange={() => toggleAccount(account.id)}
													/>
													<FieldContent className="min-w-0">
														<FieldTitle className="max-w-full"><span className="truncate">{account.name}</span></FieldTitle>
														<FieldDescription className="line-clamp-1">{account.oauth?.email ?? "Codex 账户"}</FieldDescription>
													</FieldContent>
													{!account.enabled ? <Badge variant="outline">已禁用</Badge> : null}
												</Field>
											</FieldLabel>
										))}
									</FieldGroup>
								)}
							</FieldSet>

							<FieldSeparator />

							<FieldSet>
								<FieldLegend>调度方式</FieldLegend>
								<FieldDescription>决定新请求如何选择组内账户，以及是否复用上一次选择。</FieldDescription>
								<FieldGroup className="gap-4">
									<Field>
										<FieldLabel id="group-strategy-label">负载均衡策略</FieldLabel>
										<Select
											disabled={saving}
											onValueChange={(value) => setStrategy(value as AccountGroupStrategy)}
											value={strategy}
										>
											<SelectTrigger
												aria-describedby="group-strategy-description"
												aria-labelledby="group-strategy-label"
												className="w-full"
											>
												<SelectValue />
											</SelectTrigger>
											<SelectContent position="popper">
												<SelectGroup>
													{STRATEGY_OPTIONS.map((option) => (
														<SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>
													))}
												</SelectGroup>
											</SelectContent>
										</Select>
										<FieldDescription id="group-strategy-description">{strategyHint(strategy)}</FieldDescription>
									</Field>

									{strategy !== "fallback" ? (
										<>
											<Field className="rounded-lg border p-3" data-disabled={saving || undefined} orientation="horizontal">
												<FieldContent>
													<FieldLabel htmlFor="group-session-affinity">会话保持</FieldLabel>
													<FieldDescription id="group-session-affinity-description">同一会话优先使用同一账户，减少上下文切换。</FieldDescription>
												</FieldContent>
												<Switch
													aria-describedby="group-session-affinity-description"
													checked={sessionAffinity}
													disabled={saving}
													id="group-session-affinity"
													onCheckedChange={setSessionAffinity}
												/>
											</Field>

											{sessionAffinity ? (
												<div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
													<Field>
														<FieldLabel htmlFor="group-affinity-ttl">保持时间</FieldLabel>
														<Input
															aria-describedby="group-affinity-ttl-description"
															disabled={saving || ttl === "unlimited"}
															id="group-affinity-ttl"
															maxLength={64}
															onChange={(event) => setTtl(event.target.value)}
															placeholder="例如：1h、7d"
															required={ttl !== "unlimited"}
															type="text"
															value={ttl === "unlimited" ? "" : ttl}
														/>
														<FieldDescription id="group-affinity-ttl-description">支持 30m、1h、7d 等时长格式。</FieldDescription>
													</Field>
													<Field className="w-auto" data-disabled={saving || undefined} orientation="horizontal">
														<Checkbox
															checked={ttl === "unlimited"}
															className={TRANSPARENT_CHECKBOX_CLASS}
															disabled={saving}
															id="group-affinity-unlimited"
															onCheckedChange={(checked) => setTtl(checked ? "unlimited" : "1h")}
														/>
														<FieldLabel className="whitespace-nowrap" htmlFor="group-affinity-unlimited">不限期</FieldLabel>
													</Field>
												</div>
											) : null}
										</>
									) : null}
								</FieldGroup>
							</FieldSet>
						</FieldGroup>
					</ScrollArea>

					<DialogFooter className="m-0 shrink-0 rounded-none px-4 py-3 sm:px-6 sm:py-4">
						<DialogClose asChild>
							<Button disabled={saving} type="button" variant="outline">取消</Button>
						</DialogClose>
						<Button disabled={saving || !name.trim() || (strategy !== "fallback" && sessionAffinity && !ttl.trim())} type="submit">
							{saving ? <Spinner /> : null}
							{saving ? "保存中…" : entry === "new" ? "创建账户组" : "保存更改"}
						</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}

const TRANSPARENT_CHECKBOX_CLASS = "data-checked:bg-transparent data-checked:text-foreground dark:data-checked:bg-transparent";

export function AccountNameDialog({
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

export function DeviceLoginDialog({
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
						<Button className="device-code-button h-auto w-full justify-between gap-3 border-dashed p-3 max-[46rem]:flex-col max-[46rem]:items-start" onClick={() => void copyCode()} type="button" variant="outline">
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

