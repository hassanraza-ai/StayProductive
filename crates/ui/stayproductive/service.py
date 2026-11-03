"""systemd user-service helpers for the background daemon.

The daemon is meant to outlive the GTK window. When a systemd user manager is
available, it is started through ``stayproductive.service`` so that a killed
daemon is restarted and autostart is a single ``systemctl --user enable``.

Nothing here is privileged, and nothing is enabled without an explicit call:
``start()`` only writes/updates the user unit and starts it for this session;
``enable()`` is the autostart grant the user asks for in Settings or onboarding.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

from . import paths

UNIT_NAME = "stayproductive.service"
DOCUMENTATION = "https://github.com/hassanraza-ai/StayProductive"


class ServiceError(RuntimeError):
    """The background service could not be changed."""


def systemctl_available() -> bool:
    return shutil.which("systemctl") is not None


def _systemctl(*args: str, timeout: float = 10.0) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            ["systemctl", "--user", *args],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except FileNotFoundError as error:
        raise ServiceError("systemd is not installed on this system.") from error
    except subprocess.TimeoutExpired as error:
        raise ServiceError("systemctl did not respond.") from error


def _error_text(result: subprocess.CompletedProcess[str]) -> str:
    text = (result.stderr or result.stdout or "").strip()
    return text or f"systemctl exited with status {result.returncode}."


def user_manager_running() -> bool:
    if not systemctl_available():
        return False
    try:
        return _systemctl("show-environment").returncode == 0
    except ServiceError:
        return False


def unit_path() -> Path:
    return paths.config_home() / "systemd" / "user" / UNIT_NAME


def daemon_binary() -> str | None:
    override = os.environ.get("STAYPRODUCTIVE_DAEMON")
    if override:
        return override
    repo_root = Path(__file__).resolve().parents[3]
    for candidate in (
        repo_root / "target" / "release" / "stayproductive-daemon",
        repo_root / "target" / "debug" / "stayproductive-daemon",
    ):
        if candidate.is_file():
            return str(candidate)
    return shutil.which("stayproductive-daemon")


def quote_exec(path: str) -> str:
    """Quote one ExecStart argument the way systemd expects."""

    return '"' + path.replace("\\", "\\\\").replace('"', '\\"') + '"'


def unit_text(binary: str, data_home: str | None = None) -> str:
    environment = ""
    if data_home:
        # Keep the unit's state path identical to the UI's, even when the user
        # manager does not inherit XDG_DATA_HOME.
        environment = f"Environment={quote_exec('XDG_DATA_HOME=' + data_home)}\n"
    return (
        "[Unit]\n"
        "Description=StayProductive focus daemon\n"
        f"Documentation={DOCUMENTATION}\n"
        "PartOf=graphical-session.target\n"
        "After=graphical-session.target\n"
        "\n"
        "[Service]\n"
        "Type=simple\n"
        f"ExecStart={quote_exec(binary)} --socket\n"
        f"{environment}"
        "Restart=always\n"
        "RestartSec=2\n"
        "NoNewPrivileges=yes\n"
        "\n"
        "[Install]\n"
        "WantedBy=default.target\n"
    )


def write_unit() -> bool:
    """Write the user unit when it is missing or points at an older binary."""

    binary = daemon_binary()
    if not binary:
        raise ServiceError(
            "The state service binary was not found. Build it or set "
            "STAYPRODUCTIVE_DAEMON."
        )
    text = unit_text(binary, os.environ.get("XDG_DATA_HOME"))
    path = unit_path()
    try:
        if path.is_file() and path.read_text(encoding="utf-8") == text:
            return False
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    except OSError as error:
        raise ServiceError(f"Could not write {path}: {error}") from error
    result = _systemctl("daemon-reload")
    if result.returncode != 0:
        raise ServiceError(_error_text(result))
    return True


def is_enabled() -> bool:
    if not systemctl_available():
        return False
    try:
        return _systemctl("is-enabled", "--quiet", UNIT_NAME).returncode == 0
    except ServiceError:
        return False


def is_active() -> bool:
    if not systemctl_available():
        return False
    try:
        return _systemctl("is-active", "--quiet", UNIT_NAME).returncode == 0
    except ServiceError:
        return False


def can_manage(state_path: str | None = None) -> bool:
    """Whether systemd can own this daemon instance at all."""

    if not systemctl_available() or not user_manager_running():
        return False
    if state_path and Path(state_path).expanduser().resolve() != (
        paths.default_state_path().expanduser().resolve()
    ):
        return False
    return True


def service_state(state_path: str | None = None) -> str:
    """One of ``unavailable``, ``custom-state``, ``enabled``, ``installed``, ``off``."""

    if not systemctl_available() or not user_manager_running():
        return "unavailable"
    if state_path and Path(state_path).expanduser().resolve() != (
        paths.default_state_path().expanduser().resolve()
    ):
        return "custom-state"
    if is_enabled():
        return "enabled"
    if unit_path().is_file():
        return "installed"
    return "off"


def start() -> None:
    """Start the daemon for this session through systemd (not autostart)."""

    if not systemctl_available() or not user_manager_running():
        raise ServiceError("systemd user services are not available in this session.")
    write_unit()
    if is_active():
        return
    result = _systemctl("start", UNIT_NAME)
    if result.returncode != 0:
        raise ServiceError(_error_text(result))


def stop() -> None:
    """Best-effort stop, used before falling back to a directly spawned daemon."""

    if not systemctl_available():
        return
    try:
        _systemctl("stop", UNIT_NAME)
    except ServiceError:
        pass


def enable(engine=None, state_path: str | None = None) -> None:
    """Enable the user unit and hand the running daemon to systemd.

    When a directly spawned daemon owns the socket, it is stopped first so the
    newly started unit does not lose the single-instance race.
    """

    current = service_state(state_path)
    if current == "unavailable":
        raise ServiceError("systemd user services are not available in this session.")
    if current == "custom-state":
        raise ServiceError(
            "Start at login is only available for the default data file."
        )
    if not is_active() and engine is not None:
        engine.shutdown_service()
    write_unit()
    result = _systemctl("enable", UNIT_NAME)
    if result.returncode != 0:
        raise ServiceError(_error_text(result))
    result = _systemctl("start", UNIT_NAME)
    if result.returncode != 0:
        raise ServiceError(_error_text(result))


def disable() -> None:
    """Stop starting the daemon at login, leaving this session's daemon running."""

    if not systemctl_available():
        raise ServiceError("systemd is not installed on this system.")
    result = _systemctl("disable", UNIT_NAME)
    if result.returncode != 0:
        raise ServiceError(_error_text(result))
