import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import test from "node:test";
import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { createServer } from "vite";

test("credential dialogs and group IDs work with the crypto API exposed over HTTP", async () => {
	const server = await createServer({ server: { middlewareMode: true, hmr: false, watch: null } });
	const original = Object.getOwnPropertyDescriptor(globalThis, "crypto");
	try {
		const { UserEditor } = await server.ssrLoadModule("/src/admin/dialogs.tsx");
		const { newGroup, generateApiKey, validApiKey } = await server.ssrLoadModule("/src/admin/routing-model.ts");
		const { randomUuid } = await server.ssrLoadModule("/src/lib/uuid.ts");
		Object.defineProperty(globalThis, "crypto", {
			configurable: true,
			value: { getRandomValues: webcrypto.getRandomValues.bind(webcrypto) },
		});
		const uuidV4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
		assert.match(randomUuid(), uuidV4);
		assert.match(newGroup().id, uuidV4);
		assert.ok(validApiKey(generateApiKey()));
		assert.doesNotThrow(() => renderToString(createElement(UserEditor, {
			accounts: [], groups: [], entry: "new", loading: false, route: null,
			onCancel() {}, onSave() {},
		})));
	} finally {
		if (original) Object.defineProperty(globalThis, "crypto", original);
		else delete globalThis.crypto;
		await server.close();
	}
});
