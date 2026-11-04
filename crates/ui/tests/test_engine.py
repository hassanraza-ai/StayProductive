"""Tests for the socket client and daemon lifecycle; no display required."""

from __future__ import annotations

import json
import socket
import tempfile
import threading
import unittest
from pathlib import Path

from stayproductive.engine import Engine, EngineConnectionError, EngineError


class FakeDaemon:
    """Minimal socket server that speaks the daemon's newline JSON protocol."""

    def __init__(self, path: Path, state_path: Path) -> None:
        self.path = Path(path)
        self.state_path = Path(state_path)
        self.commands: list[str] = []
        self.requests: list[dict] = []
        self._closing = False
        self._connections: list[socket.socket] = []
        self._server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._server.settimeout(0.2)
        self._server.bind(str(self.path))
        self._server.listen(4)
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()

    def _serve(self) -> None:
        while not self._closing:
            try:
                connection, _address = self._server.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            self._connections.append(connection)
            threading.Thread(
                target=self._serve_client, args=(connection,), daemon=True
            ).start()

    def _serve_client(self, connection: socket.socket) -> None:
        stream = None
        try:
            stream = connection.makefile("rw", encoding="utf-8", newline="\n")
            stream.write(
                json.dumps(
                    {"event": "ready", "state_path": str(self.state_path)}
                )
                + "\n"
            )
            stream.flush()
            for line in stream:
                if not line.strip():
                    continue
                request = json.loads(line)
                self.commands.append(request["cmd"])
                self.requests.append(request)
                if request["cmd"] == "shutdown":
                    stream.write(
                        json.dumps(
                            {"id": request["id"], "ok": True, "data": {"bye": True}}
                        )
                        + "\n"
                    )
                    stream.flush()
                    return
                stream.write(
                    json.dumps(
                        {
                            "id": request["id"],
                            "ok": True,
                            "data": {
                                "state_path": str(self.state_path),
                                "profiles": [],
                            },
                        }
                    )
                    + "\n"
                )
                stream.flush()
        except (OSError, ValueError):
            return
        finally:
            if stream is not None:
                try:
                    stream.close()
                except OSError:
                    pass
            try:
                connection.close()
            except OSError:
                pass

    def stop(self) -> None:
        self._closing = True
        for connection in self._connections:
            try:
                connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                connection.close()
            except OSError:
                pass
        self._connections.clear()
        try:
            self._server.close()
        except OSError:
            pass
        self.path.unlink(missing_ok=True)


class EngineSocketTest(unittest.TestCase):
    def setUp(self) -> None:
        self._temp = tempfile.TemporaryDirectory()
        self.addCleanup(self._temp.cleanup)
        self.dir = Path(self._temp.name)
        self.state = self.dir / "state.json"
        self.socket = self.dir / "daemon.sock"
        self.daemon = FakeDaemon(self.socket, self.state)
        self.addCleanup(self.daemon.stop)

    def engine(self, *, spawn: bool = False, state: Path | None = None) -> Engine:
        engine = Engine(
            state_path=str(state or self.state),
            socket_path=str(self.socket),
            spawn=spawn,
        )
        self.addCleanup(engine.close)
        return engine

    def test_round_trip_and_close_does_not_stop_the_daemon(self) -> None:
        engine = self.engine()
        self.assertEqual(engine.snapshot()["profiles"], [])
        engine.close()
        self.assertNotIn("shutdown", self.daemon.commands)
        self.assertTrue(self.socket.exists(), "closing the UI must not remove the socket")

        again = self.engine()
        again.snapshot()
        again.shutdown_service()
        self.assertIn("shutdown", self.daemon.commands)
        again.close()

    def test_missing_daemon_is_reported_when_spawning_is_disabled(self) -> None:
        self.daemon.stop()
        with self.assertRaises(EngineConnectionError):
            self.engine()

    def test_snapshot_reconnects_after_the_daemon_restarts(self) -> None:
        engine = self.engine()
        engine.snapshot()
        self.daemon.stop()

        self.socket.unlink(missing_ok=True)
        restarted = FakeDaemon(self.socket, self.state)
        self.addCleanup(restarted.stop)
        self.assertEqual(engine.snapshot()["profiles"], [])
        self.assertEqual(restarted.commands[-1], "snapshot")

    def test_a_different_state_file_is_refused(self) -> None:
        other = self.dir / "elsewhere" / "state.json"
        with self.assertRaises(EngineError) as caught:
            self.engine(state=other)
        self.assertIn("already running", str(caught.exception))

    def test_export_and_delete_send_explicit_intent(self) -> None:
        engine = self.engine()
        engine.snapshot()
        engine.export_data()
        engine.export_data("/tmp/elsewhere.zip", include_usage=False)
        engine.delete_history()

        commands = [request["cmd"] for request in self.daemon.requests]
        self.assertEqual(
            commands,
            ["snapshot", "export_data", "export_data", "delete_history"],
        )
        self.assertEqual(self.daemon.requests[1]["include_usage"], True)
        self.assertNotIn("path", self.daemon.requests[1])
        self.assertEqual(self.daemon.requests[2]["path"], "/tmp/elsewhere.zip")
        self.assertEqual(self.daemon.requests[2]["include_usage"], False)
        self.assertIs(self.daemon.requests[3]["confirm"], True)


if __name__ == "__main__":
    unittest.main()
