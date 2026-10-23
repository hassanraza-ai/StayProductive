"""Create and edit a profile with the same state model the engine uses."""

from __future__ import annotations

import datetime as dt

from gi.repository import Adw, Gtk

from . import widgets as w
from .app_picker import AppChooser
from .apps import DesktopApp, installed_apps
from .formatting import ACTION_LABELS, WEEKDAYS, error_text, rule_description

ACTIVATIONS = [("manual", "Manual"), ("scheduled", "Scheduled"), ("always", "Always on")]
SCHEDULE_KINDS = [
    ("daily", "Every day"),
    ("days", "Selected days"),
    ("intervals", "Split windows"),
    ("full_day", "Full day"),
    ("hourly", "Hourly cycle"),
    ("one_time", "Date range"),
]
APP_ACTIONS = [
    ("block", "Block"),
    ("daily_limit", "Daily limit"),
    ("hourly_limit", "Hourly limit"),
]
SITE_ACTIONS = [
    ("block", "Block"),
    ("adult", "Block adult"),
    ("daily_limit", "Daily limit"),
]


def _index(options: list[tuple[str, str]], key: str) -> int:
    for index, (value, _label) in enumerate(options):
        if value == key:
            return index
    return 0


class ProfileEditor(Adw.Dialog):
    def __init__(self, window, profile: dict | None = None) -> None:
        super().__init__()
        self.window = window
        self.profile = profile
        self.spec = self._spec_from_profile(profile)
        self._installed_apps = installed_apps()
        self._app_names = {app.key: app.name for app in self._installed_apps}
        self._app_rows: list[Gtk.Widget] = []
        self._site_rows: list[Gtk.Widget] = []
        self._keyword_rows: list[Gtk.Widget] = []
        self._allow_app_rows: list[Gtk.Widget] = []
        self._allow_site_rows: list[Gtk.Widget] = []
        self._window_rows: list[Gtk.Widget] = []
        self._window_entries: list[tuple[Gtk.Entry, Gtk.Entry]] = []
        self._day_buttons: list[Gtk.ToggleButton] = []
        self._build()
        self.refresh_rules()
        self._update_schedule_visibility()

    # ------------------------------------------------------------------ data
    def _spec_from_profile(self, profile: dict | None) -> dict:
        if profile is None:
            return {
                "name": "",
                "enabled": True,
                "priority": 5,
                "activation": "manual",
                "strict_eligible": False,
                "schedule": None,
                "apps": [],
                "sites": [],
                "keywords": [],
                "allow_apps": [],
                "allow_sites": [],
                "pickup_limit": None,
            }
        return {
            "name": profile.get("name", ""),
            "enabled": profile.get("enabled", True),
            "priority": profile.get("priority", 5),
            "activation": profile.get("activation", "manual"),
            "strict_eligible": profile.get("strict_eligible", False),
            "schedule": profile.get("schedule"),
            "apps": [dict(rule) for rule in profile.get("apps", [])],
            "sites": [dict(rule) for rule in profile.get("sites", [])],
            "keywords": list(profile.get("keywords", [])),
            "allow_apps": list(profile.get("allow_apps", [])),
            "allow_sites": list(profile.get("allow_sites", [])),
            "pickup_limit": profile.get("pickup_limit"),
        }

    def _default_schedule(self) -> dict:
        today = dt.date.today()
        return {
            "kind": "daily",
            "days": [],
            "windows": [{"start": "09:00", "end": "17:00"}],
            "on_minutes": 50,
            "off_minutes": 10,
            "start_date": today.isoformat(),
            "end_date": (today + dt.timedelta(days=7)).isoformat(),
        }

    # ------------------------------------------------------------------ build
    def _build(self) -> None:
        self.set_content_width(780)
        self.set_content_height(800)
        self.set_title("Edit profile" if self.profile else "New profile")
        toolbar = Adw.ToolbarView()
        header = Adw.HeaderBar()
        header.set_show_end_title_buttons(False)
        header.set_show_start_title_buttons(False)
        header.set_title_widget(
            Adw.WindowTitle(
                title="Edit profile" if self.profile else "New profile",
                subtitle="Stored locally in the state service",
            )
        )
        cancel = w.button("Cancel", css="flat", on_click=lambda _button: self.close())
        header.pack_start(cancel)
        save = w.button(
            "Save",
            css="suggested-action",
            on_click=lambda _button: self._save(),
        )
        header.pack_end(save)
        toolbar.add_top_bar(header)

        self.prefs = Adw.PreferencesPage()
        toolbar.set_content(self.prefs)
        self.set_child(toolbar)

        self._build_details()
        self._build_schedule()
        self._build_apps()
        self._build_sites()
        self._build_keywords()
        self._build_allowlists()
        self._build_pickup()

    def _build_details(self) -> None:
        group = Adw.PreferencesGroup(title="Profile")
        self.name_row = Adw.EntryRow(title="Name")
        self.name_row.set_text(self.spec.get("name", ""))
        group.add(self.name_row)

        self.activation = Adw.ComboRow(title="Activation")
        self.activation.set_model(Gtk.StringList.new([label for _, label in ACTIVATIONS]))
        self.activation.set_selected(
            _index(ACTIVATIONS, self.spec.get("activation", "manual"))
        )
        self.activation.connect("notify::selected", self._on_activation_changed)
        group.add(self.activation)

        self.enabled = Adw.SwitchRow(
            title="Enabled",
            subtitle="Disabled profiles are ignored by the rule engine",
        )
        self.enabled.set_active(bool(self.spec.get("enabled", True)))
        group.add(self.enabled)

        self.strict = Adw.SwitchRow(
            title="Strict profile",
            subtitle="Matching rules always block while this profile is active",
        )
        self.strict.set_active(bool(self.spec.get("strict_eligible", False)))
        group.add(self.strict)

        self.priority = Adw.SpinRow.new_with_range(1, 9, 1)
        self.priority.set_title("Priority")
        self.priority.set_subtitle("Higher priority wins when profiles disagree")
        self.priority.set_value(int(self.spec.get("priority", 5)))
        group.add(self.priority)
        self.prefs.add(group)

    def _build_schedule(self) -> None:
        self.schedule_group = Adw.PreferencesGroup(
            title="Schedule",
            description="Schedules use local wall-clock time and are evaluated by the core.",
        )
        self.use_schedule = Adw.SwitchRow(title="Use a schedule")
        has_schedule = self.spec.get("schedule") is not None
        self.use_schedule.set_active(has_schedule)
        self.use_schedule.connect("notify::active", self._on_schedule_toggled)
        self.schedule_group.add(self.use_schedule)

        self.kind_combo = Adw.ComboRow(title="Pattern")
        self.kind_combo.set_model(
            Gtk.StringList.new([label for _, label in SCHEDULE_KINDS])
        )
        schedule = self.spec.get("schedule") or self._default_schedule()
        self.kind_combo.set_selected(_index(SCHEDULE_KINDS, schedule.get("kind", "daily")))
        self.kind_combo.connect("notify::selected", self._on_kind_changed)
        self.schedule_group.add(self.kind_combo)

        self.days_row = Adw.ActionRow(title="Days")
        days_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=2)
        days_box.set_valign(Gtk.Align.CENTER)
        selected_days = set(schedule.get("days") or [])
        for day in WEEKDAYS:
            toggle = Gtk.ToggleButton(label=day[0])
            toggle.add_css_class("day-toggle")
            toggle.set_tooltip_text(day)
            toggle.set_active(day in selected_days)
            toggle.connect("toggled", lambda _button: self._update_days())
            days_box.append(toggle)
            self._day_buttons.append(toggle)
        self.days_row.add_suffix(days_box)
        self.schedule_group.add(self.days_row)

        self.add_window_row = Adw.ActionRow(
            title="Add a time window",
            subtitle="Windows may cross midnight, for example 22:00–02:00",
            activatable=True,
        )
        self.add_window_row.add_suffix(Gtk.Image.new_from_icon_name("list-add-symbolic"))
        self.add_window_row.connect("activated", lambda _row: self._add_window())
        self.schedule_group.add(self.add_window_row)

        self.on_row = Adw.SpinRow.new_with_range(1, 60, 1)
        self.on_row.set_title("Minutes on")
        self.on_row.set_value(int(schedule.get("on_minutes") or 50))
        self.schedule_group.add(self.on_row)

        self.off_row = Adw.SpinRow.new_with_range(1, 60, 1)
        self.off_row.set_title("Minutes off")
        self.off_row.set_value(int(schedule.get("off_minutes") or 10))
        self.schedule_group.add(self.off_row)

        self.start_date_row = Adw.EntryRow(title="Start date")
        self.start_date_row.set_text(schedule.get("start_date") or dt.date.today().isoformat())
        self.start_date_row.set_tooltip_text("YYYY-MM-DD")
        self.schedule_group.add(self.start_date_row)

        self.end_date_row = Adw.EntryRow(title="End date")
        self.end_date_row.set_text(
            schedule.get("end_date")
            or (dt.date.today() + dt.timedelta(days=7)).isoformat()
        )
        self.end_date_row.set_tooltip_text("YYYY-MM-DD")
        self.schedule_group.add(self.end_date_row)

        self.prefs.add(self.schedule_group)
        self._render_windows(schedule.get("windows") or [])

    def _build_apps(self) -> None:
        self.apps_group = Adw.PreferencesGroup(title="Applications")
        choose_row = Adw.ActionRow(
            title="Add an application",
            subtitle="Choose an installed app by name; no application ID needed.",
        )
        choose_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        choose_box.set_valign(Gtk.Align.CENTER)
        self.app_action = Gtk.DropDown.new_from_strings([label for _, label in APP_ACTIONS])
        self.app_action.connect("notify::selected", self._on_app_action_changed)
        choose_box.append(self.app_action)
        self.app_minutes = Gtk.SpinButton.new_with_range(1, 480, 5)
        self.app_minutes.set_value(30)
        self.app_minutes.set_tooltip_text("Allowed minutes")
        self.app_minutes.set_visible(False)
        choose_box.append(self.app_minutes)
        choose_box.append(
            w.button(
                "Choose app…",
                css="suggested-action",
                tooltip="Choose an installed application",
                on_click=lambda _button: self._choose_app(),
            )
        )
        choose_row.add_suffix(choose_box)
        self.apps_group.add(choose_row)

        add_row = Adw.ActionRow(
            title="Advanced: application ID",
            subtitle="Only needed when an application is not in the chooser.",
        )
        add_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        add_box.set_valign(Gtk.Align.CENTER)
        self.app_entry = Gtk.Entry()
        self.app_entry.set_placeholder_text("e.g. firefox")
        self.app_entry.set_width_chars(16)
        self.app_entry.connect("activate", lambda _entry: self._add_app())
        add_box.append(self.app_entry)
        add_box.append(
            w.button(
                icon="list-add-symbolic",
                css="flat",
                tooltip="Add application ID",
                on_click=lambda _button: self._add_app(),
            )
        )
        add_row.add_suffix(add_box)
        self.apps_group.add(add_row)
        self.prefs.add(self.apps_group)

    def _build_sites(self) -> None:
        self.sites_group = Adw.PreferencesGroup(title="Websites")
        add_row = Adw.ActionRow(title="Add a site rule")
        add_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        add_box.set_valign(Gtk.Align.CENTER)
        self.site_entry = Gtk.Entry()
        self.site_entry.set_placeholder_text("domain or *.domain")
        self.site_entry.set_width_chars(20)
        self.site_entry.connect("activate", lambda _entry: self._add_site())
        add_box.append(self.site_entry)
        self.site_action = Gtk.DropDown.new_from_strings([label for _, label in SITE_ACTIONS])
        self.site_action.connect("notify::selected", self._on_site_action_changed)
        add_box.append(self.site_action)
        self.site_minutes = Gtk.SpinButton.new_with_range(1, 480, 5)
        self.site_minutes.set_value(30)
        self.site_minutes.set_tooltip_text("Allowed minutes")
        self.site_minutes.set_visible(False)
        add_box.append(self.site_minutes)
        add_box.append(
            w.button(
                icon="list-add-symbolic",
                css="flat",
                tooltip="Add site rule",
                on_click=lambda _button: self._add_site(),
            )
        )
        add_row.add_suffix(add_box)
        self.sites_group.add(add_row)
        self.prefs.add(self.sites_group)

    def _build_keywords(self) -> None:
        self.keywords_group = Adw.PreferencesGroup(
            title="Keywords",
            description="Matched against page titles and addresses, case-insensitively.",
        )
        self.keyword_row = Adw.EntryRow(title="Add a keyword")
        self.keyword_row.connect("activate", lambda _row: self._add_keyword())
        self.keyword_row.add_suffix(
            w.button(
                icon="list-add-symbolic",
                css="flat",
                tooltip="Add keyword",
                on_click=lambda _button: self._add_keyword(),
            )
        )
        self.keywords_group.add(self.keyword_row)
        self.prefs.add(self.keywords_group)

    def _build_allowlists(self) -> None:
        self.allow_group = Adw.PreferencesGroup(
            title="Allowlist",
            description="Allowlisted targets are permitted even when another rule matches.",
        )
        allow_choose = Adw.ActionRow(
            title="Allow an application",
            subtitle="Choose an installed app by name.",
        )
        allow_choose.add_suffix(
            w.button(
                "Choose app…",
                css="flat",
                tooltip="Choose an application to allow",
                on_click=lambda _button: self._choose_allow_app(),
            )
        )
        self.allow_group.add(allow_choose)

        self.allow_app_row = Adw.EntryRow(title="Advanced: allow an application ID")
        self.allow_app_row.connect("activate", lambda _row: self._add_allow_app())
        self.allow_app_row.add_suffix(
            w.button(
                icon="list-add-symbolic",
                css="flat",
                on_click=lambda _button: self._add_allow_app(),
            )
        )
        self.allow_group.add(self.allow_app_row)

        self.allow_site_row = Adw.EntryRow(title="Allow a website")
        self.allow_site_row.connect("activate", lambda _row: self._add_allow_site())
        self.allow_site_row.add_suffix(
            w.button(
                icon="list-add-symbolic",
                css="flat",
                on_click=lambda _button: self._add_allow_site(),
            )
        )
        self.allow_group.add(self.allow_site_row)
        self.prefs.add(self.allow_group)

    def _build_pickup(self) -> None:
        self.pickup_group = Adw.PreferencesGroup(title="Phone pickups")
        self.pickup_switch = Adw.SwitchRow(
            title="Limit pickups",
            subtitle="Recorded by the core when a pickup tracker is connected",
        )
        pickup = self.spec.get("pickup_limit")
        self.pickup_switch.set_active(pickup is not None)
        self.pickup_switch.connect("notify::active", self._update_pickup_visibility)
        self.pickup_group.add(self.pickup_switch)

        self.pickup_count = Adw.SpinRow.new_with_range(1, 500, 1)
        self.pickup_count.set_title("Pickups allowed")
        self.pickup_count.set_value(int((pickup or {}).get("count", 5)))
        self.pickup_group.add(self.pickup_count)

        self.pickup_period = Adw.ComboRow(title="Period")
        self.pickup_period.set_model(Gtk.StringList.new(["Daily", "Hourly"]))
        self.pickup_period.set_selected(
            1 if (pickup or {}).get("per") == "hourly" else 0
        )
        self.pickup_group.add(self.pickup_period)
        self.prefs.add(self.pickup_group)
        self._update_pickup_visibility()

    # -------------------------------------------------------------- schedule
    def _on_activation_changed(self, *_args) -> None:
        value = ACTIVATIONS[self.activation.get_selected()][0]
        if value == "scheduled" and not self.use_schedule.get_active():
            self.use_schedule.set_active(True)
        self._update_schedule_visibility()

    def _on_schedule_toggled(self, *_args) -> None:
        self._update_schedule_visibility()

    def _on_kind_changed(self, *_args) -> None:
        self._sync_window_entries()
        self._update_schedule_visibility()

    def _update_days(self) -> None:
        self._update_schedule_visibility()

    def _selected_days(self) -> list[str]:
        return [
            day
            for day, button in zip(WEEKDAYS, self._day_buttons)
            if button.get_active()
        ]

    def _update_schedule_visibility(self) -> None:
        enabled = self.use_schedule.get_active()
        kind = SCHEDULE_KINDS[self.kind_combo.get_selected()][0]
        self.kind_combo.set_sensitive(enabled)
        self.days_row.set_sensitive(enabled)
        self.add_window_row.set_sensitive(enabled)
        self.on_row.set_sensitive(enabled)
        self.off_row.set_sensitive(enabled)
        self.start_date_row.set_sensitive(enabled)
        self.end_date_row.set_sensitive(enabled)
        self.days_row.set_visible(kind in ("days", "full_day", "hourly"))
        self.on_row.set_visible(kind == "hourly")
        self.off_row.set_visible(kind == "hourly")
        self.start_date_row.set_visible(kind == "one_time")
        self.end_date_row.set_visible(kind == "one_time")
        self.add_window_row.set_visible(kind in ("daily", "days", "intervals", "one_time"))
        for row in self._window_rows:
            row.set_visible(
                kind in ("daily", "days", "intervals", "one_time")
            )

    def _render_windows(self, windows: list[dict]) -> None:
        for row in self._window_rows:
            self.schedule_group.remove(row)
        self._window_rows = []
        self._window_entries = []
        for index, window in enumerate(windows):
            row = Adw.ActionRow(title=f"Window {index + 1}")
            box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
            box.set_valign(Gtk.Align.CENTER)
            start = Gtk.Entry()
            start.set_width_chars(6)
            start.set_max_width_chars(6)
            start.set_text(window.get("start", "09:00"))
            start.set_tooltip_text("HH:MM")
            end = Gtk.Entry()
            end.set_width_chars(6)
            end.set_max_width_chars(6)
            end.set_text(window.get("end", "17:00"))
            end.set_tooltip_text("HH:MM")
            box.append(start)
            box.append(Gtk.Label(label="to"))
            box.append(end)
            box.append(
                w.button(
                    icon="user-trash-symbolic",
                    css="flat",
                    tooltip="Remove window",
                    on_click=lambda _button, position=index: self._remove_window(position),
                )
            )
            row.add_suffix(box)
            self.schedule_group.add(row)
            self._window_rows.append(row)
            self._window_entries.append((start, end))
        self.add_window_row.set_visible(
            SCHEDULE_KINDS[self.kind_combo.get_selected()][0]
            in ("daily", "days", "intervals", "one_time")
        )

    def _sync_window_entries(self) -> None:
        if not self._window_entries:
            return
        schedule = self.spec.get("schedule") or self._default_schedule()
        windows = [
            {"start": start.get_text().strip(), "end": end.get_text().strip()}
            for start, end in self._window_entries
        ]
        schedule["windows"] = windows
        self.spec["schedule"] = schedule

    def _add_window(self) -> None:
        self._sync_window_entries()
        schedule = self.spec.get("schedule") or self._default_schedule()
        windows = list(schedule.get("windows") or [])
        windows.append({"start": "13:00", "end": "14:00"})
        schedule["windows"] = windows
        self.spec["schedule"] = schedule
        self._render_windows(windows)
        self._update_schedule_visibility()

    def _remove_window(self, position: int) -> None:
        self._sync_window_entries()
        schedule = self.spec.get("schedule") or self._default_schedule()
        windows = list(schedule.get("windows") or [])
        if 0 <= position < len(windows):
            windows.pop(position)
        schedule["windows"] = windows
        self.spec["schedule"] = schedule
        self._render_windows(windows)
        self._update_schedule_visibility()

    # ------------------------------------------------------------------ rules
    def refresh_rules(self) -> None:
        self._render_app_rules()
        self._render_site_rules()
        self._render_keyword_rules()
        self._render_allow_rules()

    def _render_app_rules(self) -> None:
        for row in self._app_rows:
            self.apps_group.remove(row)
        self._app_rows = []
        for rule in self.spec.get("apps", []):
            key = rule.get("key", "?")
            name = self._app_names.get(key, key)
            description = rule_description(rule)
            subtitle = description if name == key else f"{key} · {description}"
            row = Adw.ActionRow()
            row.set_use_markup(False)
            row.set_title(name)
            row.set_subtitle(subtitle)
            row.add_suffix(
                w.button(
                    icon="user-trash-symbolic",
                    css="flat",
                    tooltip="Remove app rule",
                    on_click=lambda _button, item=rule: self._remove_rule("apps", item),
                )
            )
            self.apps_group.add(row)
            self._app_rows.append(row)

    def _render_site_rules(self) -> None:
        for row in self._site_rows:
            self.sites_group.remove(row)
        self._site_rows = []
        for rule in self.spec.get("sites", []):
            pattern = rule.get("pattern", "?")
            row = Adw.ActionRow(title=pattern, subtitle=rule_description(rule))
            row.add_suffix(
                w.button(
                    icon="user-trash-symbolic",
                    css="flat",
                    tooltip="Remove site rule",
                    on_click=lambda _button, item=rule: self._remove_rule("sites", item),
                )
            )
            self.sites_group.add(row)
            self._site_rows.append(row)

    def _render_keyword_rules(self) -> None:
        for row in self._keyword_rows:
            self.keywords_group.remove(row)
        self._keyword_rows = []
        for keyword in self.spec.get("keywords", []):
            row = Adw.ActionRow(title=f"“{keyword}”")
            row.add_suffix(
                w.button(
                    icon="user-trash-symbolic",
                    css="flat",
                    tooltip="Remove keyword",
                    on_click=lambda _button, item=keyword: self._remove_value(
                        "keywords", item
                    ),
                )
            )
            self.keywords_group.add(row)
            self._keyword_rows.append(row)

    def _render_allow_rules(self) -> None:
        for row in self._allow_app_rows:
            self.allow_group.remove(row)
        for row in self._allow_site_rows:
            self.allow_group.remove(row)
        self._allow_app_rows = []
        self._allow_site_rows = []
        for app in self.spec.get("allow_apps", []):
            name = self._app_names.get(app, app)
            subtitle = "Allowed application" if name == app else f"Allowed application · {app}"
            row = Adw.ActionRow()
            row.set_use_markup(False)
            row.set_title(name)
            row.set_subtitle(subtitle)
            row.add_suffix(
                w.button(
                    icon="user-trash-symbolic",
                    css="flat",
                    on_click=lambda _button, item=app: self._remove_value(
                        "allow_apps", item
                    ),
                )
            )
            self.allow_group.add(row)
            self._allow_app_rows.append(row)
        for site in self.spec.get("allow_sites", []):
            row = Adw.ActionRow(title=site, subtitle="Allowed website")
            row.add_suffix(
                w.button(
                    icon="user-trash-symbolic",
                    css="flat",
                    on_click=lambda _button, item=site: self._remove_value(
                        "allow_sites", item
                    ),
                )
            )
            self.allow_group.add(row)
            self._allow_site_rows.append(row)

    def _remove_rule(self, key: str, item: dict) -> None:
        values = self.spec.get(key, [])
        self.spec[key] = [value for value in values if value is not item]
        self.refresh_rules()

    def _remove_value(self, key: str, item: str) -> None:
        self.spec[key] = [value for value in self.spec.get(key, []) if value != item]
        self.refresh_rules()

    def _on_app_action_changed(self, *_args) -> None:
        action = APP_ACTIONS[self.app_action.get_selected()][0]
        self.app_minutes.set_visible(action != "block")

    def _on_site_action_changed(self, *_args) -> None:
        action = SITE_ACTIONS[self.site_action.get_selected()][0]
        self.site_minutes.set_visible(action == "daily_limit")

    def _choose_app(self) -> None:
        AppChooser(self._installed_apps, self._add_app_from_choice).present(self.window)

    def _add_app_from_choice(self, app: DesktopApp) -> None:
        action = APP_ACTIONS[self.app_action.get_selected()][0]
        rule = {"key": app.key, "action": action}
        if action != "block":
            rule["minutes"] = int(self.app_minutes.get_value())
        existing = next(
            (item for item in self.spec["apps"] if item.get("key") == app.key),
            None,
        )
        if existing is None:
            self.spec["apps"].append(rule)
        else:
            existing.clear()
            existing.update(rule)
        self._app_names[app.key] = app.name
        self._render_app_rules()

    def _add_app(self) -> None:
        key = self.app_entry.get_text().strip()
        if not key:
            return
        action = APP_ACTIONS[self.app_action.get_selected()][0]
        rule = {"key": key, "action": action}
        if action != "block":
            rule["minutes"] = int(self.app_minutes.get_value())
        self.spec["apps"].append(rule)
        self._app_names.setdefault(key, key)
        self.app_entry.set_text("")
        self._render_app_rules()

    def _add_site(self) -> None:
        pattern = self.site_entry.get_text().strip()
        if not pattern:
            return
        action = SITE_ACTIONS[self.site_action.get_selected()][0]
        rule = {"pattern": pattern, "action": action}
        if action == "daily_limit":
            rule["minutes"] = int(self.site_minutes.get_value())
        self.spec["sites"].append(rule)
        self.site_entry.set_text("")
        self._render_site_rules()

    def _add_keyword(self) -> None:
        keyword = self.keyword_row.get_text().strip()
        if not keyword:
            return
        if keyword not in self.spec["keywords"]:
            self.spec["keywords"].append(keyword)
        self.keyword_row.set_text("")
        self._render_keyword_rules()

    def _choose_allow_app(self) -> None:
        AppChooser(self._installed_apps, self._add_allow_app_from_choice).present(self.window)

    def _add_allow_app_from_choice(self, app: DesktopApp) -> None:
        if app.key not in self.spec["allow_apps"]:
            self.spec["allow_apps"].append(app.key)
        self._app_names[app.key] = app.name
        self._render_allow_rules()

    def _add_allow_app(self) -> None:
        value = self.allow_app_row.get_text().strip()
        if not value:
            return
        if value not in self.spec["allow_apps"]:
            self.spec["allow_apps"].append(value)
        self._app_names.setdefault(value, value)
        self.allow_app_row.set_text("")
        self._render_allow_rules()

    def _add_allow_site(self) -> None:
        value = self.allow_site_row.get_text().strip()
        if not value:
            return
        if value not in self.spec["allow_sites"]:
            self.spec["allow_sites"].append(value)
        self.allow_site_row.set_text("")
        self._render_allow_rules()

    def _update_pickup_visibility(self, *_args) -> None:
        active = self.pickup_switch.get_active()
        self.pickup_count.set_sensitive(active)
        self.pickup_period.set_sensitive(active)

    # ------------------------------------------------------------------- save
    def _collect(self) -> dict:
        name = self.name_row.get_text().strip()
        if not name:
            raise ValueError("Give the profile a name.")
        activation = ACTIVATIONS[self.activation.get_selected()][0]

        schedule = None
        if self.use_schedule.get_active():
            self._sync_window_entries()
            schedule = self.spec.get("schedule") or self._default_schedule()
            kind = SCHEDULE_KINDS[self.kind_combo.get_selected()][0]
            schedule = dict(schedule)
            schedule["kind"] = kind
            schedule["days"] = self._selected_days()
            schedule["on_minutes"] = int(self.on_row.get_value())
            schedule["off_minutes"] = int(self.off_row.get_value())
            schedule["start_date"] = self.start_date_row.get_text().strip()
            schedule["end_date"] = self.end_date_row.get_text().strip()
            if kind in ("daily", "days", "intervals", "one_time"):
                windows = schedule.get("windows") or []
                if not windows:
                    raise ValueError("Add at least one time window.")
                for window in windows:
                    for side in ("start", "end"):
                        value = window.get(side, "")
                        try:
                            dt.datetime.strptime(value, "%H:%M")
                        except ValueError as exc:
                            raise ValueError(
                                f"Time “{value}” is not in HH:MM format."
                            ) from exc
            if kind == "days" and not schedule["days"]:
                raise ValueError("Choose at least one day for this schedule.")
            if kind == "one_time":
                try:
                    start_date = dt.date.fromisoformat(schedule["start_date"])
                    end_date = dt.date.fromisoformat(schedule["end_date"])
                except ValueError as exc:
                    raise ValueError("Use YYYY-MM-DD for the date range.") from exc
                if end_date < start_date:
                    raise ValueError("The end date must not be before the start date.")
        if activation == "scheduled" and schedule is None:
            raise ValueError("Scheduled profiles need a schedule.")

        apps = []
        for rule in self.spec.get("apps", []):
            entry = {
                "key": rule.get("key", ""),
                "action": rule.get("action", "block"),
            }
            if rule.get("action") != "block":
                entry["minutes"] = int(rule.get("minutes", 30))
            apps.append(entry)
        sites = []
        for rule in self.spec.get("sites", []):
            entry = {
                "pattern": rule.get("pattern", ""),
                "action": rule.get("action", "block"),
            }
            if rule.get("action") == "daily_limit":
                entry["minutes"] = int(rule.get("minutes", 30))
            sites.append(entry)

        pickup = None
        if self.pickup_switch.get_active():
            pickup = {
                "count": int(self.pickup_count.get_value()),
                "per": "hourly" if self.pickup_period.get_selected() == 1 else "daily",
            }

        return {
            "name": name,
            "enabled": self.enabled.get_active(),
            "priority": int(self.priority.get_value()),
            "activation": activation,
            "strict_eligible": self.strict.get_active(),
            "schedule": schedule,
            "apps": apps,
            "sites": sites,
            "keywords": list(self.spec.get("keywords", [])),
            "allow_apps": list(self.spec.get("allow_apps", [])),
            "allow_sites": list(self.spec.get("allow_sites", [])),
            "pickup_limit": pickup,
        }

    def _save(self) -> None:
        try:
            spec = self._collect()
            if self.profile:
                self.window.engine.update_profile(self.profile["id"], spec)
                message = "Profile saved"
            else:
                self.window.engine.create_profile(spec)
                message = "Profile created"
        except Exception as error:  # noqa: BLE001
            w.alert(self, "Could not save profile", error_text(error))
            return
        self.close()
        self.window.refresh()
        self.window.toast(message)
