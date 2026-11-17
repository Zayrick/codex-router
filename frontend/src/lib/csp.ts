// Nonce of the page's Content-Security-Policy (see Vite `html.cspNonce`).
// Elements created at runtime, such as chart <style> tags, must carry it.

let cached: string | undefined;

export function cspNonce(): string | undefined {
	if (cached === undefined && typeof document !== "undefined") {
		const meta = document.querySelector<HTMLMetaElement>('meta[property="csp-nonce"]');
		cached = meta?.nonce || meta?.getAttribute("nonce") || "";
	}
	return cached || undefined;
}
