import { PlusIcon } from "lucide-react";

import { useAdmin } from "@/admin/context";
import { IdentityPage } from "@/admin/identities/identity-page";
import { Button } from "@/components/ui/button";

export default function ApiKeysPage() {
	return <IdentityPage kind="api_key" />;
}

export function PageAction() {
	const { openApiKeyEditor } = useAdmin();
	return (
		<Button onClick={() => openApiKeyEditor("new")} type="button">
			<PlusIcon data-icon="inline-start" />
			添加 API Key
		</Button>
	);
}
