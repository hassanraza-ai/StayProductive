"""JSON client for the StayProductive local state service.

The daemon owns the state and normally runs independently of the GTK window over
a same-user Unix socket in ``$XDG_RUNTIME_DIR/stayproductive``. Closing the UI
therefore leaves blocking and schedule transitions running; opening the UI again
reconnects to the same daemon. The Rust protocol tests still use the older stdio
transport directly.
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import threading
import time
from pathlib import Path
from typing import Any

from . import paths, service

REQUEST_TIMEOUT = 20.0
CONNECT_TIMEOUT = 5.0
START_TIMEOUT = 10.0


class EngineError(RuntimeError):
    """A request failed, or the state service refused it."""


class EngineConnectionError(EngineError):
    """The state service could not be reached or stopped answering."""


class Engine:
    """One connection to the background daemon.

    All decision logic lives in the Rust core. This class only transports
    newline-delimited JSON and exposes typed snapshots to the GTK frontend.
    """

    def __init__(
        self,
        state_path: str | None = None,
        *,
        socket_path: str | None = None,
        spawn: bool = True,
    ) -> None:
        self._state_env = (
            state_path if state_path is not None else os.environ.get("STAYPRODUCTIVE_STATE")
        )
        expected = self._state_env or str(paths.default_state_path())
        self._expected_state = Path(expected).expanduser().resolve()
        self._socket_path = (
            Path(socket_path)
            if socket_path is not None
            else paths.default_socket_path(self._state_env)
        )
        self._spawn = spawn
        self._lock = threading.Lock()
        self._socket: socket.socket | None = None
        self._reader: Any = None
        self._child: subprocess.Popen | None = None
        self._next_id = 1
        self.state_path = ""
        self._establish()

    # ------------------------------------------------------------- connection
    def _establish(self) -> None:
        if self._try_connect():
            return
        if not self._spawn:
            raise EngineConnectionError(
                f"No background state service is running at {self._socket_path}."
            )
        self._start_background()
        deadline = time.monotonic() + START_TIMEOUT
        while time.monotonic() < deadline:
            if self._try_connect():
                return
            time.sleep(0.1)
        self._reap_child()
        raise EngineConnectionError(
            "The local state service did not start. See "
            f"{self._expected_state.parent / 'daemon.log'} for details."
        )

    def _try_connect(self) -> bool:
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(CONNECT_TIMEOUT)
        try:
            sock.connect(str(self._socket_path))
        except OSError:
            sock.close()
            return False
        try:
            reader = sock.makefile("r", encoding="utf-8", newline="\n")
            greeting = json.loads(reader.readline())
        except (OSError, ValueError) as error:
            sock.close()
            raise EngineConnectionError(
                "The local state service sent an unexpected greeting."
            ) from error
        if not isinstance(greeting, dict) or greeting.get("event") != "ready":
            sock.close()
            raise EngineConnectionError(
                "The local state service sent an unexpected greeting."
            )
        state_path = Path(greeting.get("state_path", "")).expanduser().resolve()
        if state_path != self._expected_state:
            sock.close()
            raise EngineError(
                f"A background service is already running for {state_path}. "
                "Stop it or use the same data file."
            )
        sock.settimeout(REQUEST_TIMEOUT)
        self._socket = sock
        self._reader = reader
        self.state_path = str(state_path)
        return True

    def _start_background(self) -> None:
        if service.can_manage(self._state_env):
            try:
                service.start()
                return
            except service.ServiceError:
                # Do not leave a unit that keeps retrying while a directly
                # spawned daemon owns the socket.
                service.stop()
        self._spawn_daemon()

    def _spawn_daemon(self) -> None:
        self._reap_child()
        binary = service.daemon_binary()
        if not binary:
            raise EngineConnectionError(
                "The state service binary was not found. Run "
                "`cargo build --release -p stayproductive-daemon` or set "
                "STAYPRODUCTIVE_DAEMON."
            )
        env = os.environ.copy()
        if self._state_env is not None:
            env["STAYPRODUCTIVE_STATE"] = self._state_env
        log = None
        try:
            log_path = self._expected_state.parent / "daemon.log"
            log_path.parent.mkdir(parents=True, exist_ok=True)
            descriptor = os.open(
                log_path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600
            )
            log = os.fdopen(descriptor, "ab", buffering=0)
        except OSError:
            log = None
        try:
            # A new session detaches the daemon from the window: closing the UI
            # must not stop enforcement.
            self._child = subprocess.Popen(
                [binary, "--socket", str(self._socket_path)],
                stdin=subprocess.DEVNULL,
                stdout=log or subprocess.DEVNULL,
                stderr=log or subprocess.DEVNULL,
                env=env,
                start_new_session=True,
                close_fds=True,
            )
        finally:
            if log is not None:
                log.close()

    def _reap_child(self) -> None:
        if self._child is not None and self._child.poll() is not None:
            self._child = None

    def _close_socket(self) -> None:
        reader = self._reader
        sock = self._socket
        self._reader = None
        self._socket = None
        if reader is not None:
            try:
                reader.close()
            except OSError:
                pass
        if sock is not None:
            try:
                sock.close()
            except OSError:
                pass

    # ---------------------------------------------------------------- requests
    def _request(
        self,
        command: str,
        payload: dict[str, Any] | None = None,
        *,
        timeout: float = REQUEST_TIMEOUT,
    ) -> Any:
        payload = payload or {}
        if "id" in payload:
            raise EngineError("the request field 'id' is reserved for the envelope")
        with self._lock:
            if self._socket is None or self._reader is None:
                self._establish()
            request_id = self._next_id
            self._next_id += 1
            message = {"id": request_id, "cmd": command, **payload}
            assert self._socket is not None and self._reader is not None
            try:
                self._socket.settimeout(timeout)
                self._socket.sendall((json.dumps(message) + "\n").encode("utf-8"))
                raw = self._reader.readline()
            except (OSError, ValueError) as error:
                self._close_socket()
                raise EngineConnectionError(
                    "The local state service stopped responding."
                ) from error
            if not raw:
                self._close_socket()
                raise EngineConnectionError(
                    "The local state service closed the connection."
                )
            try:
                response = json.loads(raw)
            except json.JSONDecodeError as error:
                raise EngineConnectionError(
                    f"The local state service sent invalid JSON: {raw!r}"
                ) from error
            if response.get("id") not in (request_id, 0):
                raise EngineConnectionError(
                    "The local state service sent a mismatched reply."
                )
            if not response.get("ok"):
                raise EngineError(response.get("error") or "The request failed.")
            return response.get("data")

    def request(self, command: str, **payload: Any) -> Any:
        return self._request(command, payload)

    def snapshot(self) -> dict[str, Any]:
        try:
            return self._request("snapshot")
        except EngineConnectionError:
            # A crash or an update between ticks should recover on the next
            # read; only a read-only command is retried automatically.
            return self._request("snapshot")

    def create_profile(self, profile: dict[str, Any]) -> dict[str, Any]:
        return self.request("create_profile", profile=profile)

    def update_profile(self, profile_id: str, profile: dict[str, Any]) -> dict[str, Any]:
        return self.request("update_profile", profile_id=profile_id, profile=profile)

    def delete_profile(self, profile_id: str) -> dict[str, Any]:
        return self.request("delete_profile", profile_id=profile_id)

    def activate_profile(self, profile_id: str) -> dict[str, Any]:
        return self.request("activate_profile", profile_id=profile_id)

    def deactivate_profile(self) -> dict[str, Any]:
        return self.request("deactivate_profile")

    def pause_profile(self, profile_id: str, minutes: int) -> dict[str, Any]:
        return self.request("pause_profile", profile_id=profile_id, minutes=minutes)

    def resume_profile(self, profile_id: str) -> dict[str, Any]:
        return self.request("resume_profile", profile_id=profile_id)

    def quick_block(self, kind: str, target: str, minutes: int) -> dict[str, Any]:
        return self.request("quick_block", kind=kind, target=target, minutes=minutes)

    def clear_quick_block(self, kind: str, target: str) -> dict[str, Any]:
        return self.request("clear_quick_block", kind=kind, target=target)

    def start_strict(self, minutes: int, cooldown_minutes: int) -> dict[str, Any]:
        return self.request(
            "start_strict", minutes=minutes, cooldown_minutes=cooldown_minutes
        )

    def request_commitment_end(self, commitment_id: str) -> dict[str, Any]:
        return self.request("request_commitment_end", commitment_id=commitment_id)

    def finish_commitment_end(self, commitment_id: str) -> dict[str, Any]:
        return self.request("finish_commitment_end", commitment_id=commitment_id)

    def set_settings(self, start_of_day: str, timezone: str) -> dict[str, Any]:
        return self.request(
            "set_settings", start_of_day=start_of_day, timezone=timezone
        )

    # ------------------------------------------------------------------ data
    def export_data(
        self, path: str | None = None, *, include_usage: bool = True
    ) -> dict[str, Any]:
        """Write a versioned archive of profiles, settings, and (optionally) usage."""

        payload: dict[str, Any] = {"include_usage": include_usage}
        if path:
            payload["path"] = path
        return self.request("export_data", **payload)

    def delete_history(self) -> dict[str, Any]:
        """Forget raw and aggregate usage. Profiles and commitments are kept."""

        return self.request("delete_history", confirm=True)

    def import_data(self, path: str) -> dict[str, Any]:
        """Restore profiles, settings, and commitments from an export archive."""

        return self.request("import_data", path=path)

    # --------------------------------------------------------------- lifecycle
    def shutdown_service(self) -> None:
        """Ask the daemon to stop. Only used for explicit service handover."""

        try:
            self._request("shutdown", timeout=2.0)
        except EngineError:
            pass
        finally:
            with self._lock:
                self._close_socket()

    def close(self) -> None:
        """Detach from the daemon without stopping it.

        The daemon is intentionally independent: it keeps enforcing when the
        window closes, and autostart is a separate user decision.
        """

        with self._lock:
            self._close_socket()
