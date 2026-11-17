import { useState, type FormEvent } from "react";
import { EyeIcon, EyeOffIcon, TriangleAlertIcon } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Field, FieldLabel } from "@/components/ui/field";
import {
	InputGroup,
	InputGroupAddon,
	InputGroupButton,
	InputGroupInput,
} from "@/components/ui/input-group";
import { ProductMark } from "./ManagementShell";

interface LoginViewProps {
	error: string | null;
	loading: boolean;
	onSubmit: (credential: string) => void;
}

function LoginView({ error, loading, onSubmit }: LoginViewProps) {
	const [credential, setCredential] = useState("");
	const [visible, setVisible] = useState(false);

	function submit(event: FormEvent<HTMLFormElement>): void {
		event.preventDefault();
		if (credential && !loading) onSubmit(credential);
	}

	return (
		<div className="auth-shell">
			<aside className="auth-aside"><div className="auth-aside-title"><span>Codex</span><span>Router</span></div></aside>
			<div className="auth-main">
				<main className="auth-card">
					<div className="auth-mobile-brand"><ProductMark compact /><strong>Codex Router</strong></div>
					<h1>登录</h1>
					<p className="auth-description">输入 API Key、账户 ID 或管理密钥。登录状态会保存在此浏览器中。</p>
					{error ? <Alert className="mt-4" variant="destructive"><TriangleAlertIcon /><AlertDescription>{error}</AlertDescription></Alert> : null}
					<form action="/admin" className="auth-form" method="post" onSubmit={submit}>
						{/* Lets browser password managers offer to save and fill the credential. */}
						<input autoComplete="username" hidden name="username" readOnly type="text" value="codex-router" />
						<Field>
							<FieldLabel htmlFor="console-credential">访问凭据</FieldLabel>
							<InputGroup className="h-10">
								<InputGroupInput
									autoComplete="current-password"
									autoFocus
									disabled={loading}
									id="console-credential"
									name="password"
									onChange={(event) => setCredential(event.target.value)}
									required
									spellCheck={false}
									type={visible ? "text" : "password"}
									value={credential}
								/>
								<InputGroupAddon align="inline-end">
									<InputGroupButton aria-label={visible ? "隐藏" : "显示"} onClick={() => setVisible((value) => !value)} size="icon-sm">
										{visible ? <EyeOffIcon /> : <EyeIcon />}
									</InputGroupButton>
								</InputGroupAddon>
							</InputGroup>
						</Field>
						<Button className="mt-1 min-h-[3.05rem] w-full" disabled={loading || !credential} size="lg" type="submit">
							{loading ? "登录中…" : "登录"}
						</Button>
					</form>
				</main>
			</div>
		</div>
	);
}

export default LoginView;
