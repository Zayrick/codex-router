// Must be the first import of the entry point. The production CSP only allows
// <style> elements that carry the page nonce, while libraries (sonner,
// react-remove-scroll, chart styles) create them at runtime. Give every
// runtime <style> element the nonce Vite exposes in <meta property="csp-nonce">.

import { cspNonce } from "@/lib/csp";

const nonce = cspNonce();

if (nonce) {
	(globalThis as { __webpack_nonce__?: string }).__webpack_nonce__ = nonce;
	const createElement = Document.prototype.createElement;
	Document.prototype.createElement = function (this: Document, tagName: string, options?: ElementCreationOptions) {
		const element = createElement.call(this, tagName, options);
		if (element instanceof HTMLStyleElement) element.nonce = nonce;
		return element;
	} as typeof Document.prototype.createElement;
}
