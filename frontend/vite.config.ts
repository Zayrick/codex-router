import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { fileURLToPath, URL } from "node:url";

const localBackendOrigin = /^https?:\/\/(?:localhost|127\.0\.0\.1|\[::1\])(?::\d+)?$/;

export default defineConfig(({ command }) => ({
	appType: "custom",
	// The Rust server serves built files below /admin/assets; the dev shell loads from the Vite origin root.
	base: command === "build" ? "/admin/" : "/",
	clearScreen: false,
	html: {
		// Rust replaces this build-time value with a fresh nonce per request.
		cspNonce: "__CODEX_ROUTER_CSP_NONCE__",
	},
	plugins: [react(), tailwindcss()],
	resolve: {
		alias: {
			"@": fileURLToPath(new URL("./src", import.meta.url)),
		},
	},
	server: {
		cors: { origin: localBackendOrigin },
		host: "127.0.0.1",
		port: 5173,
		strictPort: true,
		watch: {
			interval: 300,
			usePolling: true,
		},
	},
	build: {
		outDir: "dist",
		emptyOutDir: true,
	},
}));
