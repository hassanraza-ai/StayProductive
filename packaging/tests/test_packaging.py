"""Packaging layout and metadata checks that need no package toolchain."""

from __future__ import annotations

import configparser
import re
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PACKAGING = ROOT / "packaging"
APP_ID = "io.github.stayproductive.StayProductive"


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version = "([^"]+)"', text, re.MULTILINE)
    assert match, "workspace version is missing"
    return match.group(1)


class DesktopEntryTest(unittest.TestCase):
    def setUp(self) -> None:
        self.path = PACKAGING / "desktop" / f"{APP_ID}.desktop"
        self.parser = configparser.ConfigParser(interpolation=None)
        self.parser.optionxform = str  # type: ignore[assignment]
        self.parser.read(self.path, encoding="utf-8")

    def test_it_is_a_non_terminal_application_with_our_icon_and_launcher(self) -> None:
        entry = self.parser["Desktop Entry"]
        self.assertEqual(entry["Type"], "Application")
        self.assertEqual(entry["Exec"], "stayproductive")
        self.assertEqual(entry["Icon"], APP_ID)
        self.assertEqual(entry["Terminal"], "false")
        self.assertIn("Utility", entry["Categories"].split(";"))

    def test_the_desktop_file_validates_when_the_tool_is_available(self) -> None:
        tool = shutil_which("desktop-file-validate")
        if tool is None:
            self.skipTest("desktop-file-validate is not installed")
        result = subprocess.run(
            [tool, str(self.path)], capture_output=True, text=True, timeout=20
        )
        self.assertEqual(result.returncode, 0, result.stderr or result.stdout)


class AppStreamTest(unittest.TestCase):
    def setUp(self) -> None:
        self.path = PACKAGING / "metainfo" / f"{APP_ID}.metainfo.xml"
        self.tree = ET.parse(self.path)
        self.root = self.tree.getroot()

    def test_id_launchable_and_license_agree_with_the_desktop_entry(self) -> None:
        self.assertEqual(self.root.findtext("id"), APP_ID)
        launchable = self.root.find("launchable")
        assert launchable is not None
        self.assertEqual(launchable.text, f"{APP_ID}.desktop")
        self.assertEqual(self.root.findtext("project_license"), "Apache-2.0")
        self.assertEqual(self.root.findtext("metadata_license"), "CC0-1.0")

    def test_the_release_matches_the_cargo_version(self) -> None:
        release = self.root.find("releases/release")
        assert release is not None
        self.assertEqual(release.get("version"), workspace_version())
        self.assertRegex(release.get("date", ""), r"^\d{4}-\d{2}-\d{2}$")


class IconTest(unittest.TestCase):
    def test_the_icon_is_self_contained_svg(self) -> None:
        path = PACKAGING / "icons" / "hicolor" / "scalable" / "apps" / f"{APP_ID}.svg"
        root = ET.parse(path).getroot()
        self.assertTrue(root.tag.endswith("svg"))
        self.assertEqual(root.get("viewBox"), "0 0 128 128")
        for element in root.iter():
            for attribute in ("href", "{http://www.w3.org/1999/xlink}href"):
                value = element.get(attribute, "")
                self.assertFalse(
                    value.startswith(("http:", "https:", "file:")),
                    f"icon must not reference {value}",
                )


class ServiceUnitTest(unittest.TestCase):
    def test_the_unit_runs_the_daemon_socket_with_restart_and_no_new_privileges(
        self,
    ) -> None:
        text = (PACKAGING / "systemd" / "stayproductive.service.in").read_text(
            encoding="utf-8"
        )
        self.assertIn("ExecStart=@BINDIR@/stayproductive-daemon --socket", text)
        self.assertIn("Restart=always", text)
        self.assertIn("NoNewPrivileges=yes", text)
        self.assertIn("WantedBy=default.target", text)
        self.assertIn("PassEnvironment=DISPLAY XAUTHORITY", text)


