import { RefreshCwIcon, SaveIcon } from "lucide-react";

import { useAdmin } from "@/admin/context";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";

import { useSavePrices } from "./use-save-prices";

/** Heading actions: 从 Models.dev 同步 + 保存价格. */
export function PricingActions() {
	const { syncPricing, pricingSyncing } = useAdmin();
	const { dirty, saving, save } = useSavePrices();
	return (
		<div className="flex flex-wrap gap-2">
			<Button
				disabled={pricingSyncing || dirty}
				onClick={() => void syncPricing()}
				title={dirty ? "请先保存或撤销修改" : undefined}
				type="button"
				variant="outline"
			>
				{pricingSyncing ? <Spinner data-icon="inline-start" /> : <RefreshCwIcon data-icon="inline-start" />}从 Models.dev 同步
			</Button>
			<Button disabled={!dirty || saving} onClick={() => void save()} type="button">
				{saving ? <Spinner data-icon="inline-start" /> : <SaveIcon data-icon="inline-start" />}保存价格
			</Button>
		</div>
	);
}
