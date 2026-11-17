import { useState } from "react";
import { UserRoundIcon } from "lucide-react";

import type { AccountGroup, RouteAssignment } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { replaceUserRoute } from "@/admin/routing-model";
import { Tag } from "@/components/app/badges";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";

interface Candidate {
	id: string;
	name: string;
	enabled: boolean;
	/** Current route target name, when routed elsewhere. */
	current: string | null;
}

/** Chooses which users route to `group`. */
export function AssignIdentitiesDialog({ group, onClose }: { group: AccountGroup; onClose: () => void }) {
	const { state, saveRouting } = useAdmin();
	const assigned = new Set(state.routes.filter((route) => route.targetType === "group" && route.targetId === group.id).map((route) => route.userId));
	const [selected, setSelected] = useState<ReadonlySet<string>>(() => new Set(assigned));
	const [saving, setSaving] = useState(false);

	const targetName = (route: RouteAssignment | undefined) => {
		if (!route) return null;
		if (route.targetType === "group") return route.targetId === group.id ? null : (state.accountGroups.find((entry) => entry.id === route.targetId)?.name ?? "已移除的组");
		const account = state.codexAccounts.find((entry) => entry.id === route.targetId);
		return account ? `直连 ${account.oauth?.email ?? account.name}` : "已移除的账户";
	};
	const candidates: Candidate[] = state.users.map((user) => ({
		id: user.id,
		name: user.name,
		enabled: user.enabled,
		current: targetName(state.routes.find((route) => route.userId === user.id)),
	}));

	async function save() {
		let routes = state.routes;
		for (const candidate of candidates) {
			const id = candidate.id;
			if (selected.has(id) && !assigned.has(id)) {
				routes = replaceUserRoute(routes, id, { targetType: "group", targetId: group.id });
			} else if (!selected.has(id) && assigned.has(id)) {
				routes = replaceUserRoute(routes, id, null);
			}
		}
		setSaving(true);
		const ok = await saveRouting(state.accountGroups, routes);
		setSaving(false);
		if (ok) onClose();
	}

	return (
		<Dialog
			onOpenChange={(open) => {
				if (!open && !saving) onClose();
			}}
			open
		>
			<DialogContent className="flex max-h-[min(36rem,calc(100svh-2rem))] flex-col gap-0 p-0 sm:max-w-md" showCloseButton={!saving}>
				<DialogHeader className="border-b px-6 py-4 text-left">
					<DialogTitle>分配给用户</DialogTitle>
					<DialogDescription>选中的用户将路由到「{group.name}」，原有分配会被替换。</DialogDescription>
				</DialogHeader>
				<ScrollArea className="min-h-0 flex-1">
					<div className="flex flex-col p-2">
						{candidates.length === 0 ? <p className="p-4 text-center text-sm text-muted-foreground">还没有用户。</p> : null}
						{candidates.map((candidate) => {
							const id = candidate.id;
							return (
								<label className="flex cursor-pointer items-center gap-3 rounded-lg px-4 py-2.5 hover:bg-muted" key={id}>
									<Checkbox
										checked={selected.has(id)}
										disabled={saving}
										onCheckedChange={(checked) =>
											setSelected((current) => {
												const next = new Set(current);
												if (checked) next.add(id);
												else next.delete(id);
												return next;
											})
										}
									/>
									<UserRoundIcon className="size-3.5 text-muted-foreground" />
									<span className="flex min-w-0 flex-1 flex-col">
										<span className="truncate text-sm font-medium">{candidate.name}</span>
										{candidate.current ? <span className="truncate text-xs text-faint">当前：{candidate.current}</span> : null}
									</span>
									{!candidate.enabled ? <Tag>已停用</Tag> : null}
								</label>
							);
						})}
					</div>
				</ScrollArea>
				<DialogFooter className="m-0 rounded-none px-6 py-4">
					<DialogClose asChild>
						<Button disabled={saving} type="button" variant="outline">
							取消
						</Button>
					</DialogClose>
					<Button disabled={saving} onClick={() => void save()} type="button">
						{saving ? <Spinner data-icon="inline-start" /> : null}
						保存
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
