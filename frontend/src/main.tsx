import "./csp-bootstrap";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import Console from "./Console";
import "./index.css";

const root = document.getElementById("root");
if (!root) throw new Error("Missing React root element.");

createRoot(root).render(
	<StrictMode>
		<TooltipProvider>
			<Console />
			<Toaster position="bottom-right" />
		</TooltipProvider>
	</StrictMode>,
);
