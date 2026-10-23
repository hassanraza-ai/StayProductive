"""Discover installed desktop applications for the profile editor.

The rule engine identifies applications by a stable key (a desktop-entry id or
an executable basename). Normal users should never have to type that key, so the
UI lists the desktop entries installed for the current user and lets them choose
by display name.
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class DesktopApp:
    """One launchable application from an XDG desktop entry."""

    key: str
    name: str
    desktop_id: str
    icon: str = ""
    comment: str = ""


_ESCAPE = re.compile(r"\\([snt\\])")
_UNESCAPE = {"s": " ", "n": "\n", "t": "\t", "\\": "\\"}


def _unescape(value: str) -> str:
    return _ESCAPE.sub(lambda match: _UNESCAPE[match.group(1)], value)


def _localized(values: dict[str, str], key: str) -> str:
    value = values.get(key)
    if value:
        return value
    for candidate in ("Name[en]", "Name[en_US]"):
        if values.get(candidate):
            return values[candidate]
    for candidate, candidate_value in values.items():
        if candidate.startswith(f"{key}["):
            return candidate_value
    return ""


def _data_roots() -> list[Path]:
    home = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local" / "share")
    roots = [home]
    data_dirs = os.environ.get("XDG_DATA_DIRS")
    if data_dirs is None:
        # A minimal environment may not define XDG_DATA_DIRS; include the
        # conventional system locations and the Flatpak/Snap exports.
        data_dirs = "/usr/local/share:/usr/share"
        roots.extend(
            [
                Path.home() / ".local" / "share" / "flatpak" / "exports" / "share",
                Path("/var/lib/flatpak/exports/share"),
                Path("/var/lib/snapd/desktop"),
            ]
        )
    for part in data_dirs.split(":"):
        if part:
            roots.append(Path(part))
    unique: list[Path] = []
    seen: set[Path] = set()
    for root in roots:
        root = root.expanduser()
        if root in seen:
            continue
        seen.add(root)
        unique.append(root)
    return unique


def _parse_entry(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    in_entry = False
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return values
    for raw in lines:
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            in_entry = line[1:-1].strip() == "Desktop Entry"
            continue
        if not in_entry or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        if key and key not in values:
            values[key] = _unescape(value.strip())
    return values


def _normalize_key(value: str) -> str:
    key = value.strip().lower()
    if key.endswith(".desktop"):
        key = key[:-8]
    if not key or any(character.isspace() or ord(character) < 32 for character in key):
        return ""
    return key


def _shown_on_gnome(values: dict[str, str]) -> bool:
    only = values.get("OnlyShowIn", "")
    if only and "GNOME" not in only.split(";"):
        return False
    hidden_for = values.get("NotShowIn", "")
    return "GNOME" not in hidden_for.split(";")


def _desktop_id(path: Path, applications: Path) -> str:
    return str(path.relative_to(applications)).replace(os.sep, "-")


def installed_apps() -> list[DesktopApp]:
    """Return visible desktop applications, ordered by display name.

    User entries win over system entries with the same desktop id. Entries with
    the same rule key are collapsed so the chooser never shows an ambiguous
    duplicate.
    """

    found: dict[str, DesktopApp] = {}
    seen_ids: set[str] = set()
    for root in _data_roots():
        applications = root / "applications"
        if not applications.is_dir():
            continue
        try:
            paths = sorted(applications.rglob("*.desktop"))
        except OSError:
            continue
        for path in paths:
            if not path.is_file():
                continue
            desktop_id = _desktop_id(path, applications)
            if desktop_id in seen_ids:
                continue
            seen_ids.add(desktop_id)
            values = _parse_entry(path)
            if values.get("Type", "Application") != "Application":
                continue
            if values.get("Hidden", "false").lower() == "true":
                continue
            if values.get("NoDisplay", "false").lower() == "true":
                continue
            if not _shown_on_gnome(values):
                continue
            name = _localized(values, "Name")
            if not name:
                continue
            fallback = _normalize_key(desktop_id)
            key = _normalize_key(values.get("StartupWMClass", "")) or fallback
            if not key:
                continue
            found.setdefault(
                key,
                DesktopApp(
                    key=key,
                    name=name,
                    desktop_id=desktop_id,
                    icon=values.get("Icon", ""),
                    comment=_localized(values, "Comment"),
                ),
            )
    return sorted(found.values(), key=lambda app: (app.name.casefold(), app.key))
