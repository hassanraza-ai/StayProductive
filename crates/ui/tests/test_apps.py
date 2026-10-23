"""Tests for desktop-entry discovery; no GTK or compositor required."""

from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from stayproductive.apps import installed_apps


class InstalledAppsTest(unittest.TestCase):
    def setUp(self) -> None:
        self._temp = tempfile.TemporaryDirectory()
        root = Path(self._temp.name)
        self.home = root / "home"
        self.user_data = self.home / ".local" / "share"
        self.system_data = root / "system"
        self.user_apps = self.user_data / "applications"
        self.system_apps = self.system_data / "applications"
        self.user_apps.mkdir(parents=True)
        self.system_apps.mkdir(parents=True)

    def tearDown(self) -> None:
        self._temp.cleanup()

    def write(self, directory: Path, name: str, body: str) -> None:
        (directory / name).write_text(body, encoding="utf-8")

    def scan(self):
        environment = {
            "HOME": str(self.home),
            "XDG_DATA_HOME": str(self.user_data),
            "XDG_DATA_DIRS": str(self.system_data),
        }
        with patch.dict(os.environ, environment, clear=False):
            return installed_apps()

    def test_lists_visible_apps_with_friendly_names_and_stable_keys(self) -> None:
        self.write(
            self.user_apps,
            "org.example.Editor.desktop",
            """[Desktop Entry]
Type=Application
Name=Example Editor
Comment=Write things
Icon=example-editor
StartupWMClass=ExampleEditor
""",
        )
        self.write(
            self.system_apps,
            "org.example.Game.desktop",
            """[Desktop Entry]
Type=Application
Name=Example Game
Exec=example-game
""",
        )
        apps = self.scan()
        self.assertEqual([app.name for app in apps], ["Example Editor", "Example Game"])
        self.assertEqual(apps[0].key, "exampleeditor")
        self.assertEqual(apps[1].key, "org.example.game")
        self.assertEqual(apps[0].comment, "Write things")

    def test_hidden_nodisplay_and_wrong_desktop_entries_are_skipped(self) -> None:
        self.write(
            self.user_apps,
            "hidden.desktop",
            "[Desktop Entry]\nType=Application\nName=Hidden\nHidden=true\n",
        )
        self.write(
            self.user_apps,
            "nodisplay.desktop",
            "[Desktop Entry]\nType=Application\nName=No Display\nNoDisplay=true\n",
        )
        self.write(
            self.user_apps,
            "kde.desktop",
            "[Desktop Entry]\nType=Application\nName=KDE Only\nOnlyShowIn=KDE;\n",
        )
        self.assertEqual(self.scan(), [])

    def test_user_entry_overrides_system_entry_with_the_same_id(self) -> None:
        body = "[Desktop Entry]\nType=Application\nName={}\n"
        self.write(self.user_apps, "org.example.App.desktop", body.format("User App"))
        self.write(self.system_apps, "org.example.App.desktop", body.format("System App"))
        apps = self.scan()
        self.assertEqual(len(apps), 1)
        self.assertEqual(apps[0].name, "User App")

    def test_invalid_startup_class_falls_back_to_desktop_id(self) -> None:
        self.write(
            self.user_apps,
            "org.example.App.desktop",
            """[Desktop Entry]
Type=Application
Name=Example App
StartupWMClass=not a valid key
""",
        )
        self.assertEqual(self.scan()[0].key, "org.example.app")


if __name__ == "__main__":
    unittest.main()
