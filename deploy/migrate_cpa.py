#!/usr/bin/env python3
"""Create a private Router configuration from a stopped CLIProxyAPI installation."""

import argparse
import base64
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import secrets
import tomllib
import uuid

import yaml


def record_id(kind, value):
    seed = f"codex-router/record-id/v1\0cpa-{kind}\0{value}"
    raw = bytearray(hashlib.sha256(seed.encode()).digest()[:16])
    raw[6] = (raw[6] & 0x0F) | 0x80
    raw[8] = (raw[8] & 0x3F) | 0x80
    return str(uuid.UUID(bytes=bytes(raw)))


def scalar(value):
    if isinstance(value, dict):
        return "{ " + ", ".join(scalar(k) + " = " + scalar(v)
                                 for k, v in value.items() if v is not None) + " }"
    if isinstance(value, list):
        return "[" + ", ".join(scalar(v) for v in value) + "]"
    return json.dumps(value, ensure_ascii=False)


def table(lines, heading, values):
    lines.extend(["", heading])
    lines.extend(f"{key} = {scalar(value)}" for key, value in values.items() if value is not None)


def migrate(args):
    original = args.legacy_config.read_text()
    legacy = yaml.safe_load(original)
    keys = legacy.get("access", {}).get("api-keys", legacy.get("api-keys", []))
    if not isinstance(keys, list) or not keys or not all(isinstance(k, str) for k in keys):
        raise ValueError("Expected a nonempty client API key list")
    if len(keys) != len(set(keys)):
        raise ValueError("Duplicate legacy client keys")
    named = json.loads(args.bootstrap_users.read_text())
    if any(key not in keys for key in named.values()):
        raise ValueError("Bootstrap registry contains a key absent from CLIProxyAPI")
    if args.output.exists() and not args.replace:
        raise ValueError("Output exists; stop the Router and use --replace")
    args.secret_file.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if not args.secret_file.exists():
        with args.secret_file.open("x") as stream:
            stream.write(secrets.token_urlsafe(48) + "\n")
    secret = args.secret_file.read_text().strip()
    args.secret_file.chmod(0o600)
    if secret in keys:
        raise ValueError("Administrator secret must be distinct from client keys")

    lines = []
    table(lines, "[server]", {
        "bind": args.bind, "public_origin": args.public_origin, "cors_origin": "*",
        "maintenance_interval_seconds": 300,
        "trusted_proxies": ["127.0.0.1", "::1"],
        "log_filter": "codex_router=info,tower_http=info",
    })
    table(lines, "[admin]", {"secret": secret})
    table(lines, "[frontend]", {"dir": args.frontend_dir})
    table(lines, "[session]", {"database_path": "sessions.sqlite3"})
    table(lines, "[upstream]", {
        "chatgpt_proxy": args.socks_proxy,
        "codex_resets_url": "https://codex-resets.com/api/v1/status",
    })
    table(lines, "[usage_tracking]", {"database_path": "usage.sqlite3"})
    table(lines, "[public_account]", {"show_quota": True})

    accounts = []
    report = {"api_keys": len(keys), "bootstrap_users": len(named), "accounts": []}
    for path in sorted(args.auth_dir.glob("*.json")):
        auth = json.loads(path.read_text())
        if auth.get("type") != "codex":
            raise ValueError("Unsupported provider in " + path.name)
        token = auth["access_token"].split(".")[1]
        claims = json.loads(base64.urlsafe_b64decode(token + "=" * (-len(token) % 4)))
        account_id = auth.get("account_id") or claims["https://api.openai.com/auth"]["chatgpt_account_id"]
        identifier = record_id("oauth", account_id)
        if any(a["id"] == identifier for a in accounts):
            raise ValueError("Duplicate upstream account")
        name = auth.get("note") or auth.get("email") or path.stem
        if any(a["name"] == name for a in accounts):
            name += " " + identifier[:8]
        account = {"id": identifier, "name": name, "enabled": not auth.get("disabled", False)}
        accounts.append(account)
        table(lines, "[[state.account_routing.accounts]]", account)
        table(lines, '[state.codex_account_oauth."' + identifier + '"]', {
            "version": 1, "accessToken": auth["access_token"],
            "refreshToken": auth["refresh_token"], "idToken": auth.get("id_token"),
            "accountId": account_id, "email": auth.get("email"),
            "expiresAt": int(claims["exp"]) * 1000,
            "updatedAt": auth.get("last_refresh") or datetime.now(timezone.utc).isoformat(),
        })
        report["accounts"].append({"id": identifier, "source": path.name,
                                   "legacy_priority": auth.get("priority", 0)})
    if not accounts:
        raise ValueError("No Codex credentials found")
    group_id = record_id("group", "migrated-cpa")
    table(lines, "[[state.account_routing.groups]]", {
        "id": group_id, "name": "CPA migrated pool",
        "accountIds": [a["id"] for a in accounts], "strategy": "round-robin",
        "sessionAffinity": legacy.get("routing", {}).get("session-affinity", False),
        "sessionAffinityTtl": "1h",
    })
    names = set()
    for index, key in enumerate(keys, 1):
        name = next((name for name, value in named.items() if value == key), None)
        if not name:
            name = f"CPA key {index:02d}"
        if name in names:
            name += " " + str(index)
        names.add(name)
        identifier = record_id("user", key)
        table(lines, "[[state.users]]", {"id": identifier, "name": name, "enabled": True})
        table(lines, "[[state.users.credentials]]", {
            "id": record_id("key", key), "type": "api_key", "value": key,
            "name": "Bootstrap" if key in named.values() else "Migrated CLIProxyAPI",
            "enabled": True,
        })
        table(lines, "[[state.account_routing.routes]]", {
            "userId": identifier, "targetType": "group", "targetId": group_id,
        })
    if args.quota_from:
        previous = tomllib.loads(args.quota_from.read_text())
        identifiers = {account["id"] for account in accounts}
        for identifier, usage in previous.get("state", {}).get("account_usage", {}).items():
            if identifier in identifiers:
                table(lines, '[state.account_usage."' + identifier + '"]', usage)
    content = "\n".join(lines) + "\n"
    parsed = tomllib.loads(content)
    assert len(parsed["state"]["users"]) == len(keys)
    assert len(parsed["state"]["codex_account_oauth"]) == len(accounts)
    args.output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    args.output.write_text(content)
    args.output.chmod(0o600)
    report["group_id"] = group_id
    args.output.with_suffix(".migration.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"users": len(keys), "oauth_accounts": len(accounts),
                      "bootstrap_users": len(named), "group_id": group_id}))


if __name__ == "__main__":
    os.umask(0o077)
    parser = argparse.ArgumentParser()
    parser.add_argument("--legacy-config", type=Path, default=Path("/etc/cliproxyapi/config.yaml"))
    parser.add_argument("--auth-dir", type=Path, default=Path("/var/lib/cliproxyapi/auths"))
    parser.add_argument("--bootstrap-users", type=Path, default=Path("/var/lib/codex-bootstrap/users.json"))
    parser.add_argument("--secret-file", type=Path, default=Path("/etc/codex-router/admin-secret"))
    parser.add_argument("--output", type=Path, default=Path("/var/lib/codex-router/config.toml"))
    parser.add_argument("--bind", default="0.0.0.0:8317")
    parser.add_argument("--public-origin", required=True)
    parser.add_argument("--frontend-dir", default="/opt/codex-router/current/web")
    parser.add_argument("--socks-proxy")
    parser.add_argument("--quota-from", type=Path)
    parser.add_argument("--replace", action="store_true")
    migrate(parser.parse_args())
