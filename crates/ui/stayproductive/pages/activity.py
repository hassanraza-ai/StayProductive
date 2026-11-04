"""Activity: local records of what the engine decided."""

from __future__ import annotations

from gi.repository import Gtk

from .. import widgets as w
from ..formatting import duration_text, error_text, local_time, relative, remaining_seconds


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

        today = w.Card(spacing=12)
        today.append(w.label("Today", "card-title"))
        today.append(
            w.label(
                "Foreground time per app with rules, recorded while the desktop "
                "adapter reports focus. Nothing leaves this machine.",
                "muted",
            )
        )
        self.today_list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        self.today_list.set_margin_top(2)
        today.append(self.today_list)
        self.root.append(today)

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

        privacy = w.Card(spacing=12)
        privacy.append(w.label("Your data", "card-title"))
        privacy.append(
            w.label(
                "Usage history never leaves this machine. Export a portable archive "
                "of profiles, settings, and usage before deleting the history; "
                "profiles, schedules, quick blocks, and an active strict session are "
                "kept either way.",
                "muted",
            )
        )
        actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        actions.append(
            w.button(
                "Export data",
                icon="document-save-symbolic",
                tooltip="Write a versioned archive to the data folder",
                on_click=self._export,
            )
        )
        actions.append(
            w.button(
                "Delete history",
                icon="user-trash-symbolic",
                css="destructive-action",
                tooltip="Remove recorded usage and block decisions",
                on_click=self._confirm_delete,
            )
        )
        privacy.append(actions)
        self.export_note = w.label("No export yet this session.", "muted small")
        self.export_note.set_selectable(True)
        privacy.append(self.export_note)
        self.root.append(privacy)

    def refresh(self, snapshot: dict) -> None:
        now = snapshot.get("now")
        tz = snapshot.get("settings", {}).get("timezone", "UTC")
        w.clear(self.today_list)
        usage = snapshot.get("usage_today") or []
        if not usage:
            self.today_list.append(
                w.label("No tracked usage yet today.", "muted small")
            )
        for entry in usage:
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
            title = w.label(entry.get("app", "?"), "row-title")
            title.set_hexpand(True)
            row.append(title)
            row.append(w.label(duration_text(entry.get("seconds", 0)), "muted small"))
            self.today_list.append(row)

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

    def _export(self, _button) -> None:
        try:
            result = self.window.engine.export_data()
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not export your data", error_text(error))
            return
        path = result.get("path", "")
        files = result.get("files", [])
        self.export_note.set_label(f"Last export: {path}")
        self.window.toast(f"Exported {len(files)} files")
        w.alert(
            self.window,
            "Export ready",
            f"Saved to:\n{path}\n\nThe archive is a zip of JSON files with a "
            "versioned manifest. It contains profiles and settings, and usage "
            "history. It stays on this machine.",
        )

    def _confirm_delete(self, _button) -> None:
        w.confirm(
            self.window,
            "Delete usage history?",
            "This removes recorded foreground intervals, launches, pickups, and "
            "block decisions. Profiles, schedules, quick blocks, and an active "
            "strict session are kept. There is no undo, so export first if you "
            "want a copy.",
            "Delete history",
            self._delete_history,
            destructive=True,
        )

    def _delete_history(self) -> None:
        try:
            result = self.window.engine.delete_history()
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not delete history", error_text(error))
            return
        removed = result.get("removed", {})
        count = sum(
            int(removed.get(key, 0) or 0)
            for key in ("intervals", "launches", "pickups", "blocks")
        )
        self.window.refresh()
        self.window.toast(f"Deleted {count} usage records")

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
