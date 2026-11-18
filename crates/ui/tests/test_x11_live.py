"""Live adapter check against a throwaway Xvfb display.

This test is opt-in because it starts an X server and a second GTK process:

    STAYPRODUCTIVE_X11_LIVE=1 PYTHONPATH=crates/ui \\
        python3 -m unittest crates.ui.tests.test_x11_live

It validates the parts that cannot be tested without a display: the adapter
finds the focused window, unmaps a blocked one, writes a heartbeat, and
recovers when an allowed window is focused. A real window manager and a
logged-in desktop are still required for the iconify/overlay visuals.
"""

from __future__ import annotations

import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

from stayproductive.x11 import WindowGeometry, X11

ROOT = Path(__file__).resolve().parents[3]
LIVE = os.environ.get("STAYPRODUCTIVE_X11_LIVE") == "1"


def _free_display() -> tuple[str, subprocess.Popen]:
    for number in range(90, 120):
        socket = Path(f"/tmp/.X11-unix/X{number}")
        if socket.exists():
            continue
        process = subprocess.Popen(
            [
                "Xvfb",
                f":{number}",
                "-screen",
                "0",
                "1200x800x24",
                "-nolisten",
                "tcp",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if socket.exists():
                return f":{number}", process
            if process.poll() is not None:
                break
            time.sleep(0.05)
        process.kill()
        process.wait()
    raise RuntimeError("could not start Xvfb")


@unittest.skipUnless(LIVE, "set STAYPRODUCTIVE_X11_LIVE=1 to run the live X11 test")
@unittest.skipUnless(shutil.which("Xvfb"), "Xvfb is not installed")
class LiveAdapterTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.display, cls.xvfb = _free_display()
        cls.directory = Path(tempfile.mkdtemp(prefix="stayproductive-x11-live-"))
        cls.runtime = cls.directory / "run"
        cls.runtime.mkdir(mode=0o700)
        cls.state = cls.directory / "state.json"
        cls.state.write_text("{}", encoding="utf-8")
        policy = {
            "version": 1,
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "valid_until": (
                datetime.now(timezone.utc) + timedelta(seconds=60)
            ).isoformat(),
            "apps": [
                {
                    "app": "blockedtest",
                    "reason": "R-PROFILE",
                    "until": None,
                    "profile": "Focus",
                }
            ],
        }
        (cls.directory / "enforcement.json").write_text(
            json.dumps(policy), encoding="utf-8"
        )
        cls.heartbeat = cls.runtime / "stayproductive" / "x11-adapter.json"
        env = os.environ.copy()
        env.update(
            {
                "DISPLAY": cls.display,
                "GDK_BACKEND": "x11",
                "XDG_RUNTIME_DIR": str(cls.runtime),
                "PYTHONPATH": str(ROOT / "crates/ui"),
            }
        )
        cls.log = open(cls.directory / "adapter.log", "wb")
        cls.adapter = subprocess.Popen(
            [
                sys.executable,
                "-m",
                "stayproductive.x11_adapter",
                "--state",
                str(cls.state),
                "--heartbeat",
                str(cls.heartbeat),
                "--parent-pid",
                str(os.getpid()),
            ],
            env=env,
            stdout=cls.log,
            stderr=cls.log,
        )

    @classmethod
    def tearDownClass(cls) -> None:
        cls.adapter.send_signal(signal.SIGTERM)
        try:
            cls.adapter.wait(timeout=5)
        except subprocess.TimeoutExpired:
            cls.adapter.kill()
            cls.adapter.wait()
        cls.log.close()
        cls.xvfb.terminate()
        try:
            cls.xvfb.wait(timeout=5)
        except subprocess.TimeoutExpired:
            cls.xvfb.kill()
        shutil.rmtree(cls.directory, ignore_errors=True)

    def heartbeat_now(self) -> dict:
        try:
            return json.loads(self.heartbeat.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return {}

    def wait_for_heartbeat(self, predicate, timeout: float = 8.0) -> dict:
        deadline = time.monotonic() + timeout
        last: dict = {}
        while time.monotonic() < deadline:
            last = self.heartbeat_now()
            if predicate(last):
                return last
            if self.adapter.poll() is not None:
                log = (self.directory / "adapter.log").read_text(
                    encoding="utf-8", errors="replace"
                )
                self.fail(
                    f"adapter exited {self.adapter.returncode} before the heartbeat; "
                    f"log:\n{log}"
                )
            time.sleep(0.05)
        log = (self.directory / "adapter.log").read_text(
            encoding="utf-8", errors="replace"
        )
        self.fail(f"timed out; last heartbeat {last}; log:\n{log}")

    def test_blocks_and_unblocks_a_real_x11_window(self) -> None:
        self.wait_for_heartbeat(lambda value: value.get("policy_loaded") is True)
        backend = X11(self.display)
        blocked = backend.create_simple_window(
            "blockedtest", "Blockedtest", WindowGeometry(40, 40, 400, 300)
        )
        allowed = backend.create_simple_window(
            "allowedtest", "Allowedtest", WindowGeometry(500, 40, 400, 300)
        )
        backend.sync()
        try:
            backend.focus(blocked)
            backend.sync()
            heartbeat = self.wait_for_heartbeat(
                lambda value: value.get("blocked") is True
            )
            self.assertFalse(
                backend.is_viewable(blocked), "the blocked window must be unmapped"
            )
            # No usage accrues behind the block overlay.
            self.assertEqual(heartbeat.get("focus"), [])
            self.assertIsNone(heartbeat.get("focus_since"))

            backend.focus(allowed)
            backend.sync()
            heartbeat = self.wait_for_heartbeat(
                lambda value: value.get("blocked") is False
            )
            # The focused allowed app is reported for usage limits.
            self.assertIn("allowedtest", heartbeat.get("focus", []))
            self.assertIsNotNone(heartbeat.get("focus_since"))

            backend.map_window(blocked)
            backend.focus(blocked)
            backend.sync()
            heartbeat = self.wait_for_heartbeat(
                lambda value: value.get("blocked") is True
            )
            self.assertEqual(heartbeat.get("focus"), [])
            self.assertGreaterEqual(heartbeat.get("blocks", 0), 2)
            self.assertEqual(heartbeat.get("adapter"), "x11")
            self.assertEqual(heartbeat.get("display"), self.display)
            # Card placement and session awareness are reported so the daemon
            # and the UI can explain what the adapter sees. Xvfb presents a
            # single monitor, so the card belongs there.
            self.assertEqual(heartbeat.get("monitors"), 1)
            self.assertEqual(heartbeat.get("card_monitor"), 0)
            self.assertIn("session_aware", heartbeat)
            self.assertIsInstance(heartbeat.get("session_aware"), bool)
            # This test must never lock or suspend the real session; it only
            # verifies that the adapter reports the session as usable.
            self.assertFalse(heartbeat.get("locked", True))
            self.assertFalse(heartbeat.get("suspended", True))

            # When the policy expires, enforcement stops and the window that
            # was unmapped must become reachable again instead of staying
            # hidden forever.
            now = datetime.now(timezone.utc)
            expiring = {
                "version": 1,
                "generated_at": now.isoformat(),
                "valid_until": (now + timedelta(seconds=2)).isoformat(),
                "apps": [
                    {
                        "app": "blockedtest",
                        "reason": "R-PROFILE",
                        "until": None,
                        "profile": "Focus",
                    }
                ],
            }
            (self.directory / "enforcement.json").write_text(
                json.dumps(expiring), encoding="utf-8"
            )
            self.wait_for_heartbeat(
                lambda value: value.get("blocked") == 0, timeout=12
            )
            self.assertTrue(
                backend.is_viewable(blocked),
                "the restored window must be mapped after the policy expires",
            )
        finally:
            backend.destroy_window(blocked)
            backend.destroy_window(allowed)
            backend.close()


if __name__ == "__main__":
    unittest.main()
