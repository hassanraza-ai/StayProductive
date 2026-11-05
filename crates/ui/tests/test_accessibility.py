"""Keyboard and screen-reader checks for the GTK layer (`A-UX-10`).

Widgets need a display, so this module starts a throwaway Xvfb server the same
way `test_x11_live.py` does for the adapter, and skips when Xvfb and a display
are both missing. It asserts what GTK can answer without a screen reader: every
button carries an accessible name, every sidebar row is labelled and reachable
by keyboard, the page accelerators follow the sidebar order, and the decorative
status dot is not announced.

It builds the real `MainWindow` against a fake engine: no daemon, socket, or
user data is touched. A real Orca/high-contrast pass is still human work and is
listed as such in `docs/spec/implementation-notes.md`.

    PYTHONPATH=crates/ui python3 -m unittest crates.ui.tests.test_accessibility
"""

from __future__ import annotations

import os
import shutil
import subprocess
import time
import unittest
from pathlib import Path
from unittest import mock

_xvfb: subprocess.Popen | None = None
_modules: dict = {}
_saved_env: dict[str, str | None] = {}

SNAPSHOT = {
    "now": "2026-11-04T12:00:00Z",
    "settings": {"start_of_day": "00:00", "timezone": "UTC"},
    "profiles": [],
    "active_profiles": [],
    "manual_active": None,
    "commitment": None,
    "quick_blocks": [],
    "history": [],
    "usage_today": [],
    "enforcement": {},
    "state_path": "/tmp/stayproductive-a11y/state.json",
}


def _start_display() -> tuple[str, subprocess.Popen] | None:
    for number in range(150, 180):
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
    return None


def setUpModule() -> None:
    global _xvfb
    for key in ("DISPLAY", "WAYLAND_DISPLAY", "GDK_BACKEND"):
        _saved_env[key] = os.environ.get(key)
    display = None
    if shutil.which("Xvfb") is not None:
        started = _start_display()
        if started is not None:
            display, _xvfb = started
            os.environ["GDK_BACKEND"] = "x11"
            os.environ.pop("WAYLAND_DISPLAY", None)
    if display is None:
        display = os.environ.get("DISPLAY")
    if not display:
        raise unittest.SkipTest("no display and no Xvfb")
    os.environ["DISPLAY"] = display

    import gi

    gi.require_version("Gtk", "4.0")
    gi.require_version("Adw", "1")
    from gi.repository import Adw, Gtk

    from stayproductive import widgets as w
    from stayproductive.window import MainWindow, PAGES, SHORTCUTS, reduced_motion

    Adw.init()
    Gtk.init()
    _modules.update(
        Adw=Adw,
        Gtk=Gtk,
        w=w,
        MainWindow=MainWindow,
        PAGES=PAGES,
        SHORTCUTS=SHORTCUTS,
        reduced_motion=reduced_motion,
    )


def tearDownModule() -> None:
    if _xvfb is not None:
        _xvfb.kill()
        _xvfb.wait()
    for key, value in _saved_env.items():
        if value is None:
            os.environ.pop(key, None)
        else:
            os.environ[key] = value


class FakeEngine:
    """The window only calls `snapshot()`; every write stays out of this test."""

    def __init__(self, data: dict) -> None:
        self.data = dict(data)

    def snapshot(self) -> dict:
        return dict(self.data)


def walk(node):
    """Walk a widget tree, including dialogs that expose `get_child()` only."""

    yield node
    Gtk = _modules.get("Gtk")
    children = []
    if Gtk is not None and isinstance(node, Gtk.Widget):
        child = node.get_first_child()
        while child is not None:
            children.append(child)
            child = child.get_next_sibling()
    if not children and hasattr(node, "get_child"):
        child = node.get_child()
        if child is not None:
            children.append(child)
    for child in children:
        yield from walk(child)


class MainWindowAccessibilityTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        Gtk = _modules["Gtk"]
        Adw = _modules["Adw"]
        cls.Gtk = Gtk
        cls.w = _modules["w"]
        cls.PAGES = _modules["PAGES"]
        # Buttons that GTK or libadwaita build inside a standard control
        # (a spin button's arrows, an entry row's apply button, a menu button's
        # own toggle, a drop-down's arrow) are named by that control, not by
        # this application. The audit covers the controls we place.
        cls.composite_ancestors = (
            Gtk.DropDown,
            Gtk.SpinButton,
            Gtk.MenuButton,
            Gtk.SearchEntry,
            Adw.EntryRow,
            Adw.PasswordEntryRow,
            Adw.SpinRow,
        )
        cls.application = Adw.Application(
            application_id="io.github.stayproductive.A11yTest"
        )
        cls.application.register(None)
        cls.service = mock.patch(
            "stayproductive.pages.settings.service.service_state",
            return_value="unavailable",
        )
        cls.service.start()
        cls.engine = FakeEngine(SNAPSHOT)
        cls.window = _modules["MainWindow"](cls.application, cls.engine)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.service.stop()
        cls.window.destroy()

    def setUp(self) -> None:
        self.engine.data = dict(SNAPSHOT)

    # ------------------------------------------------------------- widget layer
    def _describe(self, widget) -> str:
        text = widget.get_tooltip_text() or ""
        label = widget.get_label() if hasattr(widget, "get_label") else ""
        return f"{type(widget).__name__}({label or text or 'no text'})"

    def _inside_composite(self, widget) -> bool:
        parent = widget.get_parent()
        while parent is not None:
            if isinstance(parent, self.composite_ancestors):
                return True
            parent = parent.get_parent()
        return False

    def test_every_button_exposes_an_accessible_name(self) -> None:
        self._assert_controls_named(self.window, minimum=10)

    def test_the_profile_editor_and_onboarding_are_labelled_too(self) -> None:
        from stayproductive.editor import ProfileEditor
        from stayproductive.onboarding import Onboarding

        editor = ProfileEditor(self.window, None)
        onboarding = Onboarding(self.window, first_run=True)
        try:
            self._assert_controls_named(editor, minimum=5)
            self._assert_controls_named(onboarding, minimum=5)
        finally:
            editor.force_close()
            onboarding.force_close()

    def _assert_controls_named(self, root, *, minimum: int) -> None:
        Gtk = self.Gtk
        checked = 0
        for widget in walk(root):
            if not isinstance(widget, (Gtk.Button, Gtk.MenuButton)):
                continue
            if self._inside_composite(widget):
                continue
            checked += 1
            explicit = Gtk.test_accessible_has_property(
                widget, Gtk.AccessibleProperty.LABEL
            )
            visible = (
                bool(widget.get_label()) if isinstance(widget, Gtk.Button) else False
            )
            with self.subTest(widget=self._describe(widget)):
                self.assertTrue(
                    explicit or visible,
                    f"{self._describe(widget)} has no accessible label",
                )
                self.assertTrue(
                    widget.get_focusable(),
                    f"{self._describe(widget)} is not reachable by keyboard",
                )
        self.assertGreaterEqual(checked, minimum, "the surface should expose its buttons")

    def test_an_icon_only_button_cannot_be_unnamed(self) -> None:
        Gtk = self.Gtk
        with self.assertRaises(ValueError):
            self.w.button(icon="open-menu-symbolic")
        named = self.w.button(icon="open-menu-symbolic", tooltip="Main menu")
        self.assertTrue(
            Gtk.test_accessible_has_property(named, Gtk.AccessibleProperty.LABEL)
        )
        # A text+icon button uses a custom child, so GTK cannot derive a name
        # from it either; the helper must set one explicitly.
        mixed = self.w.button("New profile", icon="list-add-symbolic")
        self.assertTrue(
            Gtk.test_accessible_has_property(mixed, Gtk.AccessibleProperty.LABEL)
        )
        self.assertTrue(
            Gtk.test_accessible_has_property(mixed, Gtk.AccessibleProperty.DESCRIPTION)
            is False,
            "a tooltip that repeats the label is not a second description",
        )

    # -------------------------------------------------------------- navigation
    def test_nav_rows_are_labelled_and_focusable(self) -> None:
        Gtk = self.Gtk
        rows = [
            self.window.nav.get_row_at_index(index)
            for index in range(len(self.PAGES))
        ]
        for row, (key, title, _icon, _subtitle) in zip(rows, self.PAGES):
            self.assertEqual(getattr(row, "key", None), key)
            with self.subTest(page=key):
                self.assertTrue(
                    Gtk.test_accessible_has_property(
                        row, Gtk.AccessibleProperty.LABEL
                    ),
                    f"the {title} row has no accessible label",
                )
                self.assertTrue(
                    Gtk.test_accessible_has_property(
                        row, Gtk.AccessibleProperty.DESCRIPTION
                    ),
                    f"the {title} row does not describe what the page is for",
                )
                self.assertTrue(row.get_focusable(), f"the {title} row is unreachable")

    def test_page_actions_follow_the_sidebar_order(self) -> None:
        group = self.window.nav_actions
        for index, (key, title, _icon, _subtitle) in enumerate(self.PAGES, start=1):
            action = f"nav.page-{key}"
            self.assertEqual(
                self.application.get_accels_for_action(action),
                [f"<Control>{index}"],
                f"{title} should be Control+{index}",
            )
            self.assertTrue(group.has_action(f"page-{key}"))
            self.window.activate_action(action, None)
            self.assertEqual(self.window.stack.get_visible_child_name(), key)
            self.assertEqual(self.window.title_widget.get_title(), title)

    def test_new_profile_and_shortcut_help_are_bound(self) -> None:
        group = self.window.nav_actions
        self.assertEqual(
            self.application.get_accels_for_action("nav.new-profile"), ["<Control>n"]
        )
        self.assertEqual(
            self.application.get_accels_for_action("nav.shortcuts"),
            ["<Control>question"],
        )
        self.assertTrue(group.has_action("new-profile"))
        self.assertTrue(group.has_action("shortcuts"))

    # ------------------------------------------------------------------ status
    def test_the_status_dot_is_not_announced(self) -> None:
        Gtk = self.Gtk
        self.assertEqual(
            self.window.enforcement_dot.get_accessible_role(),
            Gtk.AccessibleRole.PRESENTATION,
        )
        self.engine.data = dict(SNAPSHOT, enforcement={"state": "active", "adapter": "x11"})
        self.window.refresh()
        self.assertEqual(self.window.enforcement_label.get_label(), "Enforcement active")
        self.assertEqual(
            self.window.enforcement_dot.get_accessible_role(),
            Gtk.AccessibleRole.PRESENTATION,
            "the dot stays decorative while the text changes",
        )

    # -------------------------------------------------------------- motion
    def test_reduced_motion_removes_the_page_transition(self) -> None:
        Gtk = self.Gtk
        settings = Gtk.Settings.get_default()
        original = settings.get_property("gtk-enable-animations")
        settings.set_property("gtk-enable-animations", False)
        try:
            self.assertTrue(_modules["reduced_motion"]())
            quiet = _modules["MainWindow"](
                self.application, FakeEngine(SNAPSHOT)
            )
            try:
                self.assertEqual(quiet.stack.get_transition_duration(), 0)
            finally:
                quiet.destroy()
        finally:
            settings.set_property("gtk-enable-animations", original)
