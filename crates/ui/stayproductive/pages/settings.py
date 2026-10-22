"""Settings: clock, data location, and honest capability status."""

from __future__ import annotations

from pathlib import Path

from gi.repository import Adw, Gdk, Gio, Gtk

from .. import widgets as w
from ..formatting import error_text


class SettingsPage:
    def __init__(self, window) -> None:
        self.window = window
        self.page = Adw.PreferencesPage()
        self.page.set_vexpand(True)
        self._build()
        self.widget = self.page

    def _build(self) -> None:
        general = Adw.PreferencesGroup(
            title="Clock",
            description="Schedules and daily limits are evaluated against these settings.",
        )
        self.start_entry = Adw.EntryRow(title="Start of day")
        self.start_entry.set_text("00:00")
        self.start_entry.set_tooltip_text("24-hour time, for example 04:00")
        self.start_entry.add_suffix(self._apply_button())
        general.add(self.start_entry)

        self.timezone_entry = Adw.EntryRow(title="Time zone")
        self.timezone_entry.set_text("UTC")
        self.timezone_entry.set_tooltip_text("IANA time zone, for example Europe/Berlin")
        self.timezone_entry.add_suffix(self._apply_button())
        general.add(self.timezone_entry)
        self.page.add(general)

        capabilities = Adw.PreferencesGroup(
            title="Capabilities",
            description="StayProductive reports what this build can actually do.",
        )
        enforcement = Adw.ActionRow(
            title="Desktop enforcement",
            subtitle="Not connected · the GNOME adapter is not built yet",
        )
        enforcement.add_suffix(w.status_pill("Offline", "warning"))
        capabilities.add(enforcement)

        service = Adw.ActionRow(
            title="Local state service",
            subtitle="Connected · decisions and persistence are live",
        )
        service.add_suffix(w.status_pill("Running", "active"))
        capabilities.add(service)

        self.data_row = Adw.ActionRow(title="Data file", subtitle="—")
        copy = w.button(
            icon="edit-copy-symbolic",
            css="flat",
            tooltip="Copy path",
            on_click=lambda _button: self._copy_path(),
        )
        self.data_row.add_suffix(copy)
        capabilities.add(self.data_row)

        open_folder = Adw.ActionRow(
            title="Open data folder",
            subtitle="Local profiles, commitments, and records",
            activatable=True,
        )
        open_folder.add_suffix(Gtk.Image.new_from_icon_name("folder-open-symbolic"))
        open_folder.connect("activated", lambda _row: self._open_folder())
        capabilities.add(open_folder)
        self.page.add(capabilities)

        about = Adw.PreferencesGroup(title="About")
        version = Adw.ActionRow(
            title="StayProductive",
            subtitle="0.1.0 · clean-room Linux build",
        )
        version.add_suffix(w.status_pill("Local", "neutral"))
        about.add(version)
        about.add(
            Adw.ActionRow(
                title="Design",
                subtitle="Rust core, GTK4 and libadwaita frontend, local JSON state",
            )
        )
        about.add(
            Adw.ActionRow(
                title="Privacy",
                subtitle="No account, telemetry, or network requirement",
            )
        )
        about.add(Adw.ActionRow(title="License", subtitle="Apache-2.0"))
        self.page.add(about)

    def _apply_button(self) -> Gtk.Button:
        return w.button(
            "Apply",
            css="flat",
            on_click=lambda _button: self._apply_clock(),
        )

    def refresh(self, snapshot: dict) -> None:
        settings = snapshot.get("settings", {})
        if not self.start_entry.has_focus():
            self.start_entry.set_text(settings.get("start_of_day", "00:00"))
        if not self.timezone_entry.has_focus():
            self.timezone_entry.set_text(settings.get("timezone", "UTC"))
        self.data_row.set_subtitle(snapshot.get("state_path", "—"))

    def _apply_clock(self) -> None:
        start = self.start_entry.get_text().strip() or "00:00"
        timezone = self.timezone_entry.get_text().strip() or "UTC"
        try:
            self.window.engine.set_settings(start, timezone)
            self.window.refresh()
            self.window.toast("Clock settings saved")
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not save clock settings", error_text(error))

    def _copy_path(self) -> None:
        path = self.window.snapshot.get("state_path")
        if not path:
            return
        display = Gdk.Display.get_default()
        if display is not None:
            display.get_clipboard().set(path)
            self.window.toast("Data path copied")

    def _open_folder(self) -> None:
        path = self.window.snapshot.get("state_path")
        if not path:
            return
        parent = Path(path).parent
        try:
            Gio.AppInfo.launch_default_for_uri(
                Gio.File.new_for_path(str(parent)).get_uri(), None
            )
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not open the data folder", error_text(error))
