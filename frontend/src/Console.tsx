import { useCallback, useEffect, useState } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import App from "./App";
import LoginView from "./LoginView";
import { ProductMark } from "./ManagementShell";
import { fetchSession, signIn, type ConsoleSession } from "./session-api";
import UserConsole from "./user/UserConsole";

type ConsoleState =
	| { status: "loading" }
	| { status: "signed-out"; error: string | null }
	| { status: "signed-in"; session: ConsoleSession };

/** Single entry point at /admin: the server-side session decides which view is shown. */
function Console() {
	const [state, setState] = useState<ConsoleState>({ status: "loading" });
	const [signingIn, setSigningIn] = useState(false);

	useEffect(() => {
		let active = true;
		fetchSession()
			.then((session) => {
				if (active) setState(session ? { status: "signed-in", session } : { status: "signed-out", error: null });
			})
			.catch((error: unknown) => {
				if (active) setState({ status: "signed-out", error: errorText(error, "暂时无法读取登录状态，请稍后重试。") });
			});
		return () => {
			active = false;
		};
	}, []);

	useEffect(() => {
		if (state.status === "signed-out") document.title = "登录 · Codex Router";
		else if (state.status === "signed-in" && state.session.role === "user") document.title = "用量信息 · Codex Router";
	}, [state]);

	// A view ends when its session is gone or no longer matches its role. Ask the
	// server which session (if any) the browser holds now, since another tab may
	// have signed in with a different credential.
	const handleSignedOut = useCallback((error: string | null = null) => {
		setState({ status: "loading" });
		fetchSession()
			.then((session) => setState(session ? { status: "signed-in", session } : { status: "signed-out", error }))
			.catch(() => setState({ status: "signed-out", error }));
	}, []);

	async function handleSignIn(credential: string): Promise<void> {
		if (signingIn) return;
		setSigningIn(true);
		try {
			const session = await signIn(credential);
			setState({ status: "signed-in", session });
		} catch (error) {
			setState({ status: "signed-out", error: errorText(error, "登录失败，请稍后重试。") });
		} finally {
			setSigningIn(false);
		}
	}

	if (state.status === "loading") {
		return <div className="loading-screen"><ProductMark /><Spinner /><strong>加载中…</strong></div>;
	}
	if (state.status === "signed-out") {
		return (
			<ScrollArea className="h-svh">
				<LoginView error={state.error} loading={signingIn} onSubmit={(credential) => void handleSignIn(credential)} />
			</ScrollArea>
		);
	}
	return state.session.role === "admin"
		? <App onSignedOut={handleSignedOut} />
		: <UserConsole onSignedOut={handleSignedOut} />;
}

function errorText(error: unknown, fallback: string): string {
	return error instanceof Error && error.message ? error.message : fallback;
}

export default Console;
