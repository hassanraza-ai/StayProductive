"""Dashboard: one glance at what the engine is doing right now."""

from __future__ import annotations

from gi.repository import Gtk

from ..formatting import (
    commitment_remaining,
    error_text,
    local_time,
    parse_time,
    relative,
    remaining_seconds,
    rule_count,
)
from .. import widgets as w


class DashboardPage:
    def __init__(self, window) -> None:
        self.window = window
        self.root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=24)
        self.root.set_margin_top(28)
        self.root.set_margin_bottom(40)
        self.root.set_margin_start(28)
        self.root.set_margin_end(28)
        self._build()
        self.widget = w.page_scroller(self.root)

    def _build(self) -> None:
        # ------------------------------------------------------------ hero
        self.hero = w.Card(hero=True, spacing=18)
        top = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        self.hero_badge = w.icon_badge("appointment-soon-symbolic", 22)
        top.append(self.hero_badge)
        self.state_pill = w.status_pill("Idle")
        top.append(self.state_pill)
        spacer = Gtk.Box()
        spacer.set_hexpand(True)
        top.append(spacer)
        self.boundary_pill = w.Pill("No change scheduled")
        top.append(self.boundary_pill)
        self.hero.append(top)

        self.hero_title = w.label("Ready when you are", "hero-title")
        self.hero.append(self.hero_title)
        self.hero_subtitle = w.label("", "muted")
        self.hero.append(self.hero_subtitle)

        actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        actions.set_margin_top(4)
        self.primary_action = w.button(
            "Choose a profile",
            "go-next-symbolic",
            css="suggested-action",
            on_click=lambda _button: self.window.switch_page("profiles"),
        )
        actions.append(self.primary_action)
        self.pause_action = w.button(
            "Pause for 15 min",
            "media-playback-pause-symbolic",
            on_click=self._pause_active,
        )
        actions.append(self.pause_action)
        actions.append(
            w.button(
                "Strict session",
                "channel-secure-symbolic",
                on_click=lambda _button: self.window.switch_page("focus"),
            )
        )
        self.hero.append(actions)
        self.root.append(self.hero)

        # --------------------------------------------------------- metrics
        metrics = Gtk.Grid()
        metrics.set_column_spacing(14)
        metrics.set_row_spacing(14)
        metrics.set_column_homogeneous(True)
        self.profile_metric = self._metric(metrics, 0, 0, "Profiles", "0", "None yet")
        self.rule_metric = self._metric(metrics, 1, 0, "Rules", "0", "Across all profiles")
        self.next_metric = self._metric(metrics, 0, 1, "Next change", "—", "Nothing scheduled")
        self.commit_metric = self._metric(metrics, 1, 1, "Commitment", "None", "No strict session")
        self.root.append(metrics)

        # --------------------------------------------------- enforcement
        self.enforcement_warning = w.Card(warning=True)
        warning_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=14)
        warning_row.append(w.icon_badge("dialog-warning-symbolic", 20))
        warning_copy = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        warning_copy.append(w.label("Desktop enforcement is offline", "card-title"))
        warning_copy.append(
            w.label(
                "The rule engine is making real decisions, but no desktop adapter "
                "is reporting. Install and enable the GNOME extension in Settings. "
                "Until then, apps are not blocked and this card will stay visible.",
                "muted",
            )
        )
        warning_copy.set_hexpand(True)
        warning_row.append(warning_copy)
        self.enforcement_warning.append(warning_row)
        self.root.append(self.enforcement_warning)

        self.enforcement_active = w.Card()
        active_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=14)
        active_row.append(w.icon_badge("emblem-ok-symbolic", 20))
        active_copy = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        active_copy.append(w.label("Desktop enforcement is active", "card-title"))
        self.enforcement_detail = w.label("", "muted")
        active_copy.append(self.enforcement_detail)
        active_copy.set_hexpand(True)
        active_row.append(active_copy)
        self.enforcement_active.append(active_row)
        self.root.append(self.enforcement_active)

        # ------------------------------------------------------ quick block
        quick = w.Card()
        header = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        header.append(w.label("Block something now", "card-title"))
        header.set_hexpand(True)
        quick.append(header)
        quick.append(
            w.label(
                "Creates a real time-boxed quick block in the engine. It is stored locally "
                "and can be ended from this card or Activity.",
                "muted",
            )
        )
        form = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        form.set_margin_top(4)
        self.quick_target = Gtk.Entry()
        self.quick_target.set_placeholder_text("Application id or domain")
        self.quick_target.set_hexpand(True)
        form.append(self.quick_target)
        self.quick_kind = Gtk.DropDown.new_from_strings(["App", "Site"])
        form.append(self.quick_kind)
        self.quick_minutes = Gtk.SpinButton.new_with_range(5, 480, 5)
        self.quick_minutes.set_value(15)
        self.quick_minutes.set_tooltip_text("Minutes")
        form.append(self.quick_minutes)
        form.append(
            w.button("Block", "list-add-symbolic", css="suggested-action", on_click=self._add_quick)
        )
        quick.append(form)
        self.quick_list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        self.quick_list.set_margin_top(8)
        quick.append(self.quick_list)
        self.root.append(quick)

        # ---------------------------------------------------------- history
        history = w.Card()
        history.append(w.label("Recent block decisions", "card-title"))
        history.append(
            w.label(
                "Recorded by the engine. This list stays on this device.",
                "muted",
            )
        )
        self.history_list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        self.history_list.set_margin_top(4)
        history.append(self.history_list)
        self.root.append(history)

    def _metric(
        self,
        parent: Gtk.Grid,
        column: int,
        row: int,
        caption: str,
        value: str,
        sub: str,
    ) -> tuple:
        card = w.Card(spacing=2)
        value_label = w.label(value, "metric-value")
        caption_label = w.label(caption, "metric-caption")
        sub_label = w.label(sub, "muted")
        sub_label.add_css_class("small")
        card.append(value_label)
        card.append(caption_label)
        card.append(sub_label)
        parent.attach(card, column, row, 1, 1)
        return value_label, sub_label

    def refresh(self, snapshot: dict) -> None:
        now = snapshot.get("now")
        tz = snapshot.get("settings", {}).get("timezone", "UTC")
        profiles = snapshot.get("profiles", [])
        active = snapshot.get("active_profiles", [])
        commitment = snapshot.get("commitment")
        next_boundary = snapshot.get("next_boundary")

        if active:
            names = ", ".join(profile["name"] for profile in active)
            self.hero_title.set_label(names)
            ends = active[0].get("ends_at")
            if ends:
                self.hero_subtitle.set_label(
                    f"Active now · next change {relative(ends, now)} "
                    f"({local_time(ends, tz)} {tz})"
                )
            else:
                self.hero_subtitle.set_label(
                    "Active now · this profile has no automatic end."
                )
            self.state_pill.set_label("PROFILE ACTIVE")
            self.state_pill.remove_css_class("pill-idle")
            self.state_pill.add_css_class("pill-active")
            self.primary_action.set_label("Manage profiles")
        else:
            self.hero_title.set_label("Ready when you are")
            self.hero_subtitle.set_label(
                "Create a profile to describe what focus looks like. Everything stays "
                "on this device and no account is needed."
            )
            self.state_pill.set_label("IDLE")
            self.state_pill.remove_css_class("pill-active")
            self.state_pill.add_css_class("pill-idle")
            self.primary_action.set_label("Choose a profile")

        if next_boundary:
            self.boundary_pill.set_label(
                f"Next change {relative(next_boundary, now)}"
            )
        elif commitment and commitment.get("ends_at"):
            self.boundary_pill.set_label(
                f"Strict ends {relative(commitment['ends_at'], now)}"
            )
        else:
            self.boundary_pill.set_label("No change scheduled")

        pause_blocked = bool(
            commitment
            and "block_pause" in commitment.get("restrictions", [])
        )
        self.pause_action.set_sensitive(bool(active) and not pause_blocked)
        self.pause_action.set_tooltip_text(
            "Pausing is blocked by the active strict session."
            if pause_blocked
            else "Pause the active profile for 15 minutes."
        )

        enforcement = snapshot.get("enforcement", {})
        enforcement_active = enforcement.get("state") == "active"
        self.enforcement_warning.set_visible(not enforcement_active)
        self.enforcement_active.set_visible(enforcement_active)
        if enforcement_active:
            self.enforcement_detail.set_label(
                enforcement.get(
                    "message",
                    "The GNOME Shell extension is applying app blocks now.",
                )
            )

        self.profile_metric[0].set_label(str(len(profiles)))
        enabled = sum(1 for profile in profiles if profile.get("enabled"))
        self.profile_metric[1].set_label(
            f"{enabled} enabled" if profiles else "Create your first profile"
        )
        total_rules = sum(rule_count(profile) for profile in profiles)
        self.rule_metric[0].set_label(str(total_rules))
        self.rule_metric[1].set_label("Apps, sites, keywords")

        if next_boundary:
            self.next_metric[0].set_label(local_time(next_boundary, tz))
            self.next_metric[1].set_label(f"{relative(next_boundary, now)} · {tz}")
        else:
            self.next_metric[0].set_label("—")
            self.next_metric[1].set_label("Nothing scheduled")

        if commitment:
            remaining = commitment_remaining(commitment, now)
            state = commitment.get("state", "active")
            if state == "cooling_down":
                self.commit_metric[0].set_label("Cooling down")
            else:
                self.commit_metric[0].set_label("Strict")
            self.commit_metric[1].set_label(f"{remaining} remaining")
        else:
            self.commit_metric[0].set_label("None")
            self.commit_metric[1].set_label("No strict session")

        self._refresh_quick(snapshot)
        self._refresh_history(snapshot)

    def _refresh_quick(self, snapshot: dict) -> None:
        w.clear(self.quick_list)
        now = snapshot.get("now")
        blocks = [
            block
            for block in snapshot.get("quick_blocks", [])
            if remaining_seconds(block.get("until"), now) > 0
        ]
        if not blocks:
            empty = w.label("No quick blocks are active.", "muted")
            empty.add_css_class("small")
            self.quick_list.append(empty)
            return
        for block in blocks:
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
            badge = w.status_pill(block.get("kind", "app"), "accent")
            row.append(badge)
            title = w.label(block.get("target", "?"), "row-title")
            title.set_hexpand(True)
            row.append(title)
            row.append(
                w.label(f"ends {relative(block.get('until'), now)}", "muted small")
            )
            remove = w.button(
                icon="window-close-symbolic",
                css="flat",
                tooltip="End quick block",
                on_click=lambda _button, item=block: self._clear_quick(item),
            )
            row.append(remove)
            self.quick_list.append(row)

    def _refresh_history(self, snapshot: dict) -> None:
        w.clear(self.history_list)
        history = snapshot.get("history", [])
        if not history:
            empty = w.label(
                "No blocks recorded yet. Desktop blocks appear here once the "
                "GNOME extension is active.",
                "muted",
            )
            empty.add_css_class("small")
            self.history_list.append(empty)
            return
        for entry in history[:8]:
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
            title = w.label(entry.get("target", "?"), "row-title")
            title.set_hexpand(True)
            row.append(title)
            row.append(w.label(entry.get("reason", ""), "muted small"))
            self.history_list.append(row)

    def _pause_active(self) -> None:
        active = self.window.snapshot.get("active_profiles", [])
        if not active:
            return
        try:
            self.window.engine.pause_profile(active[0]["id"], 15)
            self.window.refresh()
            self.window.toast("Paused for 15 minutes")
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not pause", error_text(error))

    def _add_quick(self) -> None:
        target = self.quick_target.get_text().strip()
        if not target:
            self.window.show_message("Enter an app id or a domain first.")
            return
        kind = "app" if self.quick_kind.get_selected() == 0 else "site"
        minutes = int(self.quick_minutes.get_value())
        try:
            self.window.engine.quick_block(kind, target, minutes)
            self.quick_target.set_text("")
            self.window.refresh()
            self.window.toast(f"Quick block saved for {minutes} minutes")
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not create quick block", error_text(error))

    def _clear_quick(self, block: dict) -> None:
        try:
            self.window.engine.clear_quick_block(
                "site" if block.get("kind") == "site" else "app",
                block.get("target", ""),
            )
            self.window.refresh()
            self.window.toast("Quick block ended")
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not end quick block", error_text(error))
