"""Installer tests. Every path is redirected into a temporary HOME."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
INSTALLER = ROOT / "integrations" / "browser-extension" / "install.py"
EXTENSION_ID = "abcdefghijklmnopabcdefghijklmnop"


class InstallerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        self.home = root / "home"
        self.config = root / "config"
        self.data = root / "data"
        self.home.mkdir()
        self.env = os.environ.copy()
        self.env.update(
            {
                "HOME": str(self.home),
                "XDG_CONFIG_HOME": str(self.config),
                "XDG_DATA_HOME": str(self.data),
            }
        )

    def tearDown(self) -> None:
        self.temp.cleanup()

    def run_installer(self, *arguments: str):
        return subprocess.run(
            [sys.executable, str(INSTALLER), *arguments],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=self.env,
            timeout=20,
            text=True,
        )

    def manifest(self, browser: str) -> dict:
        if browser == "firefox":
            path = self.home / ".mozilla" / "native-messaging-hosts"
        else:
            path = self.config / "google-chrome" / "NativeMessagingHosts"
        return json.loads((path / "io.github.stayproductive.native.json").read_text())

    def test_chrome_requires_an_extension_id(self) -> None:
        result = self.run_installer("--browser", "chrome")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("--extension-id", result.stderr)
        self.assertFalse((self.config / "google-chrome").exists())

    def test_it_installs_the_host_and_an_exact_origin_for_chrome(self) -> None:
        result = self.run_installer(
            "--browser", "chrome", "--extension-id", f"chrome-extension://{EXTENSION_ID}/"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = self.manifest("chrome")
        self.assertEqual(manifest["name"], "io.github.stayproductive.native")
        self.assertEqual(manifest["type"], "stdio")
        self.assertEqual(
            manifest["allowed_origins"], [f"chrome-extension://{EXTENSION_ID}/"]
        )
        runner = Path(manifest["path"])
        self.assertTrue(runner.is_file())
        self.assertTrue(os.access(runner, os.X_OK))
        host = runner.parent / "stayproductive-native.py"
        self.assertTrue(host.is_file())
        self.assertIn("browser_decision", host.read_text())

    def test_firefox_uses_the_fixed_gecko_id(self) -> None:
        result = self.run_installer("--browser", "firefox")
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = self.manifest("firefox")
        self.assertEqual(manifest["allowed_extensions"], ["stayproductive@local"])
        self.assertNotIn("allowed_origins", manifest)
        self.assertTrue(Path(manifest["path"]).is_file())

    def test_a_dry_run_writes_nothing(self) -> None:
        result = self.run_installer(
            "--browser", "firefox", "--extension-id", EXTENSION_ID, "--dry-run"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("would write manifest", result.stdout)
        self.assertFalse((self.home / ".mozilla").exists())
        self.assertFalse((self.data / "stayproductive").exists())

    def test_uninstall_removes_only_the_installed_files(self) -> None:
        self.run_installer("--browser", "chrome", "--extension-id", EXTENSION_ID)
        self.run_installer("--browser", "firefox")
        result = self.run_installer("--uninstall")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(
            (self.config / "google-chrome" / "NativeMessagingHosts" / "io.github.stayproductive.native.json").exists()
        )
        self.assertFalse(
            (self.home / ".mozilla" / "native-messaging-hosts" / "io.github.stayproductive.native.json").exists()
        )
        self.assertFalse((self.data / "stayproductive" / "native").exists())

    def test_an_invalid_extension_id_is_rejected(self) -> None:
        result = self.run_installer(
            "--browser", "chrome", "--extension-id", "not-an-id"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not a Chrome extension ID", result.stderr)


if __name__ == "__main__":
    unittest.main()
