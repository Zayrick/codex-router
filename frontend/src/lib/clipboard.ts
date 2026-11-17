/**
 * Copies text to the clipboard. `navigator.clipboard` only exists in secure
 * contexts (HTTPS / localhost), so plain-HTTP deployments fall back to the
 * legacy `execCommand("copy")` path. Rejects when both methods fail.
 */
export async function copyToClipboard(text: string): Promise<void> {
	if (window.isSecureContext && navigator.clipboard?.writeText) {
		try {
			await navigator.clipboard.writeText(text);
			return;
		} catch {
			// Permission denied etc. — try the legacy path below.
		}
	}
	if (!legacyCopy(text)) throw new Error("clipboard unavailable");
}

function legacyCopy(text: string): boolean {
	const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
	// Mount next to the focused element so modal focus traps (Radix Dialog)
	// don't steal focus from the textarea before the copy happens.
	const host = active?.parentElement ?? document.body;
	const textarea = document.createElement("textarea");
	textarea.value = text;
	textarea.setAttribute("readonly", "");
	textarea.setAttribute("aria-hidden", "true");
	Object.assign(textarea.style, {
		position: "fixed",
		top: "0",
		left: "0",
		width: "1px",
		height: "1px",
		padding: "0",
		border: "0",
		opacity: "0",
		pointerEvents: "none",
	});
	host.appendChild(textarea);
	const selection = document.getSelection();
	const previousRange = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
	let ok = false;
	try {
		textarea.focus({ preventScroll: true });
		textarea.select();
		textarea.setSelectionRange(0, text.length);
		ok = document.execCommand("copy");
	} catch {
		ok = false;
	} finally {
		textarea.remove();
		if (previousRange && selection) {
			selection.removeAllRanges();
			selection.addRange(previousRange);
		}
		active?.focus({ preventScroll: true });
	}
	return ok;
}
