"""Profiles: the rules that define focus."""

from __future__ import annotations

from gi.repository import Gtk

from .. import widgets as w
from ..formatting import (
    ACTIVATION_LABELS,
    error_text,
    profile_subtitle,
    rule_description,
    schedule_summary,
)

TEMPLATES = [
    (
        "Work hours",
        "Weekdays, 09:00–17:00",
        {
            "name": "Work hours",
            "activation": "scheduled",
            "schedule": {
                "kind": "days",
                "days": ["Mon", "Tue", "Wed", "Thu", "Fri"],
                "windows": [{"start": "09:00", "end": "17:00"}],
            },
            "sites": [
                {"pattern": "youtube.com", "action": "block"},
                {"pattern": "x.com", "action": "block"},
                {"pattern": "reddit.com", "action": "block"},
                {"pattern": "instagram.com", "action": "block"},
            ],
        },
    ),
    (
        "Evening wind-down",
        "Every day, 22:00–23:59",
        {
            "name": "Evening wind-down",
            "activation": "scheduled",
            "schedule": {
                "kind": "daily",
                "windows": [{"start": "22:00", "end": "23:59"}],
            },
            "sites": [
                {"pattern": "tiktok.com", "action": "block"},
                {"pattern": "youtube.com", "action": "block"},
            ],
            "keywords": ["shorts"],
        },
    ),
    (
        "Deep work",
        "Manual session",
        {
            "name": "Deep work",
            "activation": "manual",
            "sites": [
                {"pattern": "youtube.com", "action": "block"},
                {"pattern": "x.com", "action": "block"},
                {"pattern": "reddit.com", "action": "block"},
            ],
            "keywords": ["shorts", "reels"],
        },
    ),
]


