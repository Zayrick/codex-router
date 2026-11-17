#!/usr/bin/env python3
"""LAN registration service backed by the Codex Router management API."""

import argparse
import fcntl
import hashlib
import http.cookies
import http.server
import ipaddress
import json
import os
from pathlib import Path
import secrets
import tempfile
import threading
import tomllib
import unicodedata
import urllib.error
import urllib.parse
import urllib.request


def normalize_name(value):
    if not isinstance(value, str) or any(unicodedata.category(c) == "Cc" for c in value):
        raise ValueError("Invalid name")
    name = value.strip()
    if not name or len(name) > 80 or len(name.encode("utf-16-le")) // 2 > 100:
        raise ValueError("Invalid name length")
    return name


def atomic_json(path, value):
    descriptor, temporary = tempfile.mkstemp(prefix=".users-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        descriptor = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


class RouterAPI:
    def __init__(self, url, origin, secret_file):
        parsed = urllib.parse.urlsplit(url)
        if parsed.scheme != "http" or not ipaddress.ip_address(parsed.hostname).is_loopback:
            raise ValueError("Router management URL must use loopback HTTP")
        self.url = url.rstrip("/") + "/admin/api/"
        self.origin = origin
        self.secret_file = Path(secret_file)
        self.cookie = None
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def _request(self, method, path, payload=None):
        headers = {"Origin": self.origin, "Content-Type": "application/json"}
        if self.cookie:
            headers["Cookie"] = self.cookie
        body = None if payload is None else json.dumps(payload).encode()
        request = urllib.request.Request(self.url + path, body, headers, method=method)
        with self.opener.open(request, timeout=30) as response:
            cookies = http.cookies.SimpleCookie()
            for header in response.headers.get_all("Set-Cookie", []):
                cookies.load(header)
            for name in ("__Host-codex-session", "codex-session"):
                if name in cookies:
                    self.cookie = name + "=" + cookies[name].value
            return json.load(response)

    def login(self):
        self.cookie = None
        self._request("POST", "session", {"credential": self.secret_file.read_text().strip()})
        if not self.cookie:
            raise RuntimeError("Router session cookie missing")

    def call(self, method, path, payload=None):
        if not self.cookie:
            self.login()
        try:
            return self._request(method, path, payload)
        except urllib.error.HTTPError as error:
            # A revoked or expired session requires a fresh administrator sign-in.
            if error.code != 401:
                raise
            error.close()
            self.login()
            return self._request(method, path, payload)


class Registry:
    def __init__(self, state_dir, api, group_id):
        state_dir = Path(state_dir)
        state_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.lock_file = (state_dir / "service.lock").open("a+")
        fcntl.flock(self.lock_file, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.path = state_dir / "users.json"
        self.users = json.loads(self.path.read_text()) if self.path.exists() else {}
        self.api = api
        self.group_id = group_id
        self.mutex = threading.Lock()

    def register(self, name):
        name = normalize_name(name)
        with self.mutex:
            state = self.api.call("GET", "state")
            if not any(g["id"] == self.group_id for g in state["accountGroups"]):
                raise RuntimeError("Registration account group missing")
            key = self.users.get(name)
            user = next((u for u in state["users"] if any(
                c["type"] == "api_key" and c["value"] == key for c in u["credentials"]
            )), None) if key else None
            if user and (not user["enabled"] or not any(
                c["value"] == key and c["enabled"] for c in user["credentials"]
            )):
                raise RuntimeError("Registration credential disabled")
            if not user:
                if any(u["name"] == name for u in state["users"]):
                    raise RuntimeError("Registration name already assigned")
                if not key:
                    key = "sk-" + secrets.token_hex(32)
                    updated = dict(self.users, **{name: key})
                    # Durable assignments let a retry recover a partially completed registration.
                    atomic_json(self.path, updated)
                    self.users = updated
                result = self.api.call("POST", "users", {
                    "name": name, "enabled": True,
                    "credentials": [{"type": "api_key", "value": key,
                                     "name": "Bootstrap", "enabled": True}],
                })
                user = next(u for u in result["users"] if any(
                    c["value"] == key for c in u["credentials"]
                ))
            routing = self.api.call("GET", "account-routing")
            if not any(r["userId"] == user["id"] for r in routing["routes"]):
                routing["routes"].append({"userId": user["id"], "targetType": "group",
                                          "targetId": self.group_id})
                self.api.call("PUT", "account-routing", {
                    "groups": routing["accountGroups"], "routes": routing["routes"],
                })
            return key


class Handler(http.server.BaseHTTPRequestHandler):
    server_version = "CodexBootstrap/1"

    def setup(self):
        super().setup()
        self.connection.settimeout(40)

    def log_message(self, format_string, *args):
        pass

    def send_body(self, status, body, content_type="text/plain; charset=utf-8"):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/healthz":
            self.send_body(200, b"ok\n")
        elif self.path in self.server.assets:
            content_type, body = self.server.assets[self.path]
            self.send_body(200, body, content_type)
        else:
            self.send_body(404, b"Not found\n")

    def do_POST(self):
        if self.path != "/register":
            self.send_body(404, b"Not found\n")
            return
        if self.headers.get_content_type() != "application/json" or "Origin" in self.headers:
            self.send_body(415, b"Use the installation script to register\n")
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            length = 0
        if not 0 < length <= 4096 or "Transfer-Encoding" in self.headers:
            self.send_body(413, b"Invalid request size\n")
            return
        try:
            payload = json.loads(self.rfile.read(length))
            if not isinstance(payload, dict) or set(payload) != {"name"}:
                raise ValueError("Expected name")
            name = normalize_name(payload["name"])
        except (ValueError, UnicodeError):
            self.send_body(400, b"Invalid registration name\n")
            return
        try:
            key = self.server.registry.register(name)
        except Exception as error:
            # Upstream error bodies and registry values contain credentials.
            print("Registration failed:", type(error).__name__, flush=True)
            self.send_body(503, b"Registration unavailable; contact the administrator\n")
            return
        self.send_body(200, json.dumps({"name": name, "api_key": key,
                                       "base_url": self.server.base_url}).encode(),
                       "application/json; charset=utf-8")


def build_server(config):
    api = RouterAPI(config["router_url"], config["router_origin"], config["secret_file"])
    registry = Registry(config["state_dir"], api, config["group_id"])
    assets = {}
    for platform, script, placeholder in [
        ("windows", "install.ps1", "@@CLIENT_SHA256@@"),
        ("macos", "install.sh", "@@MAC_CLIENT_SHA256@@"),
    ]:
        helper = Path(config[platform + "_helper"])
        binary = helper.read_bytes()
        text = Path(__file__).with_name(script).read_text(encoding="utf-8-sig")
        text = text.replace("@@PUBLIC_URL@@", config["public_url"].rstrip("/"))
        text = text.replace(placeholder, hashlib.sha256(binary).hexdigest())
        assets["/" + script] = ("text/plain; charset=utf-8", text.encode())
        assets["/" + helper.name] = ("application/octet-stream", binary)
    host, port = config["listen"].rsplit(":", 1)
    server = http.server.ThreadingHTTPServer((host, int(port)), Handler)
    server.registry, server.assets, server.base_url = registry, assets, config["api_base_url"]
    return server


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    with args.config.open("rb") as stream:
        server = build_server(tomllib.load(stream))
    print("Codex Router bootstrap listening on", server.server_address, flush=True)
    server.serve_forever()
