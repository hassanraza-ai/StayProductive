"""Small JSON client for the StayProductive local state service."""

from __future__ import annotations

import json
import os
import queue
import shutil
import subprocess
import sys
import threading
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[3]


class EngineError(RuntimeError):
    """A request failed, or the state service went away."""


class Engine:
    """Owns one `stayproductive-daemon` subprocess.

    All decision logic lives in the Rust core. This class only transports
    newline-delimited JSON and exposes typed snapshots to the GTK frontend.
    """

    def __init__(self, state_path: str | None = None) -> None:
        binary = os.environ.get("STAYPRODUCTIVE_DAEMON") or self._find_daemon()
        env = os.environ.copy()
        if state_path:
            env["STAYPRODUCTIVE_STATE"] = state_path
        self._process = subprocess.Popen(
            [binary],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            env=env,
        )
        self._queue: queue.Queue[str | None] = queue.Queue()
        self._lock = threading.Lock()
        self._next_id = 1
        threading.Thread(target=self._read_stdout, daemon=True).start()
        threading.Thread(target=self._read_stderr, daemon=True).start()

        try:
            ready = json.loads(self._get(5.0))
        except Exception as exc:  # noqa: BLE001 - report startup failure plainly
            self.close()
            raise EngineError(f"The local state service did not start: {exc}") from exc
        if ready.get("event") != "ready":
            self.close()
            raise EngineError("The local state service sent an unexpected greeting.")
        self.state_path = ready.get("state_path", "")

    @staticmethod
    def _find_daemon() -> str:
        override = os.environ.get("STAYPRODUCTIVE_DAEMON")
        if override:
            return override
        for candidate in (
            REPO_ROOT / "target" / "release" / "stayproductive-daemon",
            REPO_ROOT / "target" / "debug" / "stayproductive-daemon",
        ):
            if candidate.is_file():
                return str(candidate)
        if shutil.which("stayproductive-daemon"):
            return shutil.which("stayproductive-daemon")  # type: ignore[return-value]
        subprocess.run(
            [
                "cargo",
                "build",
                "--release",
                "-p",
                "stayproductive-daemon",
                "--manifest-path",
                str(REPO_ROOT / "Cargo.toml"),
            ],
            check=True,
        )
        candidate = REPO_ROOT / "target" / "release" / "stayproductive-daemon"
        if not candidate.is_file():
            raise EngineError("Could not build the local state service.")
        return str(candidate)

    def _read_stdout(self) -> None:
        assert self._process.stdout is not None
        for line in self._process.stdout:
            line = line.strip()
            if line:
                self._queue.put(line)
        self._queue.put(None)

    def _read_stderr(self) -> None:
        assert self._process.stderr is not None
        for line in self._process.stderr:
            if os.environ.get("STAYPRODUCTIVE_DEBUG"):
                print(line.rstrip(), file=sys.stderr)

    def _get(self, timeout: float) -> str:
        try:
            value = self._queue.get(timeout=timeout)
        except queue.Empty as exc:
            raise EngineError("The local state service stopped responding.") from exc
        if value is None:
            raise EngineError("The local state service exited unexpectedly.")
        return value

    def request(self, command: str, **payload: Any) -> Any:
        if "id" in payload:
            raise EngineError("the request field 'id' is reserved for the envelope")
        with self._lock:
            if self._process.poll() is not None:
                raise EngineError("The local state service is not running.")
            request_id = self._next_id
            self._next_id += 1
            message = {"id": request_id, "cmd": command, **payload}
            assert self._process.stdin is not None
            try:
                self._process.stdin.write(json.dumps(message) + "\n")
                self._process.stdin.flush()
            except (BrokenPipeError, OSError) as exc:
                raise EngineError("The local state service closed the connection.") from exc

            raw = self._get(20.0)
            try:
                response = json.loads(raw)
            except json.JSONDecodeError as exc:
                raise EngineError(f"The local state service sent invalid JSON: {raw}") from exc
            if response.get("id") not in (request_id, 0):
                raise EngineError("The local state service sent a mismatched reply.")
            if not response.get("ok"):
                raise EngineError(response.get("error") or "The request failed.")
            return response.get("data")

    def snapshot(self) -> dict[str, Any]:
        return self.request("snapshot")

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

    def close(self) -> None:
        if self._process.poll() is None:
            try:
                self.request("shutdown")
            except Exception:  # noqa: BLE001 - best-effort shutdown
                pass
        try:
            self._process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            self._process.terminate()
        if self._process.poll() is None:
            self._process.kill()
