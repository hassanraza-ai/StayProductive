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
        self.protocol_version = 1
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
                    {
                        "event": "ready",
                        "state_path": str(self.state_path),
                        "protocol_version": self.protocol_version,
                    }
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
                            "protocol_version": self.protocol_version,
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
        engine.import_data("/tmp/backup.zip")

        commands = [request["cmd"] for request in self.daemon.requests]
        self.assertEqual(
            commands,
            ["snapshot", "export_data", "export_data", "delete_history", "import_data"],
        )
        self.assertEqual(self.daemon.requests[1]["include_usage"], True)
        self.assertNotIn("path", self.daemon.requests[1])
        self.assertEqual(self.daemon.requests[2]["path"], "/tmp/elsewhere.zip")
        self.assertEqual(self.daemon.requests[2]["include_usage"], False)
        self.assertIs(self.daemon.requests[3]["confirm"], True)
        self.assertEqual(self.daemon.requests[4]["path"], "/tmp/backup.zip")

    def test_browser_calls_carry_the_page_and_no_storage_intent(self) -> None:
        engine = self.engine()
        engine.browser_policy()
        engine.browser_decision("https://bad.example.com/GAME", title="Games")

        commands = [request["cmd"] for request in self.daemon.requests]
        self.assertEqual(commands, ["browser_policy", "browser_decision"])
        decision = self.daemon.requests[1]
        self.assertEqual(decision["url"], "https://bad.example.com/GAME")
        self.assertEqual(decision["title"], "Games")
        self.assertEqual(decision["app"], "browser")
        self.assertNotIn("store", decision)

    def test_duplicate_and_pause_send_the_chosen_fields(self) -> None:
        engine = self.engine()
        engine.duplicate_profile("01ARZ3NDEKTSV4RRFFQ69G5FAV", "Focus copy")
        engine.pause_profile("01ARZ3NDEKTSV4RRFFQ69G5FAV", 15)
        engine.pause_profile("01ARZ3NDEKTSV4RRFFQ69G5FAV", 0, until_end_of_day=True)
        engine.pause_profile("01ARZ3NDEKTSV4RRFFQ69G5FAV", 0)

        duplicate, quarter, today, indefinite = self.daemon.requests[-4:]
        self.assertEqual(duplicate["cmd"], "duplicate_profile")
        self.assertEqual(duplicate["name"], "Focus copy")
        self.assertEqual(quarter["minutes"], 15)
        self.assertNotIn("until_end_of_day", quarter)
        self.assertIs(today["until_end_of_day"], True)
        self.assertEqual(indefinite["minutes"], 0)
        self.assertNotIn("until_end_of_day", indefinite)

    def test_reset_sends_confirmation_and_the_current_schema(self) -> None:
        engine = self.engine()
        engine.snapshot()
        engine.reset_all()

        self.assertEqual(self.daemon.requests[1]["cmd"], "reset_all")
        self.assertIs(self.daemon.requests[1]["confirm"], True)
        self.assertEqual(self.daemon.requests[0]["schema_version"], 1)

    def test_a_newer_service_is_reported_instead_of_guessed(self) -> None:
        self.daemon.protocol_version = 2
        with self.assertRaises(EngineError) as caught:
            self.engine()  # the version check happens on connect
        message = str(caught.exception)
        self.assertIn("protocol 2", message)
        self.assertIn("newer", message)
        self.assertIn("continues", message)

        # Once the service matches again, a fresh client connects normally.
        self.daemon.protocol_version = 1
        engine = self.engine()
        self.assertEqual(engine.snapshot()["profiles"], [])


if __name__ == "__main__":
    unittest.main()
