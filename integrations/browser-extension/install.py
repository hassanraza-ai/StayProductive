#!/usr/bin/env python3
"""Install the StayProductive native-messaging host for a browser.

This writes only per-user files: a copy of the host, a launcher, and the
browser's host manifest. It never uses sudo. The extension itself is loaded
unpacked; see the README for the two clicks that requires.

Chrome-family browsers need the unpacked extension's ID so the host can allow
exactly that origin. Firefox uses the fixed `stayproductive@local` ID.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import sys
from pathlib import Path

HOST_ID = "io.github.stayproductive.native"
FIREFOX_ID = "stayproductive@local"
CHROME_ID = re.compile(r"^[a-p]{32}$")
SOURCE = Path(__file__).resolve().parent / "native" / "stayproductive_native.py"

CHROME_DIRS = {
    "chrome": ("google-chrome",),
    "chromium": ("chromium",),
    "brave": ("BraveSoftware", "Brave-Browser"),
    "edge": ("microsoft-edge",),
}
BROWSERS = (*CHROME_DIRS, "firefox")


def home() -> Path:
    return Path(os.environ.get("HOME") or Path.home())


def config_home() -> Path:
    value = os.environ.get("XDG_CONFIG_HOME")
    return Path(value) if value else home() / ".config"


def data_home() -> Path:
    value = os.environ.get("XDG_DATA_HOME")
    return Path(value) if value else home() / ".local" / "share"


def install_dir() -> Path:
    return data_home() / "stayproductive" / "native"


def manifest_path(browser: str) -> Path:
    if browser == "firefox":
        return home() / ".mozilla" / "native-messaging-hosts" / f"{HOST_ID}.json"
    return config_home().joinpath(*CHROME_DIRS[browser], "NativeMessagingHosts", f"{HOST_ID}.json")


def normalise_ids(values: list[str] | None) -> list[str]:
    ids: list[str] = []
    for value in values or []:
        for item in value.split(","):
            item = item.strip()
            if item.startswith("chrome-extension://"):
                item = item[len("chrome-extension://") :]
            item = item.rstrip("/")
            if not item:
                continue
            if not CHROME_ID.match(item):
                raise SystemExit(
                    f"error: {item!r} is not a Chrome extension ID (32 letters a-p).\n"
                    "Open chrome://extensions with Developer mode on and copy the ID."
                )
            if item not in ids:
                ids.append(item)
    return ids


def launcher(path: Path, host: Path) -> str:
    return f"#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(host))} \"$@\"\n"


def install(browser: str, ids: list[str], dry_run: bool) -> list[Path]:
    directory = install_dir()
    host = directory / "stayproductive-native.py"
    runner = directory / "stayproductive-native"
    manifest = manifest_path(browser)
    written: list[Path] = []

    if dry_run:
        print(f"would install host: {host}")
        print(f"would install launcher: {runner}")
        print(f"would write manifest: {manifest}")
        return written

    directory.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(SOURCE, host)
    host.chmod(0o644)
    runner.write_text(launcher(directory, host), encoding="utf-8")
    runner.chmod(0o755)

    manifest.parent.mkdir(parents=True, exist_ok=True)
    if browser == "firefox":
        payload = {
            "name": HOST_ID,
            "description": "StayProductive local browser policy",
            "path": str(runner),
            "type": "stdio",
            "allowed_extensions": [FIREFOX_ID],
        }
    else:
        if not ids:
            raise SystemExit(
                "error: Chrome and Chromium need --extension-id with the unpacked "
                "extension's ID.\nLoad integrations/browser-extension/extension at "
                "chrome://extensions, copy its ID, then rerun."
            )
        payload = {
            "name": HOST_ID,
            "description": "StayProductive local browser policy",
            "path": str(runner),
            "type": "stdio",
            "allowed_origins": [f"chrome-extension://{value}/" for value in ids],
        }
    manifest.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    manifest.chmod(0o644)
    written.extend([host, runner, manifest])
    print(f"installed host for {browser}: {manifest}")
    return written


def uninstall(dry_run: bool) -> None:
    targets = [manifest_path(browser) for browser in BROWSERS]
    directory = install_dir()
    if dry_run:
        for target in targets:
            print(f"would remove: {target}")
        print(f"would remove: {directory}")
        return
    for target in targets:
        if target.exists():
            target.unlink()
            print(f"removed: {target}")
    if directory.exists():
        shutil.rmtree(directory)
        print(f"removed: {directory}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--browser",
        action="append",
        choices=BROWSERS,
        help="browser to configure; repeat for more than one (default: all)",
    )
    parser.add_argument(
        "--extension-id",
        action="append",
        help="Chrome-family unpacked extension ID; repeat or comma-separate",
    )
    parser.add_argument("--uninstall", action="store_true", help="remove the host files")
    parser.add_argument("--dry-run", action="store_true", help="print paths only")
    args = parser.parse_args(argv)

    if not SOURCE.is_file():
        print(f"error: host source is missing: {SOURCE}", file=sys.stderr)
        return 1
    if args.uninstall:
        uninstall(args.dry_run)
        return 0

    ids = normalise_ids(args.extension_id)
    browsers = args.browser or list(BROWSERS)
    if any(browser != "firefox" for browser in browsers) and not ids:
        print(
            "error: --extension-id is required for Chrome, Chromium, Brave, or Edge.\n"
            "Load the unpacked extension first and copy the ID from chrome://extensions.",
            file=sys.stderr,
        )
        return 2
    for browser in browsers:
        install(browser, ids, args.dry_run)

    if not args.dry_run:
        print()
        print("Next:")
        print("  Chromium: chrome://extensions -> Developer mode -> Load unpacked ->")
        print(f"            {Path(__file__).resolve().parent / 'extension'}")
        print("            (use manifest.json; the ID must match --extension-id)")
        print("  Firefox:  about:debugging#/runtime/this-firefox -> Load Temporary Add-on")
        print("            choose extension/manifest.firefox.json")
        print("  Restart the browser after installing or updating the host.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
