"""Shared GTK building blocks."""

from __future__ import annotations

from collections.abc import Callable

from gi.repository import Adw, Gtk


def clear(widget: Gtk.Widget) -> None:
    child = widget.get_first_child()
    while child is not None:
        next_child = child.get_next_sibling()
        if isinstance(child, Gtk.Window):
            child.destroy()
        else:
            widget.remove(child)
        child = next_child


class Card(Gtk.Box):
    def __init__(self, *, hero: bool = False, warning: bool = False, spacing: int = 14):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=spacing)
        self.add_css_class("card")
        if hero:
            self.add_css_class("hero-card")
        if warning:
            self.add_css_class("warning-card")


class Pill(Gtk.Label):
    def __init__(self, text: str, kind: str = "neutral"):
        super().__init__(label=text)
        self.add_css_class("pill")
        self.add_css_class(f"pill-{kind}")
        self.set_valign(Gtk.Align.CENTER)


class Badge(Gtk.Box):
    def __init__(self, icon_name: str, size: int = 20):
        super().__init__()
        self.add_css_class("icon-badge")
        self.set_halign(Gtk.Align.CENTER)
        self.set_valign(Gtk.Align.CENTER)
        image = Gtk.Image.new_from_icon_name(icon_name)
        image.set_pixel_size(size)
        self.append(image)


def icon_badge(icon_name: str, size: int = 20) -> Gtk.Widget:
    return Badge(icon_name, size)


def status_pill(text: str, kind: str = "neutral") -> Gtk.Label:
    return Pill(text.upper(), kind)


def pill_flow(
    items: list[str], kind: str = "neutral", max_per_line: int = 4
) -> Gtk.FlowBox:
    """Pills that wrap instead of forcing the page wider than the window."""
    flow = Gtk.FlowBox()
    flow.set_selection_mode(Gtk.SelectionMode.NONE)
    flow.set_homogeneous(False)
    flow.set_max_children_per_line(max_per_line)
    flow.set_row_spacing(6)
    flow.set_column_spacing(6)
    for item in items:
        flow.append(Pill(item, kind))
    return flow


def label(text: str, css: str | None = None, xalign: float = 0.0) -> Gtk.Label:
    widget = Gtk.Label(label=text, xalign=xalign)
    widget.set_wrap(True)
    if css:
        widget.add_css_class(css)
    return widget


def button(
    text: str | None = None,
    icon: str | None = None,
    *,
    css: str | None = None,
    tooltip: str | None = None,
    on_click: Callable[[Gtk.Button], None] | None = None,
) -> Gtk.Button:
    widget = Gtk.Button()
    if text is not None and icon is not None:
        content = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        content.append(Gtk.Image.new_from_icon_name(icon))
        content.append(Gtk.Label(label=text))
        widget.set_child(content)
    elif icon is not None:
        widget.set_icon_name(icon)
    else:
        widget.set_label(text or "")
    if css:
        widget.add_css_class(css)
    if tooltip:
        widget.set_tooltip_text(tooltip)
    if on_click:
        widget.connect("clicked", on_click)
    return widget


def page_scroller(content: Gtk.Widget, max_width: int = 1000) -> Gtk.ScrolledWindow:
    clamp = Adw.Clamp()
    clamp.set_maximum_size(max_width)
    clamp.set_tightening_threshold(700)
    clamp.set_child(content)
    scroller = Gtk.ScrolledWindow()
    scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
    scroller.set_child(clamp)
    scroller.set_vexpand(True)
    return scroller


def nav_row(key: str, title: str, icon: str) -> Gtk.ListBoxRow:
    row = Gtk.ListBoxRow()
    row.add_css_class("nav-row")
    row.key = key  # type: ignore[attr-defined]
    box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
    box.set_margin_top(8)
    box.set_margin_bottom(8)
    box.set_margin_start(10)
    box.set_margin_end(10)
    image = Gtk.Image.new_from_icon_name(icon)
    image.set_pixel_size(18)
    box.append(image)
    text = Gtk.Label(label=title, xalign=0)
    text.set_hexpand(True)
    box.append(text)
    row.set_child(box)
    return row


def alert(parent: Gtk.Widget, heading: str, body: str) -> None:
    dialog = Adw.AlertDialog(heading=heading, body=body)
    dialog.add_response("ok", "OK")
    dialog.present(parent)


def confirm(
    parent: Gtk.Widget,
    heading: str,
    body: str,
    confirm_label: str,
    on_confirm: Callable[[], None],
    *,
    destructive: bool = False,
) -> None:
    dialog = Adw.AlertDialog(heading=heading, body=body)
    dialog.add_response("cancel", "Cancel")
    dialog.add_response("confirm", confirm_label)
    dialog.set_response_appearance(
        "confirm",
        Adw.ResponseAppearance.DESTRUCTIVE
        if destructive
        else Adw.ResponseAppearance.SUGGESTED,
    )
    dialog.set_default_response("cancel")
    dialog.set_close_response("cancel")

    def respond(_dialog: Adw.AlertDialog, response: str) -> None:
        if response == "confirm":
            on_confirm()

    dialog.connect("response", respond)
    dialog.present(parent)


def menu_button(
    name: str,
    items: list[tuple[str, Callable[[], None], bool]],
) -> Gtk.MenuButton:
    """A compact popover menu. `items` are (label, callback, enabled) triples."""
    menu = Gtk.Popover()
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
    box.set_margin_top(6)
    box.set_margin_bottom(6)
    box.set_margin_start(6)
    box.set_margin_end(6)
    for text, callback, enabled in items:
        item = Gtk.Button(label=text)
        item.set_halign(Gtk.Align.FILL)
        item.add_css_class("flat")
        item.set_sensitive(enabled)
        item.connect("clicked", lambda _button, fn=callback: fn())
        box.append(item)
    menu.set_child(box)
    widget = Gtk.MenuButton()
    widget.set_icon_name("view-more-symbolic")
    widget.add_css_class("flat")
    widget.set_popover(menu)
    widget.set_tooltip_text(name)
    return widget


def row_text(title: str, subtitle: str | None = None) -> tuple[Gtk.Box, Gtk.Label]:
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
    title_label = Gtk.Label(label=title, xalign=0)
    title_label.set_ellipsize(3)
    title_label.add_css_class("row-title")
    box.append(title_label)
    if subtitle:
        sub = Gtk.Label(label=subtitle, xalign=0)
        sub.set_wrap(True)
        sub.add_css_class("muted")
        sub.add_css_class("small")
        box.append(sub)
    return box, title_label
