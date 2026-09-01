import { useState, type FormEvent } from "react";
import type {
	AccountGroup,
	AccountGroupStrategy,
	CodexAccount,
	RouteAssignment,
} from "./admin-api";

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
		if (!window.confirm(`删除账户组“${group.name}”？使用该组的调用身份将变为未分配。`)) return;
		await onChange(
			groups.filter((entry) => entry.id !== group.id),
			routes.filter((route) => route.targetType !== "group" || route.targetId !== group.id),
		);
	}

	return (
		<section className="card account-groups-card" aria-labelledby="account-groups-title">
			<div className="card-header unified-section-header">
				<div>
					<h2 id="account-groups-title">账户组</h2>
					<p>将多个 Codex 账户组成调度池，再分配给 API Key 或下游账户。</p>
				</div>
				<button
					className="button button-primary"
					disabled={saving}
					onClick={() => setEditor("new")}
					type="button"
				>
					<PlusIcon />
					添加账户组
				</button>
			</div>

			{groups.length === 0 ? (
				<div className="account-groups-empty">
					<strong>暂无账户组</strong>
					<span>创建后即可在 API Key 和下游账户中直接选择。</span>
				</div>
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
		</section>
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
		<article className="account-group-card">
			<div className="account-group-card-heading">
				<div>
					<strong>{group.name}</strong>
					<span>{strategyLabel(group.strategy)} · {group.strategy === "fallback" ? "调用身份保持" : group.sessionAffinity ? `会话保持 ${formatTtl(group.sessionAffinityTtl)}` : "无会话保持"}</span>
				</div>
				<div className="account-group-actions">
					<button className="button button-secondary button-compact" disabled={saving} onClick={onEdit} type="button">编辑</button>
					<button className="button button-danger-quiet button-compact" disabled={saving} onClick={onDelete} type="button">删除</button>
				</div>
			</div>
			<div className="account-group-members">
				{members.map((account) => (
					<span className={account.enabled ? "" : "disabled-member"} key={account.id}>
						{account.name}
					</span>
				))}
				{members.length === 0 ? <small>尚未添加成员</small> : null}
			</div>
		</article>
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
		<div className="modal-backdrop">
			<section aria-labelledby="group-editor-title" aria-modal="true" className="modal group-editor-modal" role="dialog">
				<div className="modal-header">
					<h2 id="group-editor-title">{entry === "new" ? "添加账户组" : "编辑账户组"}</h2>
					<button aria-label="关闭" className="icon-button" disabled={saving} onClick={onCancel} type="button"><CloseIcon /></button>
				</div>
				<form className="editor-form group-editor-form" onSubmit={submit}>
					<label htmlFor="group-name">
						<span>组名称</span>
						<input autoFocus disabled={saving} id="group-name" maxLength={100} onChange={(event) => setName(event.target.value)} placeholder="例如：production-pool" required type="text" value={name} />
					</label>
					<div className="strategy-field">
						<span>负载均衡策略</span>
						<select
							disabled={saving}
							onChange={(event) => setStrategy(event.target.value as AccountGroupStrategy)}
							value={strategy}
						>
							{STRATEGY_OPTIONS.map((option) => (
								<option key={option.value} value={option.value}>{option.label}</option>
							))}
						</select>
						<small>{strategyHint(strategy)}</small>
					</div>
					<fieldset className="group-member-picker">
						<legend>组内账户</legend>
						{accounts.length === 0 ? <p>暂无 Codex 账户</p> : accounts.map((account) => (
							<label className={account.enabled ? "" : "disabled-member"} key={account.id}>
								<input checked={accountIds.includes(account.id)} disabled={saving} onChange={() => toggleAccount(account.id)} type="checkbox" />
								<span>
									<strong>{account.name}</strong>
									{account.oauth?.email || !account.enabled ? <small>{account.enabled ? account.oauth?.email : "已禁用"}</small> : null}
								</span>
							</label>
						))}
					</fieldset>
					{strategy !== "fallback" ? (
						<>
							<label className="switch-row group-affinity-switch">
								<span><strong>会话保持</strong><small>同一会话优先使用同一账户</small></span>
								<input checked={sessionAffinity} className="switch-control" disabled={saving} onChange={(event) => setSessionAffinity(event.target.checked)} type="checkbox" />
							</label>
							{sessionAffinity ? (
								<div className="strategy-field">
									<span id="group-affinity-ttl-label">保持时间</span>
									<div className="ttl-control">
										<input aria-labelledby="group-affinity-ttl-label" disabled={saving || ttl === "unlimited"} id="group-affinity-ttl" maxLength={64} onChange={(event) => setTtl(event.target.value)} placeholder="例如：1h、7d" required type="text" value={ttl === "unlimited" ? "" : ttl} />
										<label><input checked={ttl === "unlimited"} disabled={saving} onChange={(event) => setTtl(event.target.checked ? "unlimited" : "1h")} type="checkbox" />不限期</label>
									</div>
								</div>
							) : null}
						</>
					) : null}
					<div className="modal-actions">
						<button className="button button-secondary" disabled={saving} onClick={onCancel} type="button">取消</button>
						<button className="button button-primary" disabled={saving || !name.trim() || (strategy !== "fallback" && sessionAffinity && !ttl.trim())} type="submit">{saving ? <span className="spinner" /> : null}{saving ? "保存中…" : "保存"}</button>
					</div>
				</form>
			</section>
		</div>
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

function PlusIcon() {
	return <svg className="icon" aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.8" viewBox="0 0 24 24"><path d="M12 5v14" /><path d="M5 12h14" /></svg>;
}

function CloseIcon() {
	return <svg className="icon" aria-hidden="true" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.8" viewBox="0 0 24 24"><path d="m6 6 12 12" /><path d="M18 6 6 18" /></svg>;
}
