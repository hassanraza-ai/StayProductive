"""Desktop integration helpers shared by Settings and onboarding.

Only the GNOME Shell extension is implemented in this build. Nothing here
enables anything by itself: callers must ask the user first, and every function
reports failure instead of silently degrading.
"""

from __future__ import annotations

import os
import shutil
from pathlib import Path

from gi.repository import Gio, GLib

GNOME_EXTENSION_UUID = "stayproductive@hassanraza.ai"
GNOME_SHELL_SCHEMA = "org.gnome.shell"


class DesktopError(RuntimeError):
    """A desktop capability could not be changed."""


def session_desktop_name() -> str:
    """Human-readable name of the current desktop session."""
    value = os.environ.get("XDG_CURRENT_DESKTOP") or os.environ.get(
        "XDG_SESSION_DESKTOP", ""
    )
    return value.split(":")[0].replace("-", " ").strip() or "this desktop"


def is_gnome() -> bool:
    desktops = " ".join(
        os.environ.get(name, "")
        for name in ("XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP")
    ).lower()
    return "gnome" in desktops or "GNOME_DESKTOP_SESSION_ID" in os.environ


def extension_source() -> Path | None:
    """Locate the bundled extension files, if this build ships them."""
    candidates = [
        parent / "integrations" / "gnome" / GNOME_EXTENSION_UUID
        for parent in Path(__file__).resolve().parents
    ]
    candidates.extend(
        [
            Path(GLib.get_user_data_dir())
            / "stayproductive"
            / "extension-source"
            / GNOME_EXTENSION_UUID,
            Path("/usr/share/stayproductive/gnome-extension"),
        ]
    )
    return next(
        (path for path in candidates if (path / "metadata.json").is_file()), None
    )


def extension_target() -> Path:
    return (
        Path(GLib.get_user_data_dir())
        / "gnome-shell"
        / "extensions"
        / GNOME_EXTENSION_UUID
    )


def extension_installed() -> bool:
    return (extension_target() / "metadata.json").is_file()


def extension_enabled() -> bool:
    if not is_gnome():
        return False
    try:
        shell = Gio.Settings.new(GNOME_SHELL_SCHEMA)
        if shell.get_boolean("disable-user-extensions"):
            return False
        return GNOME_EXTENSION_UUID in shell.get_strv("enabled-extensions")
    except Exception:  # noqa: BLE001 - a schema or dconf can be unavailable
        return False


def copy_extension() -> None:
    """Copy the bundled extension into the user's extension directory."""
    source = extension_source()
    if source is None:
        raise DesktopError(
            "This build does not contain the GNOME integration files. Reinstall "
            "StayProductive or point the package at "
            f"integrations/gnome/{GNOME_EXTENSION_UUID}."
        )
    target = extension_target()
    try:
        target.mkdir(parents=True, exist_ok=True)
        shutil.copytree(source, target, dirs_exist_ok=True)
    except OSError as error:
        raise DesktopError(f"The extension could not be copied: {error}") from error


def enable_extension() -> None:
    """Enable the user extension, failing loudly when policy blocks it."""
    try:
        shell = Gio.Settings.new(GNOME_SHELL_SCHEMA)
        if shell.get_boolean("disable-user-extensions"):
            raise DesktopError("User extensions are disabled by system policy.")
        values = shell.get_strv("enabled-extensions")
        if GNOME_EXTENSION_UUID not in values:
            values.append(GNOME_EXTENSION_UUID)
            shell.set_strv("enabled-extensions", values)
    except DesktopError:
        raise
    except Exception as error:  # noqa: BLE001 - report the real dconf failure
        raise DesktopError(f"The extension could not be enabled: {error}") from error
