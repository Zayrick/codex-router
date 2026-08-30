import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import StatusUsage from "./StatusUsage";
import "./index.css";

const root = document.getElementById("root");
if (!root) throw new Error("Missing React root element.");

// The Rust router intentionally serves the same shell at both exact page routes.
const usageStatusPath = window.location.pathname === "/status/usage";
if (usageStatusPath) document.title = "Codex 用量状态";
const application = usageStatusPath ? <StatusUsage /> : <App />;

createRoot(root).render(
	<StrictMode>
		{application}
	</StrictMode>,
);
