import { PricingActions } from "@/admin/pricing/actions";
import { PricingView } from "@/admin/pricing/pricing-view";

export default function PricingPage() {
	return <PricingView />;
}

export function PageAction() {
	return <PricingActions />;
}
