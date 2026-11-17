import { PlusIcon } from "lucide-react";

import { useAdmin } from "@/admin/context";
import { IdentityPage } from "@/admin/identities/identity-page";
import { Button } from "@/components/ui/button";

export default function DownstreamPage() {
	return <IdentityPage kind="auth_proxy" />;
}

export function PageAction() {
	const { openProxyEditor } = useAdmin();
	return (
		<Button onClick={() => openProxyEditor("new")} type="button">
			<PlusIcon data-icon="inline-start" />
			添加下游账户
		</Button>
	);
}