class InstallLayoutTest(unittest.TestCase):
    def test_launchers_use_the_installed_module_directory(self) -> None:
        makefile = (PACKAGING / "Makefile").read_text(encoding="utf-8")
        self.assertIn("s|@LIBDIR@|$(LIBDIR)|g", makefile)
        self.assertIn("s|@BINDIR@|$(BINDIR)|g", makefile)
        ui = PACKAGING / "launcher" / "stayproductive.in"
        self.assertIn('@LIBDIR@', ui.read_text(encoding="utf-8"))
        x11 = PACKAGING / "launcher" / "stayproductive-x11.in"
        self.assertIn('"$@"', x11.read_text(encoding="utf-8"))

    def test_install_stages_every_documented_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run(
                [
                    "make",
                    "-C",
                    str(PACKAGING),
                    "install",
                    f"DESTDIR={directory}",
                    "PREFIX=/usr",
                ],
                capture_output=True,
                text=True,
                timeout=600,
            )
            self.assertEqual(result.returncode, 0, result.stderr or result.stdout)
            stage = Path(directory)
            expected = [
                "usr/bin/stayproductive",
                "usr/bin/stayproductive-daemon",
                "usr/bin/stayproductive-x11",
                "usr/lib/stayproductive/stayproductive/__main__.py",
                "usr/lib/stayproductive/stayproductive/style.css",
                "usr/lib/systemd/user/stayproductive.service",
                "usr/share/applications/io.github.stayproductive.StayProductive.desktop",
                "usr/share/icons/hicolor/scalable/apps/io.github.stayproductive.StayProductive.svg",
                "usr/share/metainfo/io.github.stayproductive.StayProductive.metainfo.xml",
                "usr/share/stayproductive/gnome-extension/metadata.json",
                "usr/share/stayproductive/browser-extension/extension/manifest.json",
                "usr/share/doc/stayproductive/LICENSE",
            ]
            for relative in expected:
                path = stage / relative
                self.assertTrue(path.exists(), f"missing {relative}")
            launcher = (stage / "usr/bin/stayproductive").read_text(encoding="utf-8")
            self.assertIn("/usr/lib/stayproductive", launcher)
            self.assertNotIn("@LIBDIR@", launcher)
            unit = (stage / "usr/lib/systemd/user/stayproductive.service").read_text(
                encoding="utf-8"
            )
            self.assertIn("ExecStart=/usr/bin/stayproductive-daemon --socket", unit)
            self.assertFalse(
                list((stage / "usr/lib/stayproductive").rglob("__pycache__")),
                "bytecode caches must not be packaged",
            )


class DebianScriptTest(unittest.TestCase):
    def test_the_builder_declares_the_runtime_dependencies(self) -> None:
        text = (PACKAGING / "deb" / "build.sh").read_text(encoding="utf-8")
        self.assertIn("dpkg-deb --build --root-owner-group", text)
        for dependency in (
            "python3 (>= 3.10)",
            "python3-gi",
            "gir1.2-gtk-4.0",
            "gir1.2-adw-1",
            "systemd",
        ):
            self.assertIn(dependency, text)
        self.assertIn("VERSION=$(sed -n", text)


class VersionConsistencyTest(unittest.TestCase):
    def test_rpm_and_arch_versions_match_cargo(self) -> None:
        version = workspace_version()
        spec = (PACKAGING / "rpm" / "stayproductive.spec").read_text(encoding="utf-8")
        self.assertTrue(
            re.search(rf"^Version:\s+{re.escape(version)}$", spec, re.MULTILINE)
        )
        pkgbuild = (PACKAGING / "arch" / "PKGBUILD").read_text(encoding="utf-8")
        self.assertIn(f"pkgver={version}", pkgbuild)


class FlatpakManifestTest(unittest.TestCase):
    def setUp(self) -> None:
        import json

        path = PACKAGING / "flatpak" / f"{APP_ID}.json"
        self.manifest = json.loads(path.read_text(encoding="utf-8"))

    def test_it_targets_the_gnome_runtime_and_our_command(self) -> None:
        self.assertEqual(self.manifest["app-id"], APP_ID)
        self.assertEqual(self.manifest["runtime"], "org.gnome.Platform")
        self.assertEqual(self.manifest["command"], "stayproductive")
        self.assertIn(
            "org.freedesktop.Sdk.Extension.rust-stable",
            self.manifest["sdk-extensions"],
        )

    def test_it_does_not_ask_for_broad_host_access(self) -> None:
        arguments = self.manifest["finish-args"]
        self.assertNotIn("--filesystem=host", arguments)
        self.assertNotIn("--filesystem=home", arguments)
        self.assertIn("--socket=wayland", arguments)
        self.assertIn("--share=ipc", arguments)

    def test_it_installs_the_same_tree_as_the_other_packages(self) -> None:
        commands = self.manifest["modules"][0]["build-commands"]
        self.assertTrue(any("make -C packaging install PREFIX=/app" in c for c in commands))


def shutil_which(name: str) -> str | None:
    import shutil

    return shutil.which(name)


if __name__ == "__main__":
    unittest.main()