class ProfilesPage:
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
        copy.append(w.label("Profiles", "page-title"))
        copy.append(
            w.label(
                "A profile is a named bundle of rules plus an optional schedule. "
                "Decisions use the most specific matching rule.",
                "muted",
            )
        )
        copy.set_hexpand(True)
        intro.append(copy)
        self.new_button = w.button(
            "New profile",
            "list-add-symbolic",
            css="suggested-action",
            on_click=lambda _button: self.window.create_profile(),
        )
        self.new_button.set_valign(Gtk.Align.START)
        intro.append(self.new_button)
        self.root.append(intro)

        self.lock_notice = w.Card(warning=True, spacing=6)
        lock_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        lock_row.append(w.icon_badge("channel-secure-symbolic", 18))
        lock_row.append(
            w.label(
                "A strict session is active. Profile edits and pauses are locked. "
                "Open Strict session to request an early exit.",
                "muted small",
            )
        )
        self.lock_notice.append(lock_row)
        self.lock_notice.set_visible(False)
        self.root.append(self.lock_notice)

        self.list_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        self.root.append(self.list_box)

        self.empty = w.Card(spacing=16)
        empty_head = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=14)
        empty_head.append(w.icon_badge("view-list-symbolic", 20))
        empty_copy = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        empty_copy.append(w.label("No profiles yet", "card-title"))
        empty_copy.append(
            w.label(
                "Start from a template or build your own. Templates are ordinary "
                "profiles you can edit afterwards.",
                "muted",
            )
        )
        empty_head.append(empty_copy)
        self.empty.append(empty_head)
        templates = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        templates.set_margin_top(6)
        for title, subtitle, spec in TEMPLATES:
            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
            row.add_css_class("template-row")
            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=1)
            text.append(w.label(title, "row-title"))
            text.append(w.label(subtitle, "muted small"))
            text.set_hexpand(True)
            row.append(text)
            row.append(
                w.button(
                    "Use",
                    css="flat",
                    on_click=lambda _button, profile=spec: self._use_template(profile),
                )
            )
            templates.append(row)
        self.empty.append(templates)
        self.root.append(self.empty)

        self.hint = w.label(
            "Profiles are stored in a versioned local file. No account, cloud, or "
            "network connection is involved.",
            "muted small",
        )
        self.root.append(self.hint)

    def refresh(self, snapshot: dict) -> None:
        w.clear(self.list_box)
        profiles = snapshot.get("profiles", [])
        commitment = snapshot.get("commitment") or {}
        restrictions = commitment.get("restrictions", [])
        edits_locked = "block_edits" in restrictions
        pause_locked = "block_pause" in restrictions
        self.new_button.set_sensitive(not edits_locked)
        self.new_button.set_tooltip_text(
            "Profile edits are locked by the active strict session."
            if edits_locked
            else "Create a profile."
        )
        self.lock_notice.set_visible(edits_locked or pause_locked)
        self.empty.set_visible(not profiles)
        self.hint.set_visible(bool(profiles))
        manual_active = snapshot.get("manual_active")
        now = snapshot.get("now")
        for profile in profiles:
            self.list_box.append(
                self._profile_card(
                    profile,
                    manual_active,
                    now,
                    edits_locked=edits_locked,
                    pause_locked=pause_locked,
                )
            )

    def _profile_card(
        self,
        profile: dict,
        manual_active: str | None,
        now: str,
        *,
        edits_locked: bool = False,
        pause_locked: bool = False,
    ) -> Gtk.Widget:
        card = w.Card(spacing=12)
        top = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=14)
        badge = w.icon_badge("view-list-symbolic", 18)
        top.append(badge)

        text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3)
        name_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        name_row.append(w.label(profile.get("name", "Untitled"), "card-title"))
        if profile.get("active"):
            name_row.append(w.status_pill("Active", "active"))
        if profile.get("paused"):
            name_row.append(w.status_pill("Paused", "warning"))
        if not profile.get("enabled"):
            name_row.append(w.status_pill("Disabled", "neutral"))
        text.append(name_row)
        text.append(w.label(profile_subtitle(profile), "muted"))
        text.set_hexpand(True)
        top.append(text)

        if profile.get("activation") == "manual":
            toggle = Gtk.Switch()
            toggle.set_valign(Gtk.Align.CENTER)
            toggle.set_active(manual_active == profile.get("id"))
            toggle.set_sensitive(not edits_locked)
            toggle.set_tooltip_text(
                "Activation is locked by the active strict session."
                if edits_locked
                else "Activate this profile"
            )
            toggle.connect(
                "notify::active",
                lambda switch, _param, item=profile: self._toggle(item, switch.get_active()),
            )
            top.append(toggle)
        else:
            label = ACTIVATION_LABELS.get(profile.get("activation"), "Automatic")
            top.append(w.status_pill(label, "accent"))
        top.append(
            w.menu_button(
                "Profile actions",
                [
                    (
                        "Edit",
                        lambda item=profile: self.window.edit_profile(item),
                        not edits_locked,
                    ),
                    (
                        "Resume" if profile.get("paused") else "Pause for 30 min",
                        lambda item=profile: self._pause(item),
                        not pause_locked or bool(profile.get("paused")),
                    ),
                    (
                        "Delete",
                        lambda item=profile: self._delete(item),
                        not edits_locked,
                    ),
                ],
            )
        )
        card.append(top)

        rules = []
        for app in profile.get("apps", []):
            rules.append(f"{app.get('key')} — {rule_description(app)}")
        for site in profile.get("sites", []):
            rules.append(f"{site.get('pattern')} — {rule_description(site)}")
        for keyword in profile.get("keywords", []):
            rules.append(f"keyword “{keyword}”")
        if profile.get("pickup_limit"):
            limit = profile["pickup_limit"]
            rules.append(f"pickups — {limit.get('count')} per {limit.get('per')}")
        if rules:
            chips = w.pill_flow(rules[:5], max_per_line=3)
            chips.set_margin_top(2)
            if len(rules) > 5:
                chips.append(w.Pill(f"+{len(rules) - 5} more", "neutral"))
            card.append(chips)
        else:
            card.append(w.label("No rules yet. Edit this profile to add some.", "muted small"))
        return card

    def _use_template(self, spec: dict) -> None:
        try:
            self.window.engine.create_profile(dict(spec))
            self.window.refresh()
            self.window.toast(f"Created “{spec.get('name', 'profile')}”")
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not create profile", error_text(error))

    def _toggle(self, profile: dict, active: bool) -> None:
        try:
            if active:
                self.window.engine.activate_profile(profile["id"])
            else:
                self.window.engine.deactivate_profile()
            self.window.refresh()
            self.window.toast(
                f"Activated “{profile.get('name')}”"
                if active
                else "Manual profile deactivated"
            )
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not change activation", error_text(error))
            self.window.refresh()

    def _pause(self, profile: dict) -> None:
        try:
            if profile.get("paused"):
                self.window.engine.resume_profile(profile["id"])
                self.window.toast("Profile resumed")
            else:
                self.window.engine.pause_profile(profile["id"], 30)
                self.window.toast("Profile paused for 30 minutes")
            self.window.refresh()
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not pause or resume", error_text(error))

    def _delete(self, profile: dict) -> None:
        def remove() -> None:
            try:
                self.window.engine.delete_profile(profile["id"])
                self.window.refresh()
                self.window.toast(f"Deleted “{profile.get('name')}”")
            except Exception as error:  # noqa: BLE001
                self.window.show_error("Could not delete profile", error_text(error))

        w.confirm(
            self.window,
            f"Delete “{profile.get('name')}”?",
            "This removes the profile and its rules. Existing history is kept.",
            "Delete",
            remove,
            destructive=True,
        )
