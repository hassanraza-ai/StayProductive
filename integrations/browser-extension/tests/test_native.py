"""Framing and daemon-proxy tests for the native-messaging host."""

from __future__ import annotations

import json
import os
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
NATIVE = ROOT / "integrations" / "browser-extension" / "native"
sys.path.insert(0, str(NATIVE))
sys.path.insert(0, str(ROOT / "crates" / "ui"))

import stayproductive_native as native  # noqa: E402


def frame(value: dict) -> bytes:
    payload = json.dumps(value).encode("utf-8")
    return struct.pack("<I", len(payload)) + payload


def read_frame(stream) -> dict:
    header = stream.read(4)
    assert len(header) == 4, f"short frame header: {header!r}"
    (length,) = struct.unpack("<I", header)
    payload = stream.read(length)
    assert len(payload) == length
    return json.loads(payload.decode("utf-8"))


class FakeDaemon(threading.Thread):
    def __init__(self, path: Path, reply):
        super().__init__(daemon=True)
        self.path = path
        self.reply = reply
        self.requests: list[dict] = []
        self.ready = threading.Event()
        self.error: BaseException | None = None

    def run(self) -> None:
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            listener.bind(str(self.path))
            listener.listen(1)
            self.ready.set()
            conn, _ = listener.accept()
        except BaseException as error:  # pragma: no cover - surfaced by the test
            self.error = error
            self.ready.set()
            return
        with conn:
            conn.sendall(b'{"event":"ready","state_path":"/tmp/state.json"}\n')
            reader = conn.makefile("rb")
            for line in reader:
                self.requests.append(json.loads(line))
                conn.sendall((json.dumps(self.reply(self.requests[-1])) + "\n").encode())
        listener.close()


class NativeHostTests(unittest.TestCase):
    def run_host(self, messages, env_extra):
        env = os.environ.copy()
        env.pop("STAYPRODUCTIVE_SOCKET", None)
        env.pop("STAYPRODUCTIVE_NATIVE_SOCKET", None)
        env.update(env_extra)
        process = subprocess.run(
            [sys.executable, str(NATIVE / "stayproductive_native.py")],
            input=b"".join(frame(message) for message in messages),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            timeout=20,
        )
        replies = []
        stream = process.stdout
        while stream:
            header = stream[:4]
            if len(header) < 4:
                break
            (length,) = struct.unpack("<I", header)
            replies.append(json.loads(stream[4 : 4 + length].decode("utf-8")))
            stream = stream[4 + length :]
        return process, replies

    def test_socket_name_matches_the_ui_and_daemon(self) -> None:
        from stayproductive import paths

        with tempfile.TemporaryDirectory() as directory:
            env = {
                "XDG_RUNTIME_DIR": directory,
                "STAYPRODUCTIVE_STATE": "/tmp/portable/state.json",
            }
            saved = {key: os.environ.get(key) for key in env}
            os.environ.update(env)
            try:
                expected = paths.default_socket_path("/tmp/portable/state.json")
                self.assertEqual(native.socket_path(), expected)
                self.assertRegex(
                    native.socket_path().name, r"^daemon-[0-9a-f]{16}\.sock$"
                )
            finally:
                for key, value in saved.items():
                    if value is None:
                        os.environ.pop(key, None)
                    else:
                        os.environ[key] = value

    def test_it_proxies_two_requests_and_cannot_override_the_envelope(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "daemon.sock"
            daemon = FakeDaemon(
                path,
                lambda request: {"id": request.get("id"), "ok": True, "data": {"request": request}},
            )
            daemon.start()
            self.assertTrue(daemon.ready.wait(5), daemon.error)
            process, replies = self.run_host(
                [
                    {
                        "id": "abc-1",
                        "command": "browser_decision",
                        "args": {
                            "cmd": "shutdown",
                            "id": 999,
                            "url": "https://example.com/x",
                            "app": "firefox",
                            "title": "Example",
                            "unknown": "dropped",
                        },
                    },
                    {"id": "abc-2", "command": "browser_policy", "args": None},
                ],
                {"STAYPRODUCTIVE_NATIVE_SOCKET": str(path)},
            )
            self.assertEqual(process.returncode, 0, process.stderr.decode())
            self.assertEqual(len(replies), 2)
            self.assertEqual(replies[0]["id"], "abc-1")
            self.assertTrue(replies[0]["ok"], replies[0])
            request = replies[0]["data"]["request"]
            self.assertEqual(request["id"], 1)
            self.assertEqual(request["cmd"], "browser_decision")
            self.assertEqual(request["url"], "https://example.com/x")
            self.assertEqual(request["app"], "firefox")
            self.assertNotIn("unknown", request)
            self.assertEqual(replies[1]["data"]["request"]["cmd"], "browser_policy")
            self.assertNotIn("args", replies[1]["data"]["request"])
            self.assertEqual(len(daemon.requests), 2)

    def test_an_unknown_command_and_a_missing_daemon_fail_softly(self) -> None:
        process, replies = self.run_host(
            [
                {"id": 1, "command": "shutdown"},
                {"id": 2, "command": "browser_policy", "args": {}},
            ],
            {"STAYPRODUCTIVE_NATIVE_SOCKET": "/nonexistent/stayproductive.sock"},
        )
        self.assertEqual(process.returncode, 0, process.stderr.decode())
        self.assertEqual([reply["id"] for reply in replies], [1, 2])
        self.assertFalse(replies[0]["ok"])
        self.assertIn("unsupported command", replies[0]["error"])
        self.assertFalse(replies[1]["ok"])
        self.assertIn("not reachable", replies[1]["error"])

    def test_an_oversized_frame_is_refused_and_the_host_exits(self) -> None:
        process = subprocess.run(
            [sys.executable, str(NATIVE / "stayproductive_native.py")],
            input=struct.pack("<I", native.MAX_MESSAGE + 1),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=20,
        )
        self.assertNotEqual(process.returncode, 0)
        stream = process.stdout
        (length,) = struct.unpack("<I", stream[:4])
        reply = json.loads(stream[4 : 4 + length].decode("utf-8"))
        self.assertFalse(reply["ok"])
        self.assertIn("too large", reply["error"])

    def test_arguments_reject_non_strings_and_missing_urls(self) -> None:
        with self.assertRaises(native.NativeError):
            native.arguments("browser_decision", {"url": 5})
        with self.assertRaises(native.NativeError):
            native.arguments("browser_decision", {"url": "  "})


if __name__ == "__main__":
    unittest.main()
