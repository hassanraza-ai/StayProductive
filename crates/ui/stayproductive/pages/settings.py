"""Settings: clock, desktop integration, data location, and capabilities."""

from __future__ import annotations

from pathlib import Path

from gi.repository import Adw, Gdk, Gio, Gtk

from .. import desktop
from .. import widgets as w
from ..formatting import error_text

GNOME_EXTENSION_UUID = desktop.GNOME_EXTENSION_UUID


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
        self.enforcement_row = Adw.ActionRow(
            title="Desktop enforcement",
            subtitle="Checking the local adapter…",
        )
        self.enforcement_pill = w.status_pill("Checking", "neutral")
        self.enforcement_row.add_suffix(self.enforcement_pill)
        capabilities.add(self.enforcement_row)

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

        integration = Adw.PreferencesGroup(
            title="Desktop integration",
            description=(
                "Native window blocking is supplied by a desktop adapter. The "
                "extension is local, reads a short-lived policy file, and never "
                "uses the network."
            ),
        )
        self.extension_row = Adw.ActionRow(
            title="GNOME Shell extension",
            subtitle="Checking installation…",
        )
        self.extension_button = w.button(
            "Install",
            css="flat",
            tooltip="Install or update the local GNOME Shell integration",
            on_click=lambda _button: self._install_or_enable_extension(),
        )
        self.extension_row.add_suffix(self.extension_button)
        integration.add(self.extension_row)

        self.policy_row = Adw.ActionRow(title="Enforcement policy", subtitle="—")
        self.policy_row.add_suffix(
            w.button(
                icon="edit-copy-symbolic",
                css="flat",
                tooltip="Copy policy path",
                on_click=lambda _button: self._copy_policy_path(),
            )
        )
        integration.add(self.policy_row)

        wayland_note = Adw.ActionRow(
            title="Native Wayland and XWayland",
            subtitle=(
                "The extension covers application windows while it is enabled. "
                "GNOME Shell only loads a newly installed extension after a "
                "restart, so log out and back in once after installing it."
            ),
        )
        integration.add(wayland_note)

        self.extension_note = Adw.ActionRow(
            title="Enforcement stays local",
            subtitle=(
                "The policy expires after 30 seconds unless the app refreshes "
                "it. A disabled or crashed extension stops claiming enforcement "
                "within five seconds."
            ),
        )
        integration.add(self.extension_note)
        self.page.add(integration)

        help_group = Adw.PreferencesGroup(
            title="Help",
            description="Revisit the setup steps at any time. Nothing is reset.",
        )
        onboarding_row = Adw.ActionRow(
            title="Run onboarding again",
            subtitle=(
                "Review the purpose, choose another goal, and create a starter "
                "profile. Existing profiles and commitments are left alone."
            ),
        )
        onboarding_row.add_suffix(
            w.button(
                "Start",
                css="flat",
                tooltip="Open the first-run wizard again",
                on_click=lambda _button: self.window.show_onboarding(first_run=False),
            )
        )
        help_group.add(onboarding_row)
        help_group.add(
            Adw.ActionRow(
                title="Not a parental-control tool",
                subtitle=(
                    "You decide every rule. There is no account, purchase, "
                    "telemetry, or hidden exit."
                ),
            )
        )
        self.page.add(help_group)

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

        enforcement = snapshot.get("enforcement", {})
        active = enforcement.get("state") == "active"
        self.enforcement_row.set_subtitle(
            enforcement.get("message", "No enforcement adapter is reporting.")
        )
        self._set_pill(
            self.enforcement_pill,
            "Active" if active else "Offline",
            "active" if active else "warning",
        )
        self.policy_row.set_subtitle(enforcement.get("policy_path", "—"))
        self._refresh_extension(active)

    def _refresh_extension(self, enforcement_active: bool) -> None:
        installed = desktop.extension_installed()
        enabled = desktop.extension_enabled() if installed else False

        if not desktop.is_gnome():
            self.extension_row.set_subtitle(
                "This session is not GNOME. The extension is not used here, and "
                "no other desktop adapter is installed in this build."
            )
            self.extension_button.set_label("Unavailable")
            self.extension_button.set_sensitive(False)
            return

        self.extension_button.set_sensitive(True)
        if not installed:
            self.extension_row.set_subtitle(
                "Not installed · install it to block native Wayland and "
                "XWayland application windows."
            )
            self.extension_button.set_label("Install")
        elif not enabled:
            self.extension_row.set_subtitle(
                "Installed, not enabled · enable it, then log out and back in once."
            )
            self.extension_button.set_label("Enable")
        elif enforcement_active:
            self.extension_row.set_subtitle(
                "Enabled and reporting · application blocks are being applied now."
            )
            self.extension_button.set_label("Reinstall")
        else:
            self.extension_row.set_subtitle(
                "Enabled, waiting for GNOME Shell · log out and back in once after "
                "installation."
            )
            self.extension_button.set_label("Reinstall")

    def _extension_enabled(self) -> bool:
        return desktop.extension_enabled()

    def _set_pill(self, pill: Gtk.Label, text: str, kind: str) -> None:
        for value in ("active", "warning", "neutral", "accent", "idle"):
            pill.remove_css_class(f"pill-{value}")
        pill.add_css_class(f"pill-{kind}")
        pill.set_label(text.upper())

    def _install_or_enable_extension(self) -> None:
        try:
            desktop.copy_extension()
        except desktop.DesktopError as error:
            heading = (
                "Extension files are missing"
                if desktop.extension_source() is None
                else "Could not install the extension"
            )
            self.window.show_error(heading, str(error))
            return

        if not desktop.extension_enabled():
            try:
                desktop.enable_extension()
            except desktop.DesktopError as error:
                self.window.show_error(
                    "Extension installed; enable it manually",
                    "Open Extensions or run: gnome-extensions enable "
                    f"{GNOME_EXTENSION_UUID}\n\n{error_text(error)}",
                )
                return

        if self.window.snapshot.get("enforcement", {}).get("state") == "active":
            message = "GNOME extension updated"
        else:
            message = "GNOME extension installed · log out and back in once"
        self.window.toast(message)
        self.window.refresh()

    def _copy_policy_path(self) -> None:
        path = self.window.snapshot.get("enforcement", {}).get("policy_path")
        if not path:
            return
        display = Gdk.Display.get_default()
        if display is not None:
            display.get_clipboard().set(path)
            self.window.toast("Policy path copied")

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
