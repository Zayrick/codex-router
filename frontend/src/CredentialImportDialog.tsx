import { useState, type FormEvent } from "react";
import { FileJsonIcon, KeyRoundIcon, TriangleAlertIcon, UploadIcon } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
	Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { CodexCredentialImport } from "./admin-api";

const MAX_IMPORT_BYTES = 64 * 1024;
const MAX_TOKEN_LENGTH = 16 * 1024;
const JSON_EXAMPLE = '{\n  "tokens": {\n    "access_token": "…",\n    "refresh_token": "…",\n    "id_token": "…",\n    "account_id": "…"\n  }\n}';

export default function CredentialImportDialog({
	onCancel,
	onImport,
}: {
	onCancel: () => void;
	onImport: (value: CodexCredentialImport) => Promise<void>;
}) {
	const [mode, setMode] = useState<"json" | "manual">("json");
	const [name, setName] = useState("");
	const [source, setSource] = useState("");
	const [refreshToken, setRefreshToken] = useState("");
	const [accessToken, setAccessToken] = useState("");
	const [idToken, setIdToken] = useState("");
	const [accountId, setAccountId] = useState("");
	const [email, setEmail] = useState("");
	const [expiresAt, setExpiresAt] = useState("");
	const [reading, setReading] = useState(false);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const busy = reading || saving;
	const ready = mode === "json" ? source.trim().length > 0 : refreshToken.trim().length > 0;

	async function readFile(file: File): Promise<void> {
		setError(null);
		setSource("");
		if (file.size > MAX_IMPORT_BYTES) {
			setError("JSON 文件不能超过 64 KiB。");
			return;
		}
		setReading(true);
		try {
			const contents = (await file.text()).replace(/^\uFEFF/, "");
			parseCredentialJson(contents);
			setSource(contents);
		} catch (error) {
			setError(error instanceof Error ? error.message : "无法读取文件，请重新选择 JSON 文件。");
		} finally {
			setReading(false);
		}
	}

	async function submit(event: FormEvent<HTMLFormElement>): Promise<void> {
		event.preventDefault();
		if (busy || !ready) return;
		setError(null);
		try {
			const credentials = mode === "json" ? parseCredentialJson(source) : {
				refresh_token: refreshToken.trim(),
				access_token: accessToken.trim() || undefined,
				id_token: idToken.trim() || undefined,
				account_id: accountId.trim() || undefined,
				email: email.trim() || undefined,
				expires_at: expiresAt ? new Date(expiresAt).toISOString() : undefined,
			};
			const value: CodexCredentialImport = { credentials };
			if (name.trim()) value.name = name.trim();
			if (new TextEncoder().encode(JSON.stringify(value)).byteLength > MAX_IMPORT_BYTES) {
				throw new Error("导入内容不能超过 64 KiB，请缩减 JSON 中的无关字段。");
			}
			setSaving(true);
			await onImport(value);
		} catch (error) {
			setError(error instanceof Error ? error.message : "导入凭据失败，请检查填写的信息。");
		} finally {
			setSaving(false);
		}
	}

	return (
		<Dialog open onOpenChange={(open) => { if (!open && !busy) onCancel(); }}>
			<DialogContent className="flex h-[min(40rem,calc(100svh-2rem))] flex-col gap-0 overflow-hidden p-0 sm:max-w-xl" showCloseButton={!busy}>
				<DialogHeader className="shrink-0 border-b px-4 py-4 pr-12 sm:px-6 sm:pr-14">
					<DialogTitle>导入凭据</DialogTitle>
					<DialogDescription className="sr-only">导入 Codex 账户。</DialogDescription>
				</DialogHeader>
				<form aria-busy={busy} className="flex min-h-0 flex-1 flex-col overflow-hidden" onSubmit={(event) => void submit(event)}>
					<ScrollArea className="h-full min-h-0 flex-1">
						<div className="grid gap-4 p-4 sm:p-6">
							<ToggleGroup aria-label="凭据导入方式" disabled={busy} onValueChange={(value) => {
								if (value === "json" || value === "manual") { setMode(value); setError(null); }
							}} type="single" value={mode} variant="outline">
								<ToggleGroupItem value="json"><FileJsonIcon />JSON 导入</ToggleGroupItem>
								<ToggleGroupItem value="manual"><KeyRoundIcon />手动填写</ToggleGroupItem>
							</ToggleGroup>
							<FieldGroup className="gap-4">
								<Field>
									<FieldLabel htmlFor="import-account-name">名称</FieldLabel>
									<Input disabled={busy} id="import-account-name" maxLength={100} onChange={(event) => setName(event.target.value)} placeholder="自动使用邮箱" value={name} />
								</Field>
								{mode === "json" ? (
									<>
										<Field>
											<FieldLabel htmlFor="import-credential-file">选择 JSON 文件</FieldLabel>
											<Input accept=".json,application/json" disabled={busy} id="import-credential-file" onChange={(event) => {
												const file = event.target.files?.[0];
												event.target.value = "";
												if (file) void readFile(file);
											}} type="file" />
											<FieldDescription>auth.json / Token JSON · 最大 64 KiB</FieldDescription>
										</Field>
										<Field>
											<FieldLabel htmlFor="import-credential-json">或粘贴 JSON</FieldLabel>
											<ScrollArea className="h-48 rounded-lg border border-input focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/50">
												<Textarea autoCapitalize="none" autoComplete="off" className="min-h-48 resize-none rounded-none border-0 font-mono text-xs focus-visible:ring-0" disabled={busy} id="import-credential-json" maxLength={MAX_IMPORT_BYTES} onChange={(event) => setSource(event.target.value)} placeholder={JSON_EXAMPLE} required spellCheck={false} value={source} />
											</ScrollArea>
											<FieldDescription>refresh_token 必填。</FieldDescription>
										</Field>
									</>
								) : (
									<>
										<TokenField disabled={busy} id="import-refresh-token" label="Refresh Token（必填）" onChange={setRefreshToken} required value={refreshToken} />
										<FieldDescription>其余字段可选，留空自动获取。</FieldDescription>
										<TokenField disabled={busy} id="import-access-token" label="Access Token" onChange={setAccessToken} value={accessToken} />
										<TokenField disabled={busy} id="import-id-token" label="ID Token" onChange={setIdToken} value={idToken} />
										<div className="grid gap-4 sm:grid-cols-2">
											<Field>
												<FieldLabel htmlFor="import-account-id">Account ID</FieldLabel>
												<Input autoCapitalize="none" autoComplete="off" disabled={busy} id="import-account-id" maxLength={256} onChange={(event) => setAccountId(event.target.value)} placeholder="自动识别失败时填写" spellCheck={false} value={accountId} />
											</Field>
											<Field>
												<FieldLabel htmlFor="import-email">邮箱</FieldLabel>
												<Input autoComplete="off" disabled={busy} id="import-email" maxLength={254} onChange={(event) => setEmail(event.target.value)} placeholder="优先从 Token 识别" type="email" value={email} />
											</Field>
										</div>
										<Field>
											<FieldLabel htmlFor="import-expires-at">到期时间（本地）</FieldLabel>
											<Input disabled={busy} id="import-expires-at" onChange={(event) => setExpiresAt(event.target.value)} type="datetime-local" value={expiresAt} />
										</Field>
									</>
								)}
							</FieldGroup>
							{error ? <Alert variant="destructive"><TriangleAlertIcon /><AlertDescription>{error}</AlertDescription></Alert> : null}
						</div>
					</ScrollArea>
					<DialogFooter className="m-0 shrink-0 rounded-none px-4 py-3 sm:px-6 sm:py-4">
						<DialogClose asChild><Button disabled={busy} type="button" variant="outline">取消</Button></DialogClose>
						<Button disabled={busy || !ready} type="submit">{busy ? <Spinner /> : <UploadIcon />}{saving ? "导入中…" : reading ? "读取文件…" : "导入账户"}</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}

function TokenField({ disabled, id, label, onChange, required = false, value }: {
	disabled: boolean;
	id: string;
	label: string;
	onChange: (value: string) => void;
	required?: boolean;
	value: string;
}) {
	return (
		<Field>
			<FieldLabel htmlFor={id}>{label}</FieldLabel>
			<Input autoCapitalize="none" autoComplete="off" disabled={disabled} id={id} maxLength={MAX_TOKEN_LENGTH} onChange={(event) => onChange(event.target.value)} required={required} spellCheck={false} type="password" value={value} />
		</Field>
	);
}

function parseCredentialJson(source: string): Record<string, unknown> {
	let parsed: unknown;
	try {
		parsed = JSON.parse(source.replace(/^\uFEFF/, ""));
	} catch {
		throw new Error("JSON 格式无效，请检查括号、引号和逗号。");
	}
	if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
		throw new Error("请提供单个账户的 JSON 对象。");
	}
	return parsed as Record<string, unknown>;
}
