"""Searchable application chooser used by the profile editor."""

from __future__ import annotations

from pathlib import Path

from gi.repository import Adw, Gtk

from . import widgets as w
from .apps import DesktopApp


def _icon(app: DesktopApp) -> Gtk.Image:
    icon = app.icon.strip()
    if icon and Path(icon).is_absolute() and Path(icon).is_file():
        image = Gtk.Image.new_from_file(icon)
    else:
        image = Gtk.Image.new_from_icon_name(icon or "application-x-executable-symbolic")
    image.set_pixel_size(24)
    return image


class AppChooser(Adw.Dialog):
    """Pick an installed application by name instead of by rule key."""

    def __init__(self, apps: list[DesktopApp], on_pick) -> None:
        super().__init__()
        self._on_pick = on_pick
        self._rows: list[tuple[Adw.ActionRow, DesktopApp]] = []
        self._build(apps)

    def _build(self, apps: list[DesktopApp]) -> None:
        self.set_content_width(560)
        self.set_content_height(620)
        self.set_title("Choose an application")

        toolbar = Adw.ToolbarView()
        header = Adw.HeaderBar()
        header.set_show_end_title_buttons(False)
        header.set_title_widget(
            Adw.WindowTitle(
                title="Choose an application",
                subtitle="Installed desktop applications",
            )
        )
        header.pack_start(
            w.button("Cancel", css="flat", on_click=lambda _button: self.close())
        )
        toolbar.add_top_bar(header)

        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        content.set_margin_top(12)
        content.set_margin_bottom(12)
        content.set_margin_start(18)
        content.set_margin_end(18)

        self.search = Gtk.SearchEntry()
        self.search.set_placeholder_text("Search applications")
        self.search.connect("search-changed", self._filter)
        self.search.connect("activate", lambda _entry: self._activate_first())
        content.append(self.search)

        self.list_box = Gtk.ListBox()
        self.list_box.set_selection_mode(Gtk.SelectionMode.NONE)
        self.list_box.add_css_class("boxed-list")
        self.list_box.set_valign(Gtk.Align.START)

        for app in apps:
            row = Adw.ActionRow(activatable=True)
            row.set_use_markup(False)
            row.set_title(app.name)
            row.set_subtitle(app.comment or "Installed application")
            row.set_tooltip_text(app.key)
            row.add_prefix(_icon(app))
            row.connect("activated", lambda _row, item=app: self._pick(item))
            self.list_box.append(row)
            self._rows.append((row, app))

        self.empty = Adw.StatusPage(
            title="No matching applications",
            description="Try a different search, or enter an application ID in the editor.",
            icon_name="system-search-symbolic",
        )
        self.empty.set_visible(False)
        self.empty.set_vexpand(True)

        self.scroller = Gtk.ScrolledWindow()
        self.scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.scroller.set_vexpand(True)
        self.scroller.set_child(self.list_box)
        content.append(self.scroller)
        content.append(self.empty)

        toolbar.set_content(content)
        self.set_child(toolbar)
        self.connect("map", lambda _dialog: self.search.grab_focus())
        self._filter()

    def _filter(self, *_args) -> None:
        query = self.search.get_text().strip().casefold()
        visible = 0
        for row, app in self._rows:
            matches = (
                not query
                or query in app.name.casefold()
                or query in app.key.casefold()
                or query in app.desktop_id.casefold()
            )
            row.set_visible(matches)
            visible += int(matches)
        self.empty.set_visible(visible == 0)
        self.scroller.set_visible(visible != 0)

    def _activate_first(self) -> None:
        for row, app in self._rows:
            if row.get_visible():
                self._pick(app)
                return

    def _pick(self, app: DesktopApp) -> None:
        self._on_pick(app)
        self.close()
