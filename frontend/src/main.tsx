import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { TooltipProvider } from "@/components/ui/tooltip";
import AccountUsage from "./AccountUsage";
import App from "./App";
import "./index.css";

const root = document.getElementById("root");
if (!root) throw new Error("Missing React root element.");

// The Rust router serves one shell for the secret admin path and the root account lookup page.
const managementPath = /^\/[A-Za-z0-9_-]{1,128}\/admin\/?$/.test(window.location.pathname);
if (!managementPath) document.title = "用量信息 · Codex Router";
const application = managementPath ? <App /> : <AccountUsage />;

createRoot(root).render(
	<StrictMode>
		<TooltipProvider>{application}</TooltipProvider>
	</StrictMode>,
);
