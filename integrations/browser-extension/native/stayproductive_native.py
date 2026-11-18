#!/usr/bin/env python3
"""Native-messaging host between the browser extension and the daemon.

The browser starts this process for a `connectNative` port. Messages use the
standard native-messaging framing: a 4-byte little-endian length followed by
one UTF-8 JSON object. The host translates the small, fixed command set below
into newline-delimited JSON on the same-user daemon socket and returns the
daemon's answer.

It deliberately has no other capability: no shell, no filesystem API for the
extension, no network, and no logging of URLs. The daemon remains the only
decision point.
"""

from __future__ import annotations

import json
import os
import socket
import struct
import sys
from pathlib import Path
from typing import Any, BinaryIO

MAX_MESSAGE = 1024 * 1024
CONNECT_TIMEOUT = 5.0
READ_TIMEOUT = 20.0
ALLOWED_COMMANDS = {
    "browser_policy": (),
    "browser_decision": ("url", "title", "app"),
}


class NativeError(RuntimeError):
    """A refused message or an unreachable daemon."""


def fnv1a_hex(data: bytes) -> str:
    value = 0xCBF29CE484222325
    for byte in data:
        value ^= byte
        value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{value:016x}"


def runtime_dir() -> Path:
    value = os.environ.get("XDG_RUNTIME_DIR")
    return Path(value) if value else Path("/tmp")


def socket_path() -> Path:
    """Mirror `paths.default_socket_path` in the UI and the Rust daemon."""

    override = os.environ.get("STAYPRODUCTIVE_NATIVE_SOCKET")
    if override:
        return Path(override)
    override = os.environ.get("STAYPRODUCTIVE_SOCKET")
    if override:
        return Path(override)
    name = "daemon"
    state = os.environ.get("STAYPRODUCTIVE_STATE")
    if state:
        name = f"daemon-{fnv1a_hex(state.encode('utf-8'))}"
    return runtime_dir() / "stayproductive" / f"{name}.sock"


def read_exact(stream: BinaryIO, size: int) -> bytes:
    chunks = bytearray()
    while len(chunks) < size:
        chunk = stream.read(size - len(chunks))
        if not chunk:
            raise NativeError("native message ended early")
        chunks.extend(chunk)
    return bytes(chunks)


def read_message(stream: BinaryIO) -> dict[str, Any] | None:
    header = stream.read(4)
    if not header:
        return None
    if len(header) != 4:
        raise NativeError("native message header ended early")
    (length,) = struct.unpack("<I", header)
    if length == 0 or length > MAX_MESSAGE:
        raise NativeError("native message is too large")
    payload = read_exact(stream, length)
    try:
        value = json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise NativeError("native message is not JSON") from error
    if not isinstance(value, dict):
        raise NativeError("native message must be an object")
    return value


def write_message(stream: BinaryIO, value: dict[str, Any]) -> None:
    payload = json.dumps(value, separators=(",", ":")).encode("utf-8")
    stream.write(struct.pack("<I", len(payload)))
    stream.write(payload)
    stream.flush()


class DaemonClient:
    """One persistent, same-user connection to the state service."""

    def __init__(self, path: Path | None = None) -> None:
        self.path = path or socket_path()
        self._socket: socket.socket | None = None
        self._reader: BinaryIO | None = None

    def close(self) -> None:
        for value in (self._reader, self._socket):
            if value is not None:
                try:
                    value.close()
                except OSError:
                    pass
        self._reader = None
        self._socket = None

    def _connect(self) -> None:
        self.close()
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(CONNECT_TIMEOUT)
        try:
            sock.connect(str(self.path))
        except OSError as error:
            sock.close()
            raise NativeError(f"StayProductive service is not reachable: {error.strerror}") from error
        sock.settimeout(READ_TIMEOUT)
        reader = sock.makefile("rb")
        try:
            greeting = reader.readline(MAX_MESSAGE)
            value = json.loads(greeting.decode("utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            reader.close()
            sock.close()
            raise NativeError("StayProductive service sent no greeting") from error
        if not isinstance(value, dict) or value.get("event") != "ready":
            reader.close()
            sock.close()
            raise NativeError("StayProductive service sent an unexpected greeting")
        self._socket = sock
        self._reader = reader

    def request(self, command: str, args: dict[str, Any]) -> Any:
        if self._socket is None or self._reader is None:
            self._connect()
        assert self._socket is not None and self._reader is not None
        message = {"id": 1, "cmd": command, **args}
        try:
            self._socket.settimeout(READ_TIMEOUT)
            self._socket.sendall((json.dumps(message) + "\n").encode("utf-8"))
            raw = self._reader.readline(MAX_MESSAGE)
        except OSError as error:
            self.close()
            raise NativeError(f"StayProductive service stopped responding: {error.strerror}") from error
        if not raw:
            self.close()
            raise NativeError("StayProductive service closed the connection")
        try:
            response = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            self.close()
            raise NativeError("StayProductive service sent invalid JSON") from error
        if not isinstance(response, dict):
            self.close()
            raise NativeError("StayProductive service sent an unexpected response")
        if not response.get("ok"):
            raise NativeError(str(response.get("error") or "request refused"))
        return response.get("data")


def arguments(command: str, raw: Any) -> dict[str, Any]:
    """Select only the arguments the daemon command accepts."""

    allowed = ALLOWED_COMMANDS[command]
    if raw is None:
        return {}
    if not isinstance(raw, dict):
        raise NativeError("arguments must be an object")
    selected: dict[str, Any] = {}
    for key in allowed:
        if key not in raw:
            continue
        value = raw[key]
        if value is None:
            continue
        if not isinstance(value, str):
            raise NativeError(f"argument {key} must be a string")
        selected[key] = value
    if command == "browser_decision" and not selected.get("url", "").strip():
        raise NativeError("browser_decision requires a url")
    return selected


def handle(message: dict[str, Any], client: DaemonClient) -> dict[str, Any]:
    identifier = message.get("id")
    command = message.get("command")
    if not isinstance(command, str) or command not in ALLOWED_COMMANDS:
        return {"id": identifier, "ok": False, "error": "unsupported command"}
    try:
        data = client.request(command, arguments(command, message.get("args")))
    except NativeError as error:
        return {"id": identifier, "ok": False, "error": str(error)}
    return {"id": identifier, "ok": True, "data": data}


def main() -> int:
    client = DaemonClient()
    try:
        while True:
            try:
                message = read_message(sys.stdin.buffer)
            except NativeError as error:
                # The frame boundary is lost, so this port cannot continue.
                write_message(
                    sys.stdout.buffer,
                    {"id": None, "ok": False, "error": str(error)},
                )
                return 1
            if message is None:
                return 0
            write_message(sys.stdout.buffer, handle(message, client))
    finally:
        client.close()


if __name__ == "__main__":
    sys.exit(main())
