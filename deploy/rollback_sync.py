#!/usr/bin/env python3
"""Export current Router keys and rotated OAuth credentials before a CPA rollback."""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import tempfile
import tomllib

import yaml


def atomic_write(path, text):
    previous = path.stat() if path.exists() else path.parent.stat()
    descriptor, temporary = tempfile.mkstemp(prefix=".router-rollback-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as stream:
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
            os.fchown(stream.fileno(), previous.st_uid, previous.st_gid)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def sync(args):
    router = tomllib.loads(args.router_config.read_text())
    state = router["state"]
    legacy = yaml.safe_load(args.legacy_config.read_text())
    keys = list(dict.fromkeys(c["value"] for u in state["users"] if u["enabled"]
                             for c in u["credentials"] if c["enabled"]))
    if not keys:
        raise ValueError("Router has no enabled client credentials")
    legacy.setdefault("access", {})["api-keys"] = keys
    existing = {}
    for path in args.auth_dir.glob("*.json"):
        value = json.loads(path.read_text())
        if value.get("type") == "codex":
            existing[value.get("account_id")] = (path, value)
    accounts = {a["id"]: a for a in state["account_routing"]["accounts"]}
    updated = 0
    for identifier, oauth in state.get("codex_account_oauth", {}).items():
        if identifier not in accounts:
            continue
        account = accounts[identifier]
        path, value = existing.get(oauth["accountId"], (
            args.auth_dir / ("codex-router-" + identifier + ".json"), {"type": "codex"}
        ))
        value.update({"access_token": oauth["accessToken"], "refresh_token": oauth["refreshToken"],
                      "account_id": oauth["accountId"], "email": oauth.get("email", ""),
                      "disabled": not account["enabled"], "note": account["name"],
                      "expired": datetime.fromtimestamp(oauth["expiresAt"] / 1000, timezone.utc).isoformat(),
                      "last_refresh": oauth["updatedAt"]})
        if oauth.get("idToken"):
            value["id_token"] = oauth["idToken"]
        atomic_write(path, json.dumps(value, ensure_ascii=False, indent=2) + "\n")
        updated += 1
    atomic_write(args.legacy_config, yaml.safe_dump(legacy, allow_unicode=True, sort_keys=False))
    if args.router_users.exists():
        registered = json.loads(args.router_users.read_text())
        original = json.loads(args.legacy_users.read_text())
        original.update(registered)
        atomic_write(args.legacy_users, json.dumps(original, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"exported_keys": len(keys), "exported_oauth_accounts": updated}))


def configure_lan_listener(args):
    legacy = yaml.safe_load(args.legacy_config.read_text())
    legacy.setdefault("server", {}).update({"host": "127.0.0.1", "port": 18317})
    atomic_write(args.legacy_config, yaml.safe_dump(legacy, allow_unicode=True, sort_keys=False))
    keeper = args.keeper_env.read_text()
    keeper = keeper.replace("http://127.0.0.1:8317", "http://127.0.0.1:18317")
    atomic_write(args.keeper_env, keeper)
    print("Legacy services use the loopback backend behind the LAN console gateway")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--router-config", type=Path, default=Path("/var/lib/codex-router/config.toml"))
    parser.add_argument("--legacy-config", type=Path, default=Path("/etc/cliproxyapi/config.yaml"))
    parser.add_argument("--auth-dir", type=Path, default=Path("/var/lib/cliproxyapi/auths"))
    parser.add_argument("--router-users", type=Path, default=Path("/var/lib/codex-router-bootstrap/users.json"))
    parser.add_argument("--legacy-users", type=Path, default=Path("/var/lib/codex-bootstrap/users.json"))
    parser.add_argument("--keeper-env", type=Path, default=Path("/etc/cpa-usage-keeper/keeper.env"))
    parser.add_argument("--configure-lan-listener", action="store_true")
    args = parser.parse_args()
    if args.configure_lan_listener:
        configure_lan_listener(args)
    else:
        sync(args)
