"""Activity: local records of what the engine decided."""

from __future__ import annotations

from gi.repository import Gtk

from .. import widgets as w
from ..formatting import error_text, local_time, relative, remaining_seconds


class ActivityPage:
    def __init__(self, window) -> None:
        self.window = window
        self.root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=20)
        self.root.set_margin_top(28)
        self.root.set_margin_bottom(40)
        self.root.set_margin_start(28)
        self.root.set_margin_end(28)
        self._build()
        self.widget = w.page_scroller(self.root, max_width=940)

    def _build(self) -> None:
        copy = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        copy.append(w.label("Activity", "page-title"))
        copy.append(
            w.label(
                "Quick blocks and block decisions recorded locally by the engine. "
                "This page never contacts the network.",
                "muted",
            )
        )
        self.root.append(copy)

        quick = w.Card(spacing=12)
        quick.append(w.label("Active quick blocks", "card-title"))
        self.quick_list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        self.quick_list.set_margin_top(2)
        quick.append(self.quick_list)
        self.root.append(quick)

        history = w.Card(spacing=12)
        history.append(w.label("Block decisions", "card-title"))
        history.append(
            w.label(
                "Decisions are written by the core when a tracker reports a target. "
                "With no desktop adapter connected this list stays empty.",
                "muted",
            )
        )
        self.history_list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        history.append(self.history_list)
        self.root.append(history)

    def refresh(self, snapshot: dict) -> None:
        now = snapshot.get("now")
        tz = snapshot.get("settings", {}).get("timezone", "UTC")
        w.clear(self.quick_list)
        blocks = [
            block
            for block in snapshot.get("quick_blocks", [])
            if remaining_seconds(block.get("until"), now) > 0
        ]
        if not blocks:
            self.quick_list.append(w.label("No quick blocks are active.", "muted small"))
        for block in blocks:
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
            row.append(w.status_pill(block.get("kind", "app"), "accent"))
            title = w.label(block.get("target", "?"), "row-title")
            title.set_hexpand(True)
            row.append(title)
            row.append(
                w.label(
                    f"ends {local_time(block.get('until'), tz)} · "
                    f"{relative(block.get('until'), now)}",
                    "muted small",
                )
            )
            row.append(
                w.button(
                    icon="window-close-symbolic",
                    css="flat",
                    tooltip="End quick block",
                    on_click=lambda _button, item=block: self._clear(item),
                )
            )
            self.quick_list.append(row)

        w.clear(self.history_list)
        history = snapshot.get("history", [])
        if not history:
            self.history_list.append(
                w.label("No block decisions recorded yet.", "muted small")
            )
            return
        for entry in history:
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
            title = w.label(entry.get("target", "?"), "row-title")
            title.set_hexpand(True)
            row.append(title)
            row.append(w.label(entry.get("reason", ""), "muted small"))
            row.append(
                w.label(local_time(entry.get("at"), tz, include_date=True), "muted small")
            )
            self.history_list.append(row)

    def _clear(self, block: dict) -> None:
        try:
            self.window.engine.clear_quick_block(
                "site" if block.get("kind") == "site" else "app",
                block.get("target", ""),
            )
            self.window.refresh()
            self.window.toast("Quick block ended")
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not end quick block", error_text(error))
