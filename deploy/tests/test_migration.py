import argparse
import base64
import importlib.util
import json
from pathlib import Path
import tempfile
import tomllib
import unittest
import uuid

import yaml


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).parents[1] / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


migration = load("migrate_cpa")
rollback = load("rollback_sync")


class MigrationTests(unittest.TestCase):
    def test_legacy_backend_uses_the_lan_console_gateway_after_rollback(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            legacy = root / "config.yaml"
            keeper = root / "keeper.env"
            legacy.write_text(yaml.safe_dump({"server": {"host": "0.0.0.0", "port": 8317},
                                              "access": {"api-keys": ["sk-existing-123"]}}))
            keeper.write_text("CPA_BASE_URL=http://127.0.0.1:8317\nCPA_PUBLIC_URL=https://api.example\n")
            rollback.configure_lan_listener(argparse.Namespace(legacy_config=legacy, keeper_env=keeper))
            actual = yaml.safe_load(legacy.read_text())
            self.assertEqual({"host": "127.0.0.1", "port": 18317}, actual["server"])
            self.assertEqual(["sk-existing-123"], actual["access"]["api-keys"])
            self.assertEqual("CPA_BASE_URL=http://127.0.0.1:18317\nCPA_PUBLIC_URL=https://api.example\n", keeper.read_text())

    def test_keys_oauth_and_named_users_survive_migration_and_rollback(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            auth_dir = root / "auths"
            auth_dir.mkdir()
            claims = base64.urlsafe_b64encode(json.dumps({"exp": 2000000000}).encode()).decode().rstrip("=")
            token = "header." + claims + ".signature"
            auth = {"type": "codex", "account_id": "account-1", "email": "a@example.com",
                    "access_token": token, "refresh_token": "refresh-old", "id_token": "id-old",
                    "priority": 2, "disabled": False}
            auth_path = auth_dir / "codex-a.json"
            auth_path.write_text(json.dumps(auth))
            key = "sk-test-123456789"
            legacy = root / "legacy.yaml"
            legacy.write_text(yaml.safe_dump({"access": {"api-keys": [key]}, "routing": {"session-affinity": True}}))
            users = root / "users.json"
            users.write_text(json.dumps({"Alice": key}))
            config = root / "config.toml"
            args = argparse.Namespace(legacy_config=legacy, auth_dir=auth_dir, bootstrap_users=users,
                output=config, replace=False, secret_file=root / "secret", bind="127.0.0.1:18317",
                public_origin="https://router.example", frontend_dir="/web", socks_proxy=None, quota_from=None)
            migration.migrate(args)
            state = tomllib.loads(config.read_text())["state"]
            self.assertEqual(key, state["users"][0]["credentials"][0]["value"])
            self.assertEqual("Alice", state["users"][0]["name"])
            self.assertEqual(8, uuid.UUID(state["users"][0]["id"]).version)
            self.assertEqual(state["users"][0]["id"], state["account_routing"]["routes"][0]["userId"])
            config.write_text(config.read_text().replace('refreshToken = "refresh-old"', 'refreshToken = "refresh-rotated"'))
            extra = []
            migration.table(extra, "[[state.users]]", {
                "id": str(uuid.uuid4()), "name": "Bob", "enabled": True,
            })
            migration.table(extra, "[[state.users.credentials]]", {
                "id": str(uuid.uuid4()), "type": "api_key", "value": "sk-new-123456789", "enabled": True,
            })
            config.write_text(config.read_text() + "\n".join(extra) + "\n")
            registered = root / "registered.json"
            registered.write_text(json.dumps({"Alice": key, "Bob": "sk-new-123456789"}))
            rollback.sync(argparse.Namespace(router_config=config, legacy_config=legacy, auth_dir=auth_dir,
                router_users=registered, legacy_users=users))
            restored = json.loads(auth_path.read_text())
            self.assertEqual("refresh-rotated", restored["refresh_token"])
            self.assertEqual(2, restored["priority"])
            self.assertEqual([key, "sk-new-123456789"], yaml.safe_load(legacy.read_text())["access"]["api-keys"])
            self.assertIn("Bob", json.loads(users.read_text()))


if __name__ == "__main__":
    unittest.main()
