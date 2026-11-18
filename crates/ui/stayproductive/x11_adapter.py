"""Standalone X11 enforcement adapter.

The daemon owns every blocking decision and publishes a short-lived policy. This
process only observes the focused X11 window, matches it against that policy, and
performs the desktop operations the policy asks for: minimise the blocked window,
cover every monitor with an explanation, and offer a way back to the previously
allowed window.

It is a separate process on purpose. Closing the GTK front end must not stop
enforcement; the daemon starts and supervises this adapter when the session is
X11. The adapter writes a short-lived heartbeat so the daemon can report an
honest ``active`` or ``unavailable`` state.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")

from gi.repository import Gdk, Gio, GLib, Gtk  # noqa: E402

try:
    gi.require_version("GdkX11", "4.0")
    from gi.repository import GdkX11  # noqa: E402,F401
except (ImportError, ValueError):
    GdkX11 = None  # type: ignore[assignment]

from . import paths  # noqa: E402
from .session import SessionMonitor  # noqa: E402
from .x11 import X11, WindowGeometry  # noqa: E402
from .x11_policy import (  # noqa: E402
    Action,
    FocusEvent,
    FocusReporter,
    Planner,
    Policy,
    PolicyEntry,
    parse_timestamp,
    policy_from_json,
    preferred_monitor,
    reason_text,
)

POLL_MS = 150
HEARTBEAT_SECONDS = 1.0
PARENT_CHECK_SECONDS = 2.0
HEARTBEAT_FILE = "x11-adapter.json"

CSS = """
window.sp-overlay-window { background-color: #0e1015; }
box.sp-overlay { background-color: #0e1015; padding: 48px; }
label.sp-brand { color: #9aa4b2; font-size: 15px; font-weight: 700;
                 letter-spacing: 3px; }
label.sp-headline { color: #ffffff; font-size: 34px; font-weight: 800; }
label.sp-reason { color: #e6e9ef; font-size: 18px; }
label.sp-until { color: #9aa4b2; font-size: 15px; }
label.sp-privacy { color: #6b7480; font-size: 12px; }
button.sp-back { margin-top: 12px; }
"""


def utc_now() -> datetime:
    return datetime.now(timezone.utc)


def iso_timestamp(value: datetime | None = None) -> str:
    value = (value or utc_now()).astimezone(timezone.utc)
    return value.isoformat().replace("+00:00", "Z")


class PolicyCache:
    """Reads the daemon's policy file and drops it when it expires."""

    def __init__(self, path: Path) -> None:
        self.path = Path(path)
        self._key: tuple[int, int] | None = None
        self._policy: Policy | None = None

    def load(self, now: datetime | None = None) -> Policy | None:
        now = now or utc_now()
        try:
            stat = self.path.stat()
        except OSError:
            self._key = None
            self._policy = None
            return None
        key = (stat.st_mtime_ns, stat.st_size)
        if key != self._key:
            self._key = key
            try:
                raw = json.loads(self.path.read_text(encoding="utf-8"))
            except (OSError, ValueError) as error:
                self._policy = None
                print(
                    f"stayproductive-x11: ignoring unreadable policy: {error}",
                    file=sys.stderr,
                    flush=True,
                )
            else:
                self._policy = policy_from_json(raw, now)
        policy = self._policy
        if policy is None or policy.valid_until < now:
            return None
        return policy


def process_aliases(pid: int | None) -> list[str]:
    """Best-effort process names for windows with an unhelpful WM_CLASS.

    Only the executable name is read; no command-line arguments, URLs, or
    per-app history are stored or logged.
    """

    if not pid or pid <= 0:
        return []
    aliases: list[str] = []
    try:
        comm = Path(f"/proc/{pid}/comm").read_text(encoding="utf-8").strip()
        if comm:
            aliases.append(comm)
    except OSError:
        pass
    try:
        executable = os.readlink(f"/proc/{pid}/exe")
        name = os.path.basename(executable)
        if name:
            aliases.append(name)
    except OSError:
        pass
    try:
        raw = Path(f"/proc/{pid}/cmdline").read_bytes()
        if raw:
            first = raw.split(b"\x00", 1)[0].decode("utf-8", "replace")
            name = os.path.basename(first)
            if name:
                aliases.append(name)
    except OSError:
        pass
    return aliases


def window_identifiers(backend: X11, window: int) -> tuple[frozenset[str], str]:
    name, klass = backend.window_class(window)
    values: list[str | None] = [name, klass]
    for property_name in (
        "_GTK_APPLICATION_ID",
        "_GTK_APP_ID",
        "_BAMF_DESKTOP_FILE_HINT",
        "_NET_WM_DESKTOP_FILE",
    ):
        values.append(backend.property_string(window, property_name))
    values.extend(process_aliases(backend.window_pid(window)))
    label = klass or name or "This app"
    return frozenset(value for value in values if value), label


class Overlay:
    """One override-redirect window per monitor, with the block explanation."""

    def __init__(self, application: Gtk.Application, backend: X11, on_back) -> None:
        self.application = application
        self.backend = backend
        self.on_back = on_back
        self._windows: list[Gtk.Window] = []
        self._geometries: list[WindowGeometry] = []
        self._cards: list[Gtk.Widget] = []
        self._headline: list[Gtk.Label] = []
        self._reason: list[Gtk.Label] = []
        self._until: list[Gtk.Label] = []
        self.visible = False
        self.card_monitor = 0
        self._css_installed = False

    @property
    def monitor_count(self) -> int:
        # Before the first block no overlay window exists yet; report the
        # display's monitor count so the heartbeat is meaningful from startup.
        return len(self._windows) or len(self._monitor_geometries())

    @property
    def xids(self) -> set[int]:
        result: set[int] = set()
        for window in self._windows:
            surface = window.get_surface()
            if surface is not None:
                try:
                    result.add(surface.get_xid())
                except (AttributeError, TypeError):
                    pass
        return result

    def _install_css(self) -> None:
        if self._css_installed:
            return
        display = Gdk.Display.get_default()
        if display is None:
            return
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS.encode("utf-8"))
        Gtk.StyleContext.add_provider_for_display(
            display, provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
        )
        self._css_installed = True

    def _monitor_geometries(self) -> list[WindowGeometry]:
        display = Gdk.Display.get_default()
        if display is None:
            return []
        geometries = []
        monitors = display.get_monitors()
        for index in range(monitors.get_n_items()):
            monitor = monitors.get_item(index)
            rectangle = monitor.get_geometry()
            geometries.append(
                WindowGeometry(
                    x=rectangle.x,
                    y=rectangle.y,
                    width=max(1, rectangle.width),
                    height=max(1, rectangle.height),
                )
            )
        return geometries

    def _build_window(self, geometry: WindowGeometry) -> Gtk.Window:
        self._install_css()
        window = Gtk.Window(title="StayProductive")
        window.add_css_class("sp-overlay-window")
        window.set_decorated(False)
        window.set_resizable(False)
        window.set_can_focus(True)
        window.set_focusable(True)
        window.set_default_size(geometry.width, geometry.height)

        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16)
        box.add_css_class("sp-overlay")
        box.set_halign(Gtk.Align.CENTER)
        box.set_valign(Gtk.Align.CENTER)
        box.set_hexpand(True)
        box.set_vexpand(True)

        # The window is the dark cover for one monitor; the card holds the
        # explanation and is only shown on the monitor the blocked window was
        # on, so the user reads it where they were looking.
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16)
        box.append(card)

        brand = Gtk.Label(label="STAYPRODUCTIVE")
        brand.add_css_class("sp-brand")
        card.append(brand)

        headline = Gtk.Label(label="This app is blocked")
        headline.add_css_class("sp-headline")
        headline.set_wrap(True)
        headline.set_justify(Gtk.Justification.CENTER)
        headline.set_max_width_chars(44)
        card.append(headline)

        reason = Gtk.Label(label="")
        reason.add_css_class("sp-reason")
        reason.set_wrap(True)
        reason.set_justify(Gtk.Justification.CENTER)
        reason.set_max_width_chars(54)
        card.append(reason)

        until = Gtk.Label(label="")
        until.add_css_class("sp-until")
        card.append(until)

        back = Gtk.Button(label="Back to the previous app")
        back.add_css_class("sp-back")
        back.connect("clicked", lambda _button: self.on_back())
        card.append(back)

        privacy = Gtk.Label(
            label="Enforced locally. No account, network, or telemetry."
        )
        privacy.add_css_class("sp-privacy")
        card.append(privacy)

        window.set_child(box)

        controller = Gtk.EventControllerKey()
        controller.connect(
            "key-pressed",
            lambda _controller, keyval, _code, _state: self._on_key(keyval),
        )
        window.add_controller(controller)

        self._cards.append(card)
        self._headline.append(headline)
        self._reason.append(reason)
        self._until.append(until)
        return window

    def _on_key(self, keyval: int) -> bool:
        if keyval in (Gdk.KEY_Return, Gdk.KEY_KP_Enter, Gdk.KEY_space):
            self.on_back()
            return True
        return False

    def _ensure_windows(self) -> None:
        geometries = self._monitor_geometries()
        if not geometries:
            return
        if len(geometries) == len(self._windows) and geometries == self._geometries:
            return
        for window in self._windows:
            window.set_visible(False)
            window.destroy()
        self._windows = []
        self._geometries = geometries
        self._cards = []
        self._headline = []
        self._reason = []
        self._until = []
        for geometry in geometries:
            self._windows.append(self._build_window(geometry))

    def _prepare(self, window: Gtk.Window, geometry: WindowGeometry) -> None:
        window.realize()
        surface = window.get_surface()
        if surface is None:
            return
        try:
            xid = surface.get_xid()
        except (AttributeError, TypeError):
            return
        # Set override-redirect before the window is mapped so the window
        # manager does not decorate or reposition it. The GTK surface still
        # renders the explanation.
        self.backend.set_override_redirect(xid)
        self.backend.configure(xid, geometry)
        self.backend.raise_window(xid)

    def show(
        self,
        entry: PolicyEntry,
        label: str,
        until_text: str,
        window_id: int | None = None,
    ) -> None:
        self._ensure_windows()
        geometry = self.backend.root_geometry(window_id) if window_id else None
        card_index = preferred_monitor(geometry, self._geometries)
        if card_index < 0 or card_index >= len(self._windows):
            card_index = 0
        self.card_monitor = card_index
        headline_text = f"{label} is blocked"
        reason_text_value = reason_text(entry.reason)
        for index, window in enumerate(self._windows):
            if len(self._headline) == len(self._windows):
                self._headline[index].set_label(headline_text)
                self._reason[index].set_label(reason_text_value)
                self._until[index].set_label(until_text)
                self._cards[index].set_visible(index == card_index)
            if not window.get_visible():
                self._prepare(window, self._geometries[index])
                window.present()
            surface = window.get_surface()
            if surface is not None:
                try:
                    self.backend.raise_window(surface.get_xid())
                except (AttributeError, TypeError):
                    pass
        if self._windows:
            card_surface = self._windows[card_index].get_surface()
            if card_surface is not None:
                try:
                    self.backend.focus(card_surface.get_xid())
                except (AttributeError, TypeError):
                    pass
        self.visible = True

    def hide(self) -> None:
        for window in self._windows:
            window.set_visible(False)
        self.visible = False


class X11Adapter:
    def __init__(
        self,
        application: Gtk.Application,
        backend: X11,
        policy_path: Path,
        heartbeat_path: Path,
        *,
        parent_pid: int | None = None,
        poll_ms: int = POLL_MS,
    ) -> None:
        self.application = application
        self.backend = backend
        self.policy_cache = PolicyCache(policy_path)
        self.heartbeat_path = Path(heartbeat_path)
        self.parent_pid = parent_pid
        self.poll_ms = poll_ms
        self.planner = Planner()
        self.focus = FocusReporter()
        self.overlay = Overlay(application, backend, self.go_back)
        self.session = SessionMonitor(self._on_session_change)
        self._last_heartbeat = 0.0
        self._last_parent_check = 0.0
        self._paused = False
        self._stopping = False

    # ------------------------------------------------------------------ loop
    def start(self) -> None:
        self.backend.sync()
        self.session.start()
        self.write_heartbeat(policy_loaded=False)
        GLib.timeout_add(self.poll_ms, self.tick)

    def tick(self) -> bool:
        if self._stopping:
            return False
        now = utc_now()
        self._check_parent(now)
        self.session.refresh()
        self._apply_session_state()
        policy = self.policy_cache.load(now)
        if self._paused:
            # A locked or suspended session must not accrue app usage.
            self.focus.reset()
            if time.monotonic() - self._last_heartbeat >= HEARTBEAT_SECONDS:
                self.write_heartbeat(policy_loaded=policy is not None)
            return True
        focus = self.backend.active_window()
        if focus is not None and focus in self.overlay.xids:
            event = (
                self._event(self.planner.blocked_window, force_normal=True)
                if self.planner.blocked_window
                else None
            )
        elif focus is None:
            event = None
        else:
            event = self._event(focus)
        actions = self.planner.update(event, policy)
        self._execute(actions)
        # Report the focused app for usage limits. Behind the overlay no usage
        # accrues, so the interval is dropped and a new one starts later.
        self.focus.observe(event, now, blocked=self.planner.overlay_visible)
        if actions or time.monotonic() - self._last_heartbeat >= HEARTBEAT_SECONDS:
            self.write_heartbeat(policy_loaded=policy is not None)
        return True

    def _event(self, window: int | None, *, force_normal: bool = False) -> FocusEvent | None:
        if not window:
            return None
        identifiers, label = window_identifiers(self.backend, window)
        normal = force_normal or self.backend.is_application_window(window)
        return FocusEvent(
            window=window,
            identifiers=identifiers,
            label=label,
            normal=normal,
        )

    def _execute(self, actions: list[Action]) -> None:
        for action in actions:
            if action.kind == "hide_window" and action.window:
                self.backend.hide(action.window)
            elif action.kind == "show_overlay" and action.entry is not None:
                self.overlay.show(
                    action.entry,
                    action.label,
                    self._until_text(action.entry),
                    action.window,
                )
            elif action.kind == "hide_overlay":
                self.overlay.hide()
            elif action.kind == "restore_window" and action.window:
                # Map first: without a window manager the adapter unmapped the
                # window, and focusing an unmapped window does nothing.
                if self.backend.window_attributes(action.window) is not None:
                    self.backend.map_window(action.window)
                    self.backend.focus(action.window)
            elif action.kind == "focus_window" and action.window:
                self.backend.focus(action.window)

    @staticmethod
    def _until_text(entry: PolicyEntry) -> str:
        until = parse_timestamp(entry.until)
        if until is None:
            return "This block stays until the active profile or session ends."
        local = until.astimezone()
        return f"Blocked until {local.strftime('%H:%M')} local time."

    def go_back(self) -> None:
        self._execute(self.planner.back())
        self.write_heartbeat(policy_loaded=self.policy_cache.load() is not None)

    def _apply_session_state(self) -> None:
        """Pause enforcement while the screen is locked or the system suspends.

        The overlay must never cover a lock screen or a security prompt, and
        the lock screen's focus changes must not be read as an allowed app.
        The blocked window stays hidden and the block resumes on unlock.
        """

        paused = self.session.locked or self.session.suspended
        if paused == self._paused:
            return
        self._paused = paused
        if paused:
            self._execute(self.planner.pause())
        else:
            self.planner.resume()
        self.write_heartbeat(policy_loaded=self.policy_cache.load() is not None)

    def _on_session_change(self) -> None:
        """React to a logind signal between two polls."""

        self._apply_session_state()

    def _check_parent(self, now: datetime) -> None:
        if not self.parent_pid:
            return
        monotonic = time.monotonic()
        if monotonic - self._last_parent_check < PARENT_CHECK_SECONDS:
            return
        self._last_parent_check = monotonic
        if not Path(f"/proc/{self.parent_pid}").exists():
            self.application.quit()

    # -------------------------------------------------------------- heartbeat
    def write_heartbeat(self, *, policy_loaded: bool, active: bool = True) -> None:
        self._last_heartbeat = time.monotonic()
        payload: dict[str, Any] = {
            "adapter": "x11",
            "active": active,
            "policy_loaded": policy_loaded,
            "blocked": self.planner.overlay_visible,
            "overlay_visible": self.planner.overlay_visible,
            "blocks": self.planner.blocks,
            "focus": list(self.focus.aliases),
            "focus_since": (
                iso_timestamp(self.focus.since) if self.focus.since else None
            ),
            "monitors": self.overlay.monitor_count,
            "card_monitor": self.overlay.card_monitor,
            "locked": self.session.locked,
            "suspended": self.session.suspended,
            "session_aware": self.session.available,
            "display": os.environ.get("DISPLAY", ""),
            "checked_at": iso_timestamp(),
        }
        try:
            write_private_json(self.heartbeat_path, payload)
        except OSError as error:
            print(
                f"stayproductive-x11: could not write heartbeat: {error}",
                file=sys.stderr,
                flush=True,
            )

    def stop(self) -> None:
        if self._stopping:
            return
        self._stopping = True
        self.overlay.hide()
        self.write_heartbeat(policy_loaded=False, active=False)
        self.backend.close()
        self.application.release()


def write_private_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = path.with_name(path.name + ".tmp")
    descriptor = os.open(
        temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600
    )
    with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
        json.dump(payload, stream)
        stream.write("\n")
    os.replace(temporary, path)


def default_policy_path(state_path: Path) -> Path:
    return state_path.expanduser().resolve().parent / "enforcement.json"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="StayProductive X11 enforcement adapter"
    )
    parser.add_argument("--state", help="state file whose policy should be read")
    parser.add_argument("--policy", help="explicit enforcement policy path")
    parser.add_argument("--heartbeat", help="explicit heartbeat path")
    parser.add_argument("--parent-pid", type=int, default=0)
    parser.add_argument("--poll-ms", type=int, default=POLL_MS)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    state_text = args.state or os.environ.get("STAYPRODUCTIVE_STATE")
    state_path = (
        Path(state_text).expanduser()
        if state_text
        else paths.default_state_path().expanduser()
    )
    policy_path = (
        Path(args.policy).expanduser()
        if args.policy
        else default_policy_path(state_path)
    )
    heartbeat_path = (
        Path(args.heartbeat).expanduser()
        if args.heartbeat
        else paths.runtime_dir() / "stayproductive" / HEARTBEAT_FILE
    )
    if GdkX11 is None:
        print(
            "stayproductive-x11: this GTK build has no X11 backend.",
            file=sys.stderr,
            flush=True,
        )
        return 1
    try:
        backend = X11()
    except Exception as error:  # noqa: BLE001 - report a plain startup failure
        print(f"stayproductive-x11: {error}", file=sys.stderr, flush=True)
        return 1

    application = Gtk.Application(
        application_id="io.github.stayproductive.X11Adapter",
        flags=Gio.ApplicationFlags.NON_UNIQUE,
    )
    adapter: X11Adapter | None = None

    def activate(_application: Gtk.Application) -> None:
        nonlocal adapter
        _application.hold()
        adapter = X11Adapter(
            application,
            backend,
            policy_path,
            heartbeat_path,
            parent_pid=args.parent_pid or None,
            poll_ms=max(50, args.poll_ms),
        )
        adapter.start()

    def shutdown(_application: Gtk.Application) -> None:
        if adapter is not None:
            adapter.stop()

    def stop(*_args) -> bool:
        application.quit()
        return False

    application.connect("activate", activate)
    application.connect("shutdown", shutdown)
    for signal_number in (2, 15):
        GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal_number, stop)
    return application.run([sys.argv[0]])


if __name__ == "__main__":
    raise SystemExit(main())
