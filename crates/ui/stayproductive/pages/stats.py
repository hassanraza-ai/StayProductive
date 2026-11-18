"""Stats: local screen time, top distractions, and block history."""

from __future__ import annotations

from gi.repository import Gtk

from .. import widgets as w
from ..formatting import (
    app_name,
    duration_text,
    error_text,
    local_time,
    relative,
    remaining_seconds,
)

RANGES = [
    ("today", "Today"),
    ("week", "Week"),
    ("month", "Month"),
]


class StatsPage:
    def __init__(self, window) -> None:
        self.window = window
        self.range_key = "today"
        self.root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=20)
        self.root.set_margin_top(28)
        self.root.set_margin_bottom(40)
        self.root.set_margin_start(28)
        self.root.set_margin_end(28)
        self._build()
        self.widget = w.page_scroller(self.root, max_width=940)

    def _build(self) -> None:
        header = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        copy = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        copy.set_hexpand(True)
        copy.append(w.label("Stats", "page-title"))
        copy.append(
            w.label(
                "Screen time and block decisions recorded locally by this "
                "machine. Nothing here is uploaded.",
                "muted",
            )
        )
        header.append(copy)
        self.range_selector = Gtk.DropDown.new_from_strings(
            [label for _key, label in RANGES]
        )
        self.range_selector.set_selected(0)
        self.range_selector.set_tooltip_text("Statistics range")
        self.range_selector.set_valign(Gtk.Align.CENTER)
        self.range_selector.connect("notify::selected", self._range_changed)
        header.append(self.range_selector)
        self.root.append(header)

        metrics = Gtk.Grid()
        metrics.set_column_spacing(14)
        metrics.set_row_spacing(14)
        metrics.set_column_homogeneous(True)
        self.time_metric = self._metric(metrics, 0, "Screen time", "0 min")
        self.block_metric = self._metric(metrics, 1, "Blocks", "0")
        self.pickup_metric = self._metric(metrics, 2, "Pickups", "0")
        self.launch_metric = self._metric(metrics, 3, "Launches", "0")
        self.root.append(metrics)

        top = w.Card(spacing=12)
        top.append(w.label("Top distractions", "card-title"))
        self.range_note = w.label(
            "Apps with rules, ranked by foreground time in the selected range.",
            "muted",
        )
        top.append(self.range_note)
        self.ranking_list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        self.ranking_list.set_margin_top(2)
        top.append(self.ranking_list)
        self.root.append(top)

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
                "Reason codes and end times are written by the engine when a "
                "tracker reports a blocked target. Window titles are never "
                "recorded here.",
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
                "Usage history stays on this machine. Export a portable archive "
                "of profiles, settings, and usage before deleting the history; "
                "profiles, schedules, quick blocks, and an active strict session "
                "are kept either way.",
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

    def _metric(self, parent: Gtk.Grid, column: int, caption: str, value: str) -> Gtk.Label:
        card = w.Card(spacing=4)
        card.set_hexpand(True)
        card.append(w.label(caption.upper(), "metric-caption"))
        value_label = w.label(value, "metric-value")
        card.append(value_label)
        parent.attach(card, column, 0, 1, 1)
        return value_label

    def _range_changed(self, selector: Gtk.DropDown, _param) -> None:
        index = selector.get_selected()
        if 0 <= index < len(RANGES):
            self.range_key = RANGES[index][0]
            self.refresh(self.window.snapshot)

    def _trend(self, entry: dict) -> str:
        seconds = int(entry.get("seconds", 0) or 0)
        previous = int(entry.get("previous_seconds", 0) or 0)
        if previous <= 0:
            return "new this period" if seconds > 0 else "—"
        if seconds == previous:
            return "no change"
        change = round((seconds - previous) / previous * 100)
        if change == 0:
            return "no change"
        direction = "up" if change > 0 else "down"
        return f"{direction} {abs(change)}% vs previous period"

    def refresh(self, snapshot: dict) -> None:
        now = snapshot.get("now")
        tz = snapshot.get("settings", {}).get("timezone", "UTC")
        stats = (snapshot.get("stats") or {}).get(self.range_key) or {}
        self.time_metric.set_label(duration_text(stats.get("seconds", 0)))
        self.block_metric.set_label(str(stats.get("blocks", 0)))
        self.pickup_metric.set_label(str(stats.get("pickups", 0)))
        self.launch_metric.set_label(str(stats.get("launches", 0)))

        label = next(label for key, label in RANGES if key == self.range_key)
        self.range_note.set_label(
            f"Apps with rules, ranked by foreground time {'today' if self.range_key == 'today' else f'this {label.lower()}'}. "
            "Trend compares the same length of time before it."
        )

        w.clear(self.ranking_list)
        ranking = stats.get("apps") or []
        if not ranking:
            self.ranking_list.append(
                w.label("No tracked screen time in this range.", "muted small")
            )
        for entry in ranking[:8]:
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
            title = w.label(app_name(entry.get("app", "?")), "row-title")
            title.set_hexpand(True)
            row.append(title)
            row.append(w.label(f"{float(entry.get('share', 0)):.0f}%", "muted small"))
            row.append(w.label(duration_text(entry.get("seconds", 0)), "muted small"))
            row.append(w.label(self._trend(entry), "muted small"))
            self.ranking_list.append(row)

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
            title = w.label(app_name(block.get("target", "?")), "row-title")
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
            title = w.label(app_name(entry.get("target", "?")), "row-title")
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
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not end the quick block", error_text(error))
