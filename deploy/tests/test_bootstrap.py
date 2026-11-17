import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import uuid

spec = importlib.util.spec_from_file_location("bootstrap", Path(__file__).parents[1] / "bootstrap/server.py")
bootstrap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bootstrap)


class FakeRouter:
    def __init__(self):
        self.users = []
        self.groups = [{"id": str(uuid.uuid4()), "name": "Shared pool"}]
        self.routes = []
        self.fail_route_once = False

    def call(self, method, path, payload=None):
        if method == "GET" and path == "state":
            return copy.deepcopy({"users": self.users, "accountGroups": self.groups})
        if method == "GET" and path == "account-routing":
            return copy.deepcopy({"accountGroups": self.groups, "routes": self.routes})
        if method == "POST" and path == "users":
            self.users.append(dict(payload, id=str(uuid.uuid4())))
            return copy.deepcopy({"users": self.users})
        if method == "PUT" and path == "account-routing":
            assert set(payload) == {"groups", "routes"}
            if self.fail_route_once:
                self.fail_route_once = False
                raise OSError("Router unavailable")
            self.groups, self.routes = copy.deepcopy((payload["groups"], payload["routes"]))
            return {}
        raise AssertionError((method, path))


class RegistrationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.api = FakeRouter()
        self.registry = bootstrap.Registry(self.directory.name, self.api, self.api.groups[0]["id"])
        self.addCleanup(lambda: self.registry.lock_file.close())

    def test_retry_after_route_failure_and_restart_reuses_key_and_user(self):
        self.api.fail_route_once = True
        with self.assertRaises(OSError):
            self.registry.register("Alice")
        key = json.loads(self.registry.path.read_text())["Alice"]
        self.registry.lock_file.close()
        self.registry = bootstrap.Registry(self.directory.name, self.api, self.api.groups[0]["id"])
        self.assertEqual(key, self.registry.register(" Alice "))
        self.assertEqual(1, len(self.api.users))
        self.assertEqual(1, len(self.api.routes))
        self.assertEqual(key, self.registry.register("Alice"))

    def test_existing_route_is_preserved_and_disabled_key_stays_disabled(self):
        self.registry.register("Alice")
        self.api.routes[0] = {"userId": self.api.users[0]["id"], "targetType": "account", "targetId": "custom"}
        self.registry.register("Alice")
        self.assertEqual("custom", self.api.routes[0]["targetId"])
        self.api.users[0]["credentials"][0]["enabled"] = False
        with self.assertRaises(RuntimeError):
            self.registry.register("Alice")
        self.assertFalse(self.api.users[0]["credentials"][0]["enabled"])

    def test_unregistered_name_cannot_claim_an_existing_user(self):
        self.api.users.append({"id": "existing", "name": "Alice", "enabled": True,
                               "credentials": [{"type": "api_key", "value": "sk-private", "enabled": True}]})
        with self.assertRaises(RuntimeError):
            self.registry.register("Alice")
        self.assertFalse(self.registry.path.exists())
        self.assertEqual(1, len(self.api.users))


if __name__ == "__main__":
    unittest.main()
