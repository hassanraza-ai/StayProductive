"""Schedules: when each profile takes effect."""

from __future__ import annotations

from gi.repository import Gtk

from .. import widgets as w
from ..formatting import local_time, relative, schedule_summary


class SchedulesPage:
    def __init__(self, window) -> None:
        self.window = window
        self.root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=20)
        self.root.set_margin_top(28)
        self.root.set_margin_bottom(40)
        self.root.set_margin_start(28)
        self.root.set_margin_end(28)
        self._build()
        self.widget = w.page_scroller(self.root)

    def _build(self) -> None:
        intro = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=16)
        copy = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        copy.append(w.label("Schedules", "page-title"))
        copy.append(
            w.label(
                "Schedules use local wall-clock time. Crossing midnight, DST, and the "
                "start-of-day setting are handled by the core scheduler.",
                "muted",
            )
        )
        copy.set_hexpand(True)
        intro.append(copy)
        intro.append(
            w.button(
                "New profile",
                "list-add-symbolic",
                css="suggested-action",
                on_click=lambda _button: self.window.create_profile(),
            )
        )
        self.root.append(intro)
        self.list_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        self.root.append(self.list_box)
        self.empty = w.Card(spacing=14)
        head = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=14)
        head.append(w.icon_badge("appointment-new-symbolic", 20))
        text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        text.append(w.label("No scheduled profiles", "card-title"))
        text.append(
            w.label(
                "Edit a profile and choose “Scheduled” to give it a repeating window.",
                "muted",
            )
        )
        head.append(text)
        self.empty.append(head)
        self.root.append(self.empty)

    def refresh(self, snapshot: dict) -> None:
        w.clear(self.list_box)
        now = snapshot.get("now")
        tz = snapshot.get("settings", {}).get("timezone", "UTC")
        scheduled = [
            profile
            for profile in snapshot.get("profiles", [])
            if profile.get("activation") == "scheduled" or profile.get("schedule")
        ]
        self.empty.set_visible(not scheduled)
        for profile in scheduled:
            self.list_box.append(self._card(profile, now, tz))

    def _card(self, profile: dict, now: str, tz: str) -> Gtk.Widget:
        card = w.Card(spacing=10)
        top = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        top.append(w.icon_badge("appointment-new-symbolic", 18))
        text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        text.append(w.label(profile.get("name", "Untitled"), "card-title"))
        text.append(w.label(schedule_summary(profile.get("schedule")), "muted"))
        text.set_hexpand(True)
        top.append(text)

        status = profile.get("status", {})
        if status.get("active"):
            top.append(w.status_pill("Active now", "active"))
        elif profile.get("activation") != "scheduled":
            top.append(w.status_pill("Manual", "neutral"))
        else:
            top.append(w.status_pill("Waiting", "neutral"))
        top.append(
            w.button(
                "Edit",
                "document-edit-symbolic",
                css="flat",
                on_click=lambda _button, item=profile: self.window.edit_profile(item),
            )
        )
        card.append(top)

        detail = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=16)
        if profile.get("activation") != "scheduled":
            detail.append(w.label("This profile is activated manually.", "muted small"))
        elif status.get("active"):
            ends = status.get("ends_at")
            detail.append(
                w.label(
                    f"Active until {local_time(ends, tz)} · {relative(ends, now)}",
                    "muted small",
                )
            )
        else:
            next_start = status.get("next_start")
            detail.append(
                w.label(
                    f"Next start {local_time(next_start, tz, include_date=True)} · "
                    f"{relative(next_start, now)}",
                    "muted small",
                )
            )
        if profile.get("paused"):
            detail.append(w.status_pill("Paused", "warning"))
        card.append(detail)
        return card
