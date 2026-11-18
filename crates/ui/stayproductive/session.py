"""Session lock and suspend awareness for the X11 adapter.

Through systemd-logind the adapter learns that the screen locked or the system
is suspending. While either is true it releases the overlay so the explanation
never covers a lock screen or a security prompt, and it ignores focus changes
so the lock screen cannot be mistaken for an allowed application. Everything
else keeps running: the blocked window stays hidden and the block resumes when
the session is usable again.
"""

from __future__ import annotations

import time

import gi

gi.require_version("Gio", "2.0")

from gi.repository import Gio, GLib  # noqa: E402

REFRESH_SECONDS = 2.0


class SessionMonitor:
    """Watches logind for screen lock and suspend, with a polling fallback.

    ``Lock``/``Unlock``/``PrepareForSleep`` signals give an immediate reaction;
    the ``LockedHint`` property is re-read periodically because signals can be
    missed or arrive late.
    """

    def __init__(self, on_change) -> None:
        self.on_change = on_change
        self.available = False
        self.locked = False
        self.suspended = False
        self._session = None
        self._last_refresh = 0.0

    def start(self) -> bool:
        """Connect to logind. Returns False when it is not reachable."""

        try:
            manager = Gio.DBusProxy.new_for_bus_sync(
                Gio.BusType.SYSTEM,
                Gio.DBusProxyFlags.NONE,
                None,
                "org.freedesktop.login1",
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
                None,
            )
            manager.connect("g-signal", self._on_manager_signal)
            self._session = Gio.DBusProxy.new_for_bus_sync(
                Gio.BusType.SYSTEM,
                Gio.DBusProxyFlags.NONE,
                None,
                "org.freedesktop.login1",
                "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session",
                None,
            )
            self._session.connect("g-signal", self._on_session_signal)
            self.locked = self._locked_hint()
            self.available = True
        except GLib.Error:
            self.available = False
            self._session = None
        return self.available

    def refresh(self, force: bool = False) -> None:
        """Re-read the lock state; throttled because it is a bus round trip."""

        if self._session is None:
            return
        now = time.monotonic()
        if not force and now - self._last_refresh < REFRESH_SECONDS:
            return
        self._last_refresh = now
        locked = self._locked_hint()
        if locked != self.locked:
            self.locked = locked
            self.on_change()

    def _locked_hint(self) -> bool:
        if self._session is None:
            return False
        try:
            result = self._session.call_sync(
                "org.freedesktop.DBus.Properties.Get",
                GLib.Variant(
                    "(ss)",
                    ("org.freedesktop.login1.Session", "LockedHint"),
                ),
                Gio.DBusCallFlags.NONE,
                -1,
                None,
            )
            return bool(result.unpack()[0])
        except GLib.Error:
            return False

    def _on_manager_signal(self, _proxy, _sender, signal, params) -> None:
        if signal == "PrepareForSleep":
            values = params.unpack()
            if values:
                self.suspended = bool(values[0])
                self.on_change()

    def _on_session_signal(self, _proxy, _sender, signal, params) -> None:
        if signal == "Lock":
            self.locked = True
            self.on_change()
        elif signal == "Unlock":
            self.locked = False
            self.on_change()
        elif signal == "PropertiesChanged":
            values = params.unpack()
            changed = values[1] if len(values) > 1 else {}
            if isinstance(changed, dict) and "LockedHint" in changed:
                self.locked = bool(changed["LockedHint"])
                self.on_change()
