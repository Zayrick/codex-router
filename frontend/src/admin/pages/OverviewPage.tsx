import { TriangleAlertIcon } from "lucide-react";

import { useAdmin } from "@/admin/context";
import { ActivityCard } from "@/admin/overview/activity";
import { AttentionCard, ResourcesCard } from "@/admin/overview/attention";
import { useOverviewData } from "@/admin/overview/data";
import { FailuresCard, WeekActivityCard, WeekHealthCard } from "@/admin/overview/history";
import { LiveConnectionsCard } from "@/admin/overview/live";
import { IdentitiesCard, QuotaRiskCard } from "@/admin/overview/quota";
import { KpiRow, StatusStrip } from "@/admin/overview/summary";
import { Alert, AlertDescription } from "@/components/ui/alert";

/** Figma "Admin v2 / 概览": today's traffic, health and items needing attention. */
export default function OverviewPage() {
	const { now } = useAdmin();
	const data = useOverviewData(now);

	return (
		<div className="flex flex-col gap-4">
			{data.error && !data.today ? (
				<Alert variant="destructive">
					<TriangleAlertIcon />
					<AlertDescription>{data.error}</AlertDescription>
				</Alert>
			) : null}
			<StatusStrip data={data} />
			<KpiRow data={data} />
			<LiveConnectionsCard />
			<div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
				<ActivityCard data={data} />
				<AttentionCard data={data} />
			</div>
			<div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
				<QuotaRiskCard />
				<IdentitiesCard data={data} />
			</div>
			<div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
				<FailuresCard data={data} />
				<ResourcesCard />
			</div>
			<div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
				<WeekActivityCard data={data} />
				<WeekHealthCard data={data} />
			</div>
		</div>
	);
}
