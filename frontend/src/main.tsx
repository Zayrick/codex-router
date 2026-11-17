import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { TooltipProvider } from "@/components/ui/tooltip";
import Console from "./Console";
import "./index.css";

const root = document.getElementById("root");
if (!root) throw new Error("Missing React root element.");

createRoot(root).render(
	<StrictMode>
		<TooltipProvider>
			<Console />
		</TooltipProvider>
	</StrictMode>,
);
