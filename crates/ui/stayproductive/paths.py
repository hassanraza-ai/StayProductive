"""Filesystem locations shared by the UI, the daemon, and the service unit.

The Rust daemon implements the same rules. Keep ``fnv1a_hex`` and the socket
name in sync: a custom state file gets its own socket so a test or portable
instance never talks to the login service's daemon.
"""

from __future__ import annotations

import os
from pathlib import Path


def data_home() -> Path:
    value = os.environ.get("XDG_DATA_HOME")
    return Path(value) if value else Path.home() / ".local/share"


def config_home() -> Path:
    value = os.environ.get("XDG_CONFIG_HOME")
    return Path(value) if value else Path.home() / ".config"


def runtime_dir() -> Path:
    value = os.environ.get("XDG_RUNTIME_DIR")
    return Path(value) if value else Path("/tmp")


def default_state_path() -> Path:
    return data_home() / "stayproductive" / "state.json"


def fnv1a_hex(data: bytes) -> str:
    """Stable 64-bit FNV-1a; mirrored by ``fnv1a_hex`` in the daemon."""

    value = 0xCBF29CE484222325
    for byte in data:
        value ^= byte
        value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{value:016x}"


def default_socket_path(state: str | None = None) -> Path:
    override = os.environ.get("STAYPRODUCTIVE_SOCKET")
    if override:
        return Path(override)
    if state is None:
        state = os.environ.get("STAYPRODUCTIVE_STATE")
    name = "daemon"
    if state:
        name = f"{name}-{fnv1a_hex(state.encode('utf-8'))}"
    return runtime_dir() / "stayproductive" / f"{name}.sock"
