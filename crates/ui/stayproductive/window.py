"""Main window and navigation."""

from __future__ import annotations

from gi.repository import Adw, Gio, GLib, Gtk

from . import widgets as w
from .editor import ProfileEditor
from .engine import Engine, EngineError
from .formatting import error_text
from .pages.activity import ActivityPage
from .pages.dashboard import DashboardPage
from .pages.focus import FocusPage
from .pages.profiles import ProfilesPage
from .pages.schedules import SchedulesPage
from .pages.settings import SettingsPage

APP_TITLE = "StayProductive"

PAGES = [
    ("dashboard", "Dashboard", "view-grid-symbolic", "Today at a glance"),
    ("profiles", "Profiles", "view-list-symbolic", "Rules and activation"),
    ("schedules", "Schedules", "appointment-new-symbolic", "When focus applies"),
    ("focus", "Strict session", "channel-secure-symbolic", "Commitments and cool-down"),
    ("activity", "Activity", "document-open-recent-symbolic", "Local records"),
    ("settings", "Settings", "emblem-system-symbolic", "Clock and capabilities"),
]


class MainWindow(Adw.ApplicationWindow):
    def __init__(self, application: Adw.Application, engine: Engine) -> None:
        super().__init__(application=application, title=APP_TITLE)
        self.engine = engine
        self.snapshot: dict = {}
        self.editor: ProfileEditor | None = None
        self._error_shown = False
        self._build()
        self.refresh()
        GLib.timeout_add_seconds(5, self._tick)

    def _build(self) -> None:
        self.set_default_size(1200, 800)
        self.set_size_request(900, 600)
        self._install_css()

        self.toast_overlay = Adw.ToastOverlay()
        split = Adw.NavigationSplitView()
        split.set_min_sidebar_width(230)
        split.set_max_sidebar_width(300)
        split.set_sidebar_width_fraction(0.24)

        # --------------------------------------------------------- sidebar
        sidebar_toolbar = Adw.ToolbarView()
        sidebar_header = Adw.HeaderBar()
        sidebar_header.set_show_end_title_buttons(False)
        sidebar_header.set_show_start_title_buttons(False)
        sidebar_header.set_title_widget(
            Adw.WindowTitle(title=APP_TITLE, subtitle="Focus with intent")
        )
        sidebar_toolbar.add_top_bar(sidebar_header)
        sidebar_content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        sidebar_content.set_margin_top(10)
        sidebar_content.set_margin_bottom(12)
        sidebar_content.set_margin_start(8)
        sidebar_content.set_margin_end(8)

        self.nav = Gtk.ListBox()
        self.nav.add_css_class("navigation-sidebar")
        self.nav.set_selection_mode(Gtk.SelectionMode.SINGLE)
        for key, title, icon, _subtitle in PAGES:
            self.nav.append(w.nav_row(key, title, icon))
        self.nav.connect("row-selected", self._on_row_selected)
        sidebar_content.append(self.nav)

        spacer = Gtk.Box()
        spacer.set_vexpand(True)
        sidebar_content.append(spacer)

        status = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
        status.add_css_class("sidebar-status")
        status.set_margin_top(6)
        status.set_margin_bottom(6)
        status.set_margin_start(10)
        status.set_margin_end(10)
        status_line = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        self.enforcement_dot = Gtk.Label(label="●")
        self.enforcement_dot.add_css_class("status-dot-offline")
        status_line.append(self.enforcement_dot)
        self.enforcement_label = w.label("Enforcement offline", "row-title")
        status_line.append(self.enforcement_label)
        status.append(status_line)
        self.service_label = w.label("State service running", "muted small")
        status.append(self.service_label)
        sidebar_content.append(status)
        sidebar_toolbar.set_content(sidebar_content)
        sidebar_page = Adw.NavigationPage(title=APP_TITLE)
        sidebar_page.set_child(sidebar_toolbar)
        sidebar_page.set_size_request(250, -1)
        split.set_sidebar(sidebar_page)

        # --------------------------------------------------------- content
        content_toolbar = Adw.ToolbarView()
        self.header = Adw.HeaderBar()
        self.title_widget = Adw.WindowTitle(title="Dashboard", subtitle="Today at a glance")
        self.header.set_title_widget(self.title_widget)
        self.header_actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        self.header.pack_end(self.header_actions)
        self.header.pack_end(self._global_menu())
        content_toolbar.add_top_bar(self.header)

        self.stack = Gtk.Stack()
        self.stack.set_transition_type(Gtk.StackTransitionType.CROSSFADE)
        self.stack.set_transition_duration(160)
        self.stack.set_hhomogeneous(False)
        self.stack.set_vhomogeneous(False)
        self.stack.set_vexpand(True)

        self.dashboard = DashboardPage(self)
        self.profiles = ProfilesPage(self)
        self.schedules = SchedulesPage(self)
        self.focus = FocusPage(self)
        self.activity = ActivityPage(self)
        self.settings = SettingsPage(self)
        self.page_widgets = {
            "dashboard": self.dashboard,
            "profiles": self.profiles,
            "schedules": self.schedules,
            "focus": self.focus,
            "activity": self.activity,
            "settings": self.settings,
        }
        for key, title, _icon, _subtitle in PAGES:
            self.stack.add_named(self.page_widgets[key].widget, key)
        content_toolbar.set_content(self.stack)
        content_page = Adw.NavigationPage(title=APP_TITLE)
        content_page.set_child(content_toolbar)
        split.set_content(content_page)
        self.toast_overlay.set_child(split)
        self.set_content(self.toast_overlay)

        GLib.idle_add(self._select_initial)

    def _select_initial(self) -> bool:
        self.nav.select_row(self.nav.get_row_at_index(0))
        return False

    def _global_menu(self) -> Gtk.MenuButton:
        menu = Gtk.Popover()
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        box.set_margin_top(6)
        box.set_margin_bottom(6)
        box.set_margin_start(6)
        box.set_margin_end(6)

        about = Gtk.Button(label="About StayProductive")
        about.add_css_class("flat")
        about.set_halign(Gtk.Align.FILL)
        about.connect("clicked", lambda _button: self._show_about())
        box.append(about)

        data = Gtk.Button(label="Copy data path")
        data.add_css_class("flat")
        data.set_halign(Gtk.Align.FILL)
        data.connect("clicked", lambda _button: self._copy_data_path())
        box.append(data)
        menu.set_child(box)

        button = Gtk.MenuButton()
        button.set_icon_name("open-menu-symbolic")
        button.set_popover(menu)
        button.set_tooltip_text("Main menu")
        return button

    def _install_css(self) -> None:
        from pathlib import Path

        css_path = Path(__file__).with_name("style.css")
        if not css_path.is_file():
            return
        provider = Gtk.CssProvider()
        provider.load_from_path(str(css_path))
        display = self.get_display()
        if display is not None:
            Gtk.StyleContext.add_provider_for_display(
                display, provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
            )

    # -------------------------------------------------------------- navigation
    def _on_row_selected(self, _listbox: Gtk.ListBox, row: Gtk.ListBoxRow | None) -> None:
        if row is None:
            return
        key = getattr(row, "key", "dashboard")
        self.stack.set_visible_child_name(key)
        meta = next(item for item in PAGES if item[0] == key)
        self.title_widget.set_title(meta[1])
        self.title_widget.set_subtitle(meta[3])
        self._set_actions(key)

    def _set_actions(self, key: str) -> None:
        w.clear(self.header_actions)
        if key == "dashboard":
            self.header_actions.append(
                w.button(
                    "New profile",
                    "list-add-symbolic",
                    css="flat",
                    tooltip="Create a profile",
                    on_click=lambda _button: self.create_profile(),
                )
            )

    def switch_page(self, key: str) -> None:
        for index, (page_key, *_rest) in enumerate(PAGES):
            if page_key == key:
                self.nav.select_row(self.nav.get_row_at_index(index))
                return

    # -------------------------------------------------------------- operations
    def create_profile(self) -> None:
        if self._edits_blocked():
            return
        self.editor = ProfileEditor(self, None)
        self.editor.present(self)

    def edit_profile(self, profile: dict) -> None:
        if self._edits_blocked():
            return
        self.editor = ProfileEditor(self, profile)
        self.editor.present(self)

    def _edits_blocked(self) -> bool:
        commitment = self.snapshot.get("commitment")
        if commitment and "block_edits" in commitment.get("restrictions", []):
            self.show_error(
                "Rules are locked",
                "An active strict session blocks rule edits. Request an early exit "
                "from the Strict session page and wait for the cool-down.",
            )
            return True
        return False

    def refresh(self) -> None:
        try:
            self.snapshot = self.engine.snapshot()
            self._error_shown = False
        except Exception as error:  # noqa: BLE001
            if not self._error_shown:
                self._error_shown = True
                self.show_error("The state service is unavailable", error_text(error))
            return

        for page in self.page_widgets.values():
            page.refresh(self.snapshot)

        enforcement = self.snapshot.get("enforcement", {})
        if enforcement.get("state") == "active":
            self.enforcement_label.set_label("Enforcement active")
            self.enforcement_dot.remove_css_class("status-dot-offline")
            self.enforcement_dot.add_css_class("status-dot-online")
            self.service_label.set_label("GNOME extension reporting")
        else:
            self.enforcement_label.set_label("Enforcement offline")
            self.enforcement_dot.remove_css_class("status-dot-online")
            self.enforcement_dot.add_css_class("status-dot-offline")
            self.service_label.set_label("State service running")

    def _tick(self) -> bool:
        self.refresh()
        return True

    def toast(self, text: str) -> None:
        self.toast_overlay.add_toast(Adw.Toast.new(text))

    def show_message(self, text: str) -> None:
        self.toast(text)

    def show_error(self, heading: str, body: str | Exception) -> None:
        text = body if isinstance(body, str) else error_text(body)
        w.alert(self, heading, text)

    def _show_about(self) -> None:
        about = Adw.AboutDialog(
            application_name=APP_TITLE,
            application_icon="preferences-system-time-symbolic",
            developer_name="StayProductive contributors",
            version="0.1.0",
            comments=(
                "A local-first focus app for Linux. The rule engine is written in "
                "Rust; this interface is GTK4 and libadwaita. No account, telemetry, "
                "or network connection is required."
            ),
            license_type=Gtk.License.APACHE_2_0,
        )
        about.present(self)

    def _copy_data_path(self) -> None:
        path = self.snapshot.get("state_path")
        if not path:
            return
        clipboard = self.get_display().get_clipboard()
        clipboard.set(path)
        self.toast("Data path copied")
