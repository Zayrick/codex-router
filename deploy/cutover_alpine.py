#!/usr/bin/env python3
"""Switch the prepared 18317/18320 instances to the existing CPA ports."""

from datetime import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pwd
import shutil
import sqlite3
import subprocess
import sys
import time
import tomllib
import urllib.request


LEGACY = ["codex-bootstrap", "cpa-usage-keeper", "codex-account-proxy", "cliproxyapi"]
NEW = ["codex-router-bootstrap", "codex-router"]
RELEASE = Path("/opt/codex-router/current")
COMPAT = Path("/etc/nginx/http.d/codex-router-compat.conf")
CRONTAB = Path("/etc/crontabs/root")


def run(*args, check=True):
    return subprocess.run(args, check=check)


def own_tree(path, username):
    user = pwd.getpwnam(username)
    for entry in [path, *path.rglob("*")]:
        os.chown(entry, user.pw_uid, user.pw_gid)


def main():
    if os.geteuid() != 0:
        raise PermissionError("Run as root")
    if Path("/etc/nginx/http.d/codex-router-console.conf").exists():
        raise RuntimeError("Use a release update for an installed LAN console gateway")
    os.umask(0o077)
    lock = Path("/run/codex-router-migration.lock").open("w")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    original = tomllib.loads(Path("/var/lib/codex-router/config.toml").read_text())
    if original["server"]["bind"] != "127.0.0.1:18317":
        raise ValueError("Expected the verified preflight instance on port 18317")
    for process in Path("/proc").glob("[0-9]*/cmdline"):
        try:
            arguments = process.read_bytes().split(b"\0")
        except FileNotFoundError:
            continue
        if b"/usr/local/sbin/cpa-stack-update" in arguments:
            raise RuntimeError("The CPA updater is running")

    backup = Path("/var/backups/codex-router-migration") / datetime.now().strftime("%Y%m%dT%H%M%S")
    backup.mkdir(parents=True, mode=0o700)
    shutil.copy2(CRONTAB, backup / "crontab")
    Path("/etc/codex-router/rollback-dir").write_text(str(backup) + "\n")
    preflight = Path("/etc/nginx/http.d/codex-router-compat-preflight.conf")
    if preflight.exists():
        preflight.rename(backup / preflight.name)
        run("nginx", "-t")
        run("rc-service", "nginx", "reload")
    CRONTAB.write_text("\n".join(line for line in CRONTAB.read_text().splitlines()
                                if "/usr/local/sbin/cpa-stack-update" not in line) + "\n")
    try:
        for service in [*LEGACY, *NEW]:
            run("rc-service", service, "stop")
        paths = ["etc/nginx", "etc/cliproxyapi", "etc/codex-account-proxy", "etc/cpa-usage-keeper",
                 "opt/cliproxyapi", "opt/cpa-usage-keeper", "opt/codex-account-proxy", "opt/codex-bootstrap",
                 "var/lib/cliproxyapi", "var/lib/cpa-usage-keeper", "var/lib/codex-account-proxy",
                 "var/lib/codex-bootstrap", "srv/codex-scripts", "usr/local/sbin/cpa-stack-update",
                 "etc/crontabs/root", "root/cpa-stack-credentials.txt"]
        for service in LEGACY:
            paths.append("etc/init.d/" + service)
            if Path("/etc/conf.d", service).exists():
                paths.append("etc/conf.d/" + service)
        for path in paths:
            if not Path("/", path).exists():
                raise FileNotFoundError(path)
        archive = backup / "legacy.tar.gz"
        run("tar", "-czf", str(archive), "-C", "/", *paths)
        subprocess.run(["tar", "-tzf", str(archive)], stdout=subprocess.DEVNULL, check=True)
        with archive.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        (backup / "SHA256SUMS").write_text(digest + "  legacy.tar.gz\n")
        database = sqlite3.connect("file:/var/lib/cpa-usage-keeper/app.db?mode=ro", uri=True)
        try:
            if database.execute("PRAGMA quick_check").fetchone() != ("ok",):
                raise RuntimeError("Keeper database verification failed")
        finally:
            database.close()
        print("VERIFIED BACKUP", backup, archive.stat().st_size, flush=True)

        runtime = Path("/var/lib/codex-router")
        shutil.move(runtime, backup / "preflight-router")
        runtime.mkdir(mode=0o700)
        run("python3", str(RELEASE / "deploy/migrate_cpa.py"),
            "--public-origin", original["server"]["public_origin"],
            "--socks-proxy", original["upstream"]["chatgpt_proxy"],
            "--quota-from", str(backup / "preflight-router/config.toml"))
        own_tree(runtime, "codex-router")
        bootstrap = Path("/var/lib/codex-router-bootstrap")
        shutil.move(bootstrap, backup / "preflight-bootstrap")
        bootstrap.mkdir(mode=0o700)
        shutil.copyfile("/var/lib/codex-bootstrap/users.json", bootstrap / "users.json")
        own_tree(bootstrap, "codex-router-bootstrap")
        settings_path = Path("/etc/codex-router/bootstrap.toml")
        settings = tomllib.loads(settings_path.read_text())
        settings["listen"] = "192.168.160.172:8320"
        settings["router_url"] = "http://127.0.0.1:8317"
        settings_path.write_text("\n".join(k + " = " + json.dumps(v) for k, v in settings.items()) + "\n")
        shutil.copyfile(RELEASE / "deploy/nginx-account-compat.conf", COMPAT)
        run("nginx", "-t")
        run("rc-service", "codex-router", "start")
        for attempt in range(20):
            try:
                with urllib.request.urlopen("http://127.0.0.1:8317/healthz", timeout=2) as response:
                    if response.status == 204:
                        break
            except OSError:
                if attempt == 19:
                    raise
                time.sleep(0.5)
        else:
            raise RuntimeError("Router health check failed")
        run("rc-service", "nginx", "reload")
        run("rc-service", "codex-router-bootstrap", "start")
        sys.path.insert(0, str(RELEASE / "deploy/bootstrap"))
        from server import RouterAPI
        api = RouterAPI("http://127.0.0.1:8317", original["server"]["public_origin"],
                        "/etc/codex-router/admin-secret")
        state = api.call("GET", "state")
        api.call("GET", "quota?refresh=1")
        expected = len(original["state"]["users"]) - sum(
            user["name"] == "__router_deployment_probe__" for user in original["state"]["users"])
        if len(state["users"]) < expected or len(state["codexAccounts"]) != len(original["state"]["codex_account_oauth"]):
            raise RuntimeError("Migration counts changed unexpectedly")
        for service in LEGACY:
            run("rc-update", "del", service, "default")
        for service in reversed(NEW):
            run("rc-update", "add", service, "default")
        print("CUTOVER COMPLETE", backup, flush=True)
    except BaseException:
        print("Cutover failed; restoring the legacy services", file=sys.stderr, flush=True)
        for service in NEW:
            run("rc-service", service, "stop", check=False)
            run("rc-update", "del", service, "default", check=False)
        if COMPAT.exists():
            COMPAT.rename(backup / "failed-compat.conf")
        run("nginx", "-t")
        run("rc-service", "nginx", "reload")
        shutil.copy2(backup / "crontab", CRONTAB)
        for service in reversed(LEGACY):
            run("rc-update", "add", service, "default")
            run("rc-service", service, "start")
        raise


if __name__ == "__main__":
    main()
