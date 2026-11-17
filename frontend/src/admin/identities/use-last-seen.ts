import { AdminSessionExpiredError, type LastSeen } from "@/admin-api";
import { useAdmin } from "@/admin/context";
import { useResource, type Resource } from "@/admin/use-resource";

/** Last use of every model and identity across all recorded events. */
export function useLastSeen(): Resource<LastSeen> {
	const { api, reportError } = useAdmin();
	return useResource((signal) => api.getLastSeen(signal), [], {
		onSessionExpired: () => reportError(new AdminSessionExpiredError(), ""),
	});
}
