// Dialogs shared by the console pages: user editor, account group editor,
// Codex account editor and device login.

import { useState, type FormEvent } from "react";
import { CopyIcon, ExternalLinkIcon, EyeIcon, EyeOffIcon, PlusIcon, RefreshCwIcon, Trash2Icon, TriangleAlertIcon } from "lucide-react";

import type {
	AccountGroup,
	AccountGroupStrategy,
	CodexAccount,
	CodexAccountDeviceAuthorization,
	CredentialType,
	ProxyTestResult,
	RouteAssignment,
	User,
	UserInput,
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
import { randomUuid } from "@/lib/uuid";

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
import { CREDENTIAL_LABEL } from "./identities/model";
import { copyToClipboard } from "@/lib/clipboard";

export { default as CredentialImportDialog } from "@/CredentialImportDialog";
export { default as DeleteConfirmationDialog } from "@/DeleteConfirmationDialog";

const UNASSIGNED_ROUTE_TARGET_VALUE = "__unassigned_route_target__";
const MAX_CREDENTIALS = 20;

interface CredentialDraft {
	/** Stable React key; new credentials have no server id yet. */
	key: string;
	id?: string;
	type: CredentialType;
	value: string;
	name: string;
	enabled: boolean;
}

function newCredential(type: CredentialType): CredentialDraft {
	return { key: randomUuid(), type, value: type === "api_key" ? generateApiKey() : "", name: "", enabled: true };
}

function validCredential(credential: CredentialDraft): boolean {
	return credential.type === "api_key" ? validApiKey(credential.value) : validAccountId(credential.value.trim());
}

export function UserEditor({
	accounts,
	entry,
	groups,
	loading,
	onCancel,
	onSave,
	route,
}: {
	accounts: CodexAccount[];
	entry: User | "new";
	groups: AccountGroup[];
	loading: boolean;
	onCancel: () => void;
	onSave: (value: UserInput, target: RouteTargetSelection) => void;
	route: RouteAssignment | null;
}) {
	const [name, setName] = useState(entry === "new" ? "" : entry.name);
	const [enabled, setEnabled] = useState(entry === "new" ? true : entry.enabled);
	const [credentials, setCredentials] = useState<CredentialDraft[]>(() =>
		entry === "new" ? [newCredential("api_key")] : entry.credentials.map((credential) => ({ ...credential, key: credential.id })),
	);
	const [target, setTarget] = useState<RouteTargetSelection>(route);

	const values = credentials.map((credential) => (credential.type === "api_key" ? credential.value : credential.value.trim()));
	const duplicate = new Set(values).size !== values.length;
	const valid = Boolean(name.trim()) && credentials.length > 0 && credentials.length <= MAX_CREDENTIALS && credentials.every(validCredential) && !duplicate;

	function update(key: string, patch: Partial<CredentialDraft>): void {
		setCredentials((current) => current.map((credential) => (credential.key === key ? { ...credential, ...patch } : credential)));
	}

	function submit(event: FormEvent<HTMLFormElement>): void {
		event.preventDefault();
		if (!valid) return;
		onSave(
			{
				name: name.trim(),
				enabled,
				credentials: credentials.map(({ id, type, value, name: label, enabled: active }) => ({
					...(id ? { id } : {}),
					type,
					value: type === "api_key" ? value : value.trim(),
					name: label.trim(),
					enabled: active,
				})),
			},
			target,
		);
	}

	return (
		<Dialog open onOpenChange={(open) => { if (!open && !loading) onCancel(); }}>
			<DialogContent className="flex h-[min(44rem,calc(100svh-2rem))] flex-col gap-0 overflow-hidden p-0 sm:max-w-xl" showCloseButton={!loading}>
				<DialogHeader className="shrink-0 border-b px-4 py-4 pr-12 text-left sm:px-6 sm:pr-14">
					<DialogTitle>{entry === "new" ? "添加用户" : "编辑用户"}</DialogTitle>
					<DialogDescription>用户的任一凭据都可调用服务和登录控制台，用量合并统计。</DialogDescription>
				</DialogHeader>
				<form className="flex min-h-0 flex-1 flex-col overflow-hidden" onSubmit={submit}>
					<ScrollArea className="h-full min-h-0 flex-1">
						<FieldGroup className="p-4 sm:p-6">
							<Field>
								<FieldLabel htmlFor="user-name">名称</FieldLabel>
								<Input autoFocus disabled={loading} id="user-name" maxLength={100} onChange={(event) => setName(event.target.value)} placeholder="例如：张三" required type="text" value={name} />
							</Field>
							<FieldSet>
								<FieldLegend variant="label">凭据</FieldLegend>
								<div className="flex flex-col gap-2">
									{credentials.map((credential) => (
										<CredentialRow
											credential={credential}
											duplicate={values.filter((value) => value === (credential.type === "api_key" ? credential.value : credential.value.trim())).length > 1}
											key={credential.key}
											loading={loading}
											onChange={(patch) => update(credential.key, patch)}
											onRemove={credentials.length > 1 ? () => setCredentials((current) => current.filter((item) => item.key !== credential.key)) : undefined}
										/>
									))}
								</div>
								<div className="flex flex-wrap gap-2">
									{(["api_key", "account_id"] as const).map((type) => (
										<Button
											disabled={loading || credentials.length >= MAX_CREDENTIALS}
											key={type}
											onClick={() => setCredentials((current) => [...current, newCredential(type)])}
											size="sm"
											type="button"
											variant="outline"
										>
											<PlusIcon data-icon="inline-start" />
											{CREDENTIAL_LABEL[type]}
										</Button>
									))}
								</div>
								<FieldDescription>API Key 至少 {MIN_API_KEY_LENGTH} 位，含字母、数字与符号。账户 ID 是 auth proxy 请求携带的 ChatGPT 账户 ID。</FieldDescription>
							</FieldSet>
							<AccountTargetSelect accounts={accounts} groups={groups} loading={loading} onChange={setTarget} target={target} unassignedHint="未分配时 API Key 无法调用，账户 ID 透传客户端自带的上游凭据。" />
							<Field className="rounded-lg border p-3" orientation="horizontal">
								<FieldContent>
									<FieldLabel htmlFor="user-enabled">启用</FieldLabel>
									<FieldDescription>停用后该用户的全部凭据失效。</FieldDescription>
								</FieldContent>
								<Switch checked={enabled} disabled={loading} id="user-enabled" onCheckedChange={setEnabled} />
							</Field>
						</FieldGroup>
					</ScrollArea>
					<DialogFooter className="m-0 shrink-0 rounded-none px-4 py-3 sm:px-6 sm:py-4">
						<DialogClose asChild><Button disabled={loading} type="button" variant="outline">取消</Button></DialogClose>
						<Button disabled={loading || !valid} type="submit">{loading ? <Spinner /> : null}{loading ? "保存中…" : "保存"}</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}

function CredentialRow({
	credential,
	duplicate,
	loading,
	onChange,
	onRemove,
}: {
	credential: CredentialDraft;
	duplicate: boolean;
	loading: boolean;
	onChange: (patch: Partial<CredentialDraft>) => void;
	onRemove: (() => void) | undefined;
}) {
	const isKey = credential.type === "api_key";
	const [visible, setVisible] = useState(!credential.id);
	const invalid = Boolean(credential.value) && (!validCredential(credential) || duplicate);
	return (
		<div className={cn("flex flex-col gap-2 rounded-lg border p-3", !credential.enabled && "bg-muted/40")}>
			<div className="flex items-center gap-2">
				<Badge className="shrink-0" variant="secondary">{CREDENTIAL_LABEL[credential.type]}</Badge>
				<InputGroup className="min-w-0 flex-1">
					<InputGroupInput
						aria-invalid={invalid || undefined}
						aria-label={CREDENTIAL_LABEL[credential.type]}
						autoCapitalize="none"
						autoComplete="off"
						className="font-mono text-xs"
						disabled={loading}
						maxLength={isKey ? MAX_API_KEY_LENGTH : MAX_ACCOUNT_ID_LENGTH}
						onChange={(event) => onChange({ value: event.target.value })}
						placeholder={isKey ? undefined : "下游请求的 ChatGPT 账户 ID"}
						required
						spellCheck={false}
						type={isKey && !visible ? "password" : "text"}
						value={credential.value}
					/>
					{isKey ? (
						<InputGroupAddon align="inline-end">
							<InputGroupButton aria-label={visible ? "隐藏" : "显示"} onClick={() => setVisible((value) => !value)} size="icon-xs">
								{visible ? <EyeOffIcon /> : <EyeIcon />}
							</InputGroupButton>
							<InputGroupButton aria-label="重新生成" disabled={loading} onClick={() => { onChange({ value: generateApiKey() }); setVisible(true); }} size="icon-xs">
								<RefreshCwIcon />
							</InputGroupButton>
						</InputGroupAddon>
					) : null}
				</InputGroup>
				<Button aria-label="移除凭据" className="text-muted-foreground" disabled={loading || !onRemove} onClick={onRemove} size="icon-sm" type="button" variant="ghost">
					<Trash2Icon />
				</Button>
			</div>
			<div className="flex items-center gap-2">
				<Input aria-label="备注" className="h-8 flex-1" disabled={loading} maxLength={100} onChange={(event) => onChange({ name: event.target.value })} placeholder="备注（可选），如 my-laptop" type="text" value={credential.name} />
				<Switch aria-label={credential.enabled ? "停用凭据" : "启用凭据"} checked={credential.enabled} disabled={loading} onCheckedChange={(checked) => onChange({ enabled: checked })} />
			</div>
			{duplicate ? <p className="text-xs text-destructive">与其他凭据重复。</p> : null}
		</div>
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
								<FieldDescription>用于在用户的路由设置中识别这个组。</FieldDescription>
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

export function AccountEditDialog({
	account,
	busy,
	onCancel,
	onSave,
	onTestProxy,
}: {
	account: CodexAccount;
	busy: boolean;
	onCancel: () => void;
	onSave: (value: { name: string; proxy: string | null }) => void;
	onTestProxy: (proxy: string) => Promise<ProxyTestResult>;
}) {
	const [name, setName] = useState(account.name);
	const [proxy, setProxy] = useState(account.proxy ?? "");
	const [testing, setTesting] = useState(false);
	const [result, setResult] = useState<ProxyTestResult | null>(null);

	function submit(event: FormEvent<HTMLFormElement>): void {
		event.preventDefault();
		const value = name.trim();
		if (value) onSave({ name: value, proxy: proxy.trim() || null });
	}

	async function testProxy(): Promise<void> {
		setTesting(true);
		setResult(null);
		try {
			setResult(await onTestProxy(proxy.trim()));
		} catch (error) {
			setResult({ ok: false, latencyMs: 0, error: error instanceof Error ? error.message : "测试失败" });
		} finally {
			setTesting(false);
		}
	}

	return (
		<Dialog open onOpenChange={(open) => { if (!open && !busy) onCancel(); }}>
			<DialogContent showCloseButton={!busy}>
				<DialogHeader>
					<DialogTitle>编辑账户</DialogTitle>
				</DialogHeader>
				<form className="grid gap-4" onSubmit={submit}>
					<FieldGroup>
						<Field>
							<FieldLabel htmlFor="codex-account-name">显示名称</FieldLabel>
							<Input autoFocus disabled={busy} id="codex-account-name" maxLength={100} onChange={(event) => setName(event.target.value)} required type="text" value={name} />
						</Field>
						<Field>
							<FieldLabel htmlFor="codex-account-proxy">代理</FieldLabel>
							<InputGroup>
								<InputGroupInput
									autoComplete="off"
									disabled={busy}
									id="codex-account-proxy"
									onChange={(event) => {
										setProxy(event.target.value);
										setResult(null);
									}}
									placeholder="socks5h://user:pass@host:1080"
									spellCheck={false}
									type="text"
									value={proxy}
								/>
								<InputGroupAddon align="inline-end">
									<InputGroupButton disabled={busy || testing} onClick={() => void testProxy()} type="button">
										{testing ? <Spinner /> : null}
										测试
									</InputGroupButton>
								</InputGroupAddon>
							</InputGroup>
							<FieldDescription className={result && !result.ok ? "text-danger" : undefined}>
								{result
									? result.ok
										? `可用 · 出口 ${result.ip ?? "未知"}${result.location ? `（${result.location}）` : ""} · ${result.latencyMs} ms`
										: `不可用：${result.error ?? "未知错误"}`
									: "支持 socks5、socks5h、http；留空使用默认线路。"}
							</FieldDescription>
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
		try {
			await copyToClipboard(flow.authorization.userCode);
			setCopiedCode(flow.authorization.userCode);
		} catch {
			setCopiedCode(null);
		}
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

