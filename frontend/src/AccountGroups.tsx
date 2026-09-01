import { useState, type FormEvent } from "react";
import { PlusIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardAction,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
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
import {
	Empty,
	EmptyDescription,
	EmptyHeader,
	EmptyTitle,
} from "@/components/ui/empty";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import type {
	AccountGroup,
	AccountGroupStrategy,
	CodexAccount,
	RouteAssignment,
} from "./admin-api";
import DeleteConfirmationDialog from "./DeleteConfirmationDialog";

interface AccountGroupsProps {
	accounts: CodexAccount[];
	groups: AccountGroup[];
	routes: RouteAssignment[];
	saving: boolean;
	onChange: (
		groups: AccountGroup[],
		routes: RouteAssignment[],
	) => Promise<boolean>;
}

export default function AccountGroups({
	accounts,
	groups,
	routes,
	saving,
	onChange,
}: AccountGroupsProps) {
	const [editor, setEditor] = useState<AccountGroup | "new" | null>(null);

	async function saveGroup(group: AccountGroup): Promise<void> {
		const nextGroups = editor === "new"
			? [...groups, group]
			: groups.map((entry) => entry.id === group.id ? group : entry);
		if (await onChange(nextGroups, routes)) setEditor(null);
	}

	async function deleteGroup(group: AccountGroup): Promise<void> {
		await onChange(
			groups.filter((entry) => entry.id !== group.id),
			routes.filter((route) => route.targetType !== "group" || route.targetId !== group.id),
		);
	}

	return (
		<Card className="account-groups-card" aria-labelledby="account-groups-title">
			<CardHeader className="max-sm:grid-cols-1">
				<CardTitle id="account-groups-title">账户组</CardTitle>
				<CardDescription>将多个 Codex 账户组成调度池，再分配给 API Key 或下游账户。</CardDescription>
				<CardAction className="max-sm:col-start-1 max-sm:row-auto max-sm:mt-2 max-sm:justify-self-stretch max-sm:[&>[data-slot=button]]:w-full">
					<Button disabled={saving} onClick={() => setEditor("new")} type="button">
						<PlusIcon data-icon="inline-start" />
						添加账户组
					</Button>
				</CardAction>
			</CardHeader>

			<CardContent>
				{groups.length === 0 ? (
					<Empty className="account-groups-empty border">
						<EmptyHeader>
							<EmptyTitle>暂无账户组</EmptyTitle>
							<EmptyDescription>创建后即可在 API Key 和下游账户中直接选择。</EmptyDescription>
						</EmptyHeader>
					</Empty>
				) : (
					<div className="account-group-list">
						{groups.map((group) => (
							<AccountGroupCard
								accounts={accounts}
								group={group}
								key={group.id}
								onDelete={() => void deleteGroup(group)}
								onEdit={() => setEditor(group)}
								saving={saving}
							/>
						))}
					</div>
				)}
			</CardContent>

			{editor ? (
				<GroupEditorDialog
					accounts={accounts}
					entry={editor}
					key={editor === "new" ? "new" : editor.id}
					onCancel={() => setEditor(null)}
					onSave={saveGroup}
					saving={saving}
				/>
			) : null}
		</Card>
	);
}

function AccountGroupCard({
	accounts,
	group,
	onDelete,
	onEdit,
	saving,
}: {
	accounts: CodexAccount[];
	group: AccountGroup;
	onDelete: () => void;
	onEdit: () => void;
	saving: boolean;
}) {
	const members = group.accountIds
		.map((id) => accounts.find((account) => account.id === id))
		.filter((account): account is CodexAccount => Boolean(account));

	return (
		<Card className="account-group-card" size="sm">
			<CardHeader>
				<CardTitle className="truncate" title={group.name}>{group.name}</CardTitle>
				<CardDescription className="truncate">{strategyLabel(group.strategy)} · {group.strategy === "fallback" ? "调用身份保持" : group.sessionAffinity ? `会话保持 ${formatTtl(group.sessionAffinityTtl)}` : "无会话保持"}</CardDescription>
				<CardAction className="account-group-actions">
					<Button disabled={saving} onClick={onEdit} size="sm" type="button" variant="outline">编辑</Button>
					<DeleteConfirmationDialog
						description="使用该组的调用身份将变为未分配。此操作无法撤销。"
						onConfirm={onDelete}
						title={`删除账户组“${group.name}”？`}
						trigger={<Button disabled={saving} size="sm" type="button" variant="destructive">删除</Button>}
					/>
				</CardAction>
			</CardHeader>
			<CardContent className="account-group-members">
				{members.map((account) => (
					<Badge className={account.enabled ? "" : "disabled-member"} key={account.id} variant="outline">
						{account.name}
					</Badge>
				))}
				{members.length === 0 ? <small>尚未添加成员</small> : null}
			</CardContent>
		</Card>
	);
}

function GroupEditorDialog({
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

function newGroup(): AccountGroup {
	return {
		id: crypto.randomUUID(),
		name: "",
		accountIds: [],
		strategy: "round-robin",
		sessionAffinity: true,
		sessionAffinityTtl: "1h",
	};
}

function formatTtl(value: string): string {
	return value === "unlimited" ? "不限期" : value;
}

const STRATEGY_OPTIONS = [
	{ value: "round-robin", label: "轮询" },
	{ value: "weighted-round-robin", label: "额度加权轮询" },
	{ value: "fallback", label: "调用身份粘性 / Fallback" },
] as const satisfies ReadonlyArray<{
	value: AccountGroupStrategy;
	label: string;
}>;

const TRANSPARENT_CHECKBOX_CLASS = "data-checked:bg-transparent data-checked:text-foreground dark:data-checked:bg-transparent";

function strategyLabel(value: AccountGroupStrategy): string {
	if (value === "weighted-round-robin") return "额度加权轮询";
	if (value === "fallback") return "Fallback";
	return "轮询";
}

function strategyHint(value: AccountGroupStrategy): string {
	if (value === "weighted-round-robin") return "按各账户 Codex 额度窗口的平均“剩余百分比 ÷ 剩余分钟”平滑分配。";
	if (value === "fallback") return "每个 API Key 或下游 account id 固定使用一个账户，仅在该账户不可用时切换。";
	return "按账户顺序平均轮换请求。";
}
