import { PlusIcon } from "lucide-react";

import { useAdmin } from "@/admin/context";
import { IdentityPage } from "@/admin/identities/identity-page";
import { Button } from "@/components/ui/button";

export default function UsersPage() {
	return <IdentityPage />;
}

export function PageAction() {
	const { openUserEditor } = useAdmin();
	return (
		<Button onClick={() => openUserEditor("new")} type="button">
			<PlusIcon data-icon="inline-start" />
			添加用户
		</Button>
	);
}
