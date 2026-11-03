"""Tests for the systemd user-service helpers; no systemd required."""

from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from stayproductive import paths, service


class PathsTest(unittest.TestCase):
    def test_fnv_matches_the_daemon(self) -> None:
        self.assertEqual(paths.fnv1a_hex(b""), "cbf29ce484222325")
        self.assertEqual(paths.fnv1a_hex(b"abc"), "e71fa2190541574b")

    def test_custom_state_gets_its_own_socket(self) -> None:
        with mock.patch.dict(
            os.environ, {"XDG_RUNTIME_DIR": "/run/user/1"}, clear=False
        ):
            os.environ.pop("STAYPRODUCTIVE_SOCKET", None)
            os.environ.pop("STAYPRODUCTIVE_STATE", None)
            plain = paths.default_socket_path()
            custom = paths.default_socket_path("/tmp/other-state.json")
        self.assertEqual(plain, Path("/run/user/1/stayproductive/daemon.sock"))
        self.assertEqual(
            custom,
            Path(
                f"/run/user/1/stayproductive/daemon-{paths.fnv1a_hex(b'/tmp/other-state.json')}.sock"
            ),
        )
        self.assertNotEqual(plain, custom)

    def test_socket_override_wins(self) -> None:
        with mock.patch.dict(
            os.environ, {"STAYPRODUCTIVE_SOCKET": "/tmp/explicit.sock"}, clear=False
        ):
            self.assertEqual(
                paths.default_socket_path("/tmp/state.json"), Path("/tmp/explicit.sock")
            )


class UnitFileTest(unittest.TestCase):
    def test_exec_start_is_quoted_for_systemd(self) -> None:
        self.assertEqual(service.quote_exec("/usr/bin/plain"), '"/usr/bin/plain"')
        self.assertEqual(
            service.quote_exec('/opt/My "App"/daemon'),
            '"/opt/My \\"App\\"/daemon"',
        )
        self.assertEqual(
            service.quote_exec("/back\\slash"), '"/back\\\\slash"'
        )

    def test_unit_starts_the_socket_daemon_and_restarts_it(self) -> None:
        text = service.unit_text("/opt/stayproductive/bin/stayproductive-daemon")
        self.assertIn(
            'ExecStart="/opt/stayproductive/bin/stayproductive-daemon" --socket',
            text,
        )
        self.assertIn("Restart=always", text)
        self.assertIn("NoNewPrivileges=yes", text)
        self.assertIn("WantedBy=default.target", text)
        self.assertNotIn("Environment=", text)

    def test_unit_carries_the_ui_data_home(self) -> None:
        text = service.unit_text(
            "/opt/stayproductive/bin/stayproductive-daemon", "/home/user/.local/share"
        )
        self.assertIn(
            'Environment="XDG_DATA_HOME=/home/user/.local/share"', text
        )

    def test_unit_path_honours_xdg_config_home(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            with mock.patch.dict(
                os.environ, {"XDG_CONFIG_HOME": temp}, clear=False
            ):
                self.assertEqual(
                    service.unit_path(),
                    Path(temp) / "systemd" / "user" / "stayproductive.service",
                )

    def test_daemon_override_is_used(self) -> None:
        with mock.patch.dict(
            os.environ, {"STAYPRODUCTIVE_DAEMON": "/tmp/fake-daemon"}, clear=False
        ):
            self.assertEqual(service.daemon_binary(), "/tmp/fake-daemon")


class ServiceStateTest(unittest.TestCase):
    def test_reports_unavailable_without_systemd(self) -> None:
        with mock.patch.object(service, "systemctl_available", return_value=False):
            self.assertEqual(service.service_state(), "unavailable")

    def test_custom_state_is_not_managed(self) -> None:
        with (
            mock.patch.object(service, "systemctl_available", return_value=True),
            mock.patch.object(service, "user_manager_running", return_value=True),
        ):
            self.assertEqual(
                service.service_state("/tmp/somewhere-else/state.json"),
                "custom-state",
            )

    def test_enabled_and_installed_states(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            state = Path(temp) / "state.json"
            config = Path(temp) / "config"
            with (
                mock.patch.dict(
                    os.environ,
                    {
                        "XDG_CONFIG_HOME": str(config),
                        "XDG_DATA_HOME": str(Path(temp) / "data"),
                    },
                    clear=False,
                ),
                mock.patch.object(service, "systemctl_available", return_value=True),
                mock.patch.object(service, "user_manager_running", return_value=True),
            ):
                os.environ.pop("STAYPRODUCTIVE_STATE", None)
                default_state = paths.default_state_path()
                with mock.patch.object(service, "is_enabled", return_value=False):
                    self.assertEqual(service.service_state(default_state), "off")
                    service.unit_path().parent.mkdir(parents=True, exist_ok=True)
                    service.unit_path().write_text("[Unit]\n")
                    self.assertEqual(
                        service.service_state(default_state), "installed"
                    )
                with mock.patch.object(service, "is_enabled", return_value=True):
                    self.assertEqual(
                        service.service_state(default_state), "enabled"
                    )
            self.assertNotEqual(state, default_state)

    def test_enable_hands_over_a_directly_spawned_daemon(self) -> None:
        engine = mock.Mock()
        calls: list[str] = []

        def systemctl(*args: str, timeout: float = 10.0):
            calls.append(args[0])
            return mock.Mock(returncode=0, stdout="", stderr="")

        with (
            mock.patch.object(service, "service_state", return_value="off"),
            mock.patch.object(service, "is_active", return_value=False),
            mock.patch.object(service, "write_unit", return_value=True),
            mock.patch.object(service, "_systemctl", side_effect=systemctl),
        ):
            service.enable(engine, "/tmp/state.json")
        self.assertEqual(engine.shutdown_service.call_count, 1)
        self.assertEqual(calls, ["enable", "start"])

    def test_disable_keeps_the_current_session_running(self) -> None:
        calls: list[tuple[str, ...]] = []

        def systemctl(*args: str, timeout: float = 10.0):
            calls.append(args)
            return mock.Mock(returncode=0, stdout="", stderr="")

        with mock.patch.object(service, "systemctl_available", return_value=True):
            with mock.patch.object(service, "_systemctl", side_effect=systemctl):
                service.disable()
        self.assertEqual(calls, [("disable", service.UNIT_NAME)])

    def test_enable_surfaces_systemctl_errors(self) -> None:
        with (
            mock.patch.object(service, "service_state", return_value="off"),
            mock.patch.object(service, "is_active", return_value=True),
            mock.patch.object(service, "write_unit", return_value=True),
            mock.patch.object(
                service,
                "_systemctl",
                return_value=mock.Mock(
                    returncode=1, stdout="", stderr="Unit is masked."
                ),
            ),
        ):
            with self.assertRaises(service.ServiceError) as caught:
                service.enable()
        self.assertIn("masked", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
