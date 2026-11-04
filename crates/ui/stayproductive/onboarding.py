"""First-run onboarding wizard (F-UX-01).

The wizard is deliberately boring: five short steps, one primary action at a
time, and nothing enabled without an explicit click. The goal logic is kept in
pure functions so it can be tested without a display.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")

from gi.repository import Adw, Gtk

from . import desktop, service
from . import widgets as w
from .app_picker import AppChooser
from .apps import DesktopApp, installed_apps
from .formatting import error_text, schedule_summary

STEP_NAMES = ("welcome", "goal", "permissions", "profile", "done")
STEP_TITLES = {
    "welcome": "Welcome",
    "goal": "Choose a goal",
    "permissions": "Capabilities",
    "profile": "Your first profile",
    "done": "You're set",
}


@dataclass(frozen=True)
class Goal:
    """One plain-language starting point for a first profile."""

    key: str
    title: str
    description: str
    icon: str
    profile_name: str
    activation: str
    schedule: dict | None
    app_hints: tuple[str, ...] = ()
    sites: tuple[str, ...] = ()
    keywords: tuple[str, ...] = ()


def _days_schedule(days: tuple[str, ...], start: str, end: str) -> dict:
    return {
        "kind": "days",
        "days": list(days),
        "windows": [{"start": start, "end": end}],
    }


def _daily_schedule(start: str, end: str) -> dict:
    return {"kind": "daily", "days": [], "windows": [{"start": start, "end": end}]}


GAME_AND_LAUNCHER_HINTS = (
    "steam",
    "lutris",
    "heroic",
    "bottles",
    "minecraft",
    "mindustry",
    "openttd",
    "warzone2100",
    "terraria",
    "rimworld",
    "project_zomboid",
    "kingdoms_and_castles",
    "mini_metro",
    "train_fever",
    "transport_fever",
    "universe_sandbox",
    "space_haven",
    "imperator_rome",
    "lego_bricktales",
    "bridge_constructor",
    "project_hospital",
)

GOALS: tuple[Goal, ...] = (
    Goal(
        key="social",
        title="Social",
        description="Keep feeds and chat from pulling you away.",
        icon="system-users-symbolic",
        profile_name="Social media limits",
        activation="manual",
        schedule=None,
        app_hints=(
            "discord",
            "telegramdesktop",
            "telegram-desktop",
            "signal",
            "signal-desktop",
            "element",
            "whatsapp",
            "whatsapp-for-linux",
            "caprine",
        ),
        sites=("instagram.com", "tiktok.com", "facebook.com", "x.com", "reddit.com"),
    ),
    Goal(
        key="work",
        title="Work",
        description="Keep games and feeds out of working hours.",
        icon="computer-symbolic",
        profile_name="Work hours",
        activation="scheduled",
        schedule=_days_schedule(("Mon", "Tue", "Wed", "Thu", "Fri"), "09:00", "17:00"),
        app_hints=GAME_AND_LAUNCHER_HINTS,
        sites=("youtube.com", "x.com", "reddit.com", "twitch.tv"),
        keywords=("shorts", "reels"),
    ),
    Goal(
        key="study",
        title="Study",
        description="Protect a daily block for coursework or deep learning.",
        icon="view-paged-symbolic",
        profile_name="Study block",
        activation="scheduled",
        schedule=_daily_schedule("18:00", "22:00"),
        app_hints=GAME_AND_LAUNCHER_HINTS,
        sites=("youtube.com", "tiktok.com", "instagram.com", "reddit.com", "x.com"),
        keywords=("shorts", "reels"),
    ),
    Goal(
        key="sleep",
        title="Sleep",
        description="Wind down at night and keep the morning quiet.",
        icon="weather-clear-night-symbolic",
        profile_name="Evening wind-down",
        activation="scheduled",
        schedule=_daily_schedule("22:00", "07:00"),
        app_hints=("steam", "vlc", "haruna", "org.gnome.totem", "rhythmbox", "spotify"),
        sites=("youtube.com", "tiktok.com", "instagram.com", "netflix.com", "twitch.tv"),
        keywords=("shorts", "reels"),
    ),
    Goal(
        key="custom",
        title="Custom",
        description="Start empty and choose every rule yourself.",
        icon="applications-other-symbolic",
        profile_name="My focus profile",
        activation="manual",
        schedule=None,
    ),
)

GOAL_BY_KEY = {goal.key: goal for goal in GOALS}

# A first profile should feel curated, not exhaustive. Users can add more apps
# from the picker in step 4.
MAX_STARTER_APPS = 6


def _strip_desktop(value: str) -> str:
    value = value.strip().casefold()
    if value.endswith(".desktop"):
        value = value[:-8]
    return value


def app_aliases(app: DesktopApp) -> set[str]:
    """Names a goal hint may use to identify one installed application.

    Desktop entry ids, executable-style keys, Snap's ``<snap>_<app>`` form and
    GOG's ``gog_com-<name>`` form all name the same launchable application. The
    chooser stores one stable key, but a goal should still find the app.
    """

    values = [app.key, app.name, _strip_desktop(app.desktop_id)]
    stem = _strip_desktop(app.desktop_id)
    if stem.startswith("gog_com-"):
        values.append(stem[len("gog_com-") :])
    aliases: set[str] = set()
    for value in values:
        value = value.strip().casefold()
        if not value:
            continue
        aliases.add(value)
        if "." in value:
            tail = value.rsplit(".", 1)[1]
            if len(tail) > 1 and not tail.isdigit():
                aliases.add(tail)
        if "_" in value:
            head, tail = value.rsplit("_", 1)
            if len(tail) > 1 and not tail.isdigit():
                aliases.add(tail)
            if tail.isdigit() and head:
                aliases.add(head)
        if value.startswith("gog_com-"):
            gog = value[len("gog_com-") :]
            if "_" in gog:
                base, suffix = gog.rsplit("_", 1)
                if suffix.isdigit() and base:
                    aliases.add(base)
    return aliases


def apps_for_goal(
    goal: Goal, apps: list[DesktopApp], *, limit: int | None = None
) -> list[DesktopApp]:
    """Installed apps that match a goal's hints, in hint priority order."""
    hints = [_strip_desktop(hint) for hint in goal.app_hints]
    if not hints:
        return []
    selected: list[DesktopApp] = []
    seen: set[str] = set()
    for hint in hints:
        for app in apps:
            if app.key in seen:
                continue
            if hint in app_aliases(app):
                selected.append(app)
                seen.add(app.key)
        if limit is not None and len(selected) >= limit:
            break
    if limit is not None:
        return selected[:limit]
    return selected


def starter_profile(
    goal: Goal,
    apps: list[DesktopApp],
    *,
    selected_apps: list[DesktopApp] | None = None,
    name: str | None = None,
) -> dict:
    """Build the profile specification the daemon accepts.

    ``selected_apps`` is the user's edited list from step 4. When it is omitted,
    the goal's pre-selected apps are used.
    """

    chosen = (
        apps_for_goal(goal, apps, limit=MAX_STARTER_APPS)
        if selected_apps is None
        else selected_apps
    )
    unique: list[DesktopApp] = []
    seen: set[str] = set()
    for app in chosen:
        if app.key not in seen:
            seen.add(app.key)
            unique.append(app)

    spec: dict = {
        "name": (name or goal.profile_name).strip() or goal.profile_name,
        "enabled": True,
        "priority": 5,
        "activation": goal.activation,
        "strict_eligible": False,
        "apps": [{"key": app.key, "action": "block"} for app in unique],
        "sites": [{"pattern": site, "action": "block"} for site in goal.sites],
        "keywords": list(goal.keywords),
        "allow_apps": [],
        "allow_sites": [],
        "pickup_limit": None,
    }
    if goal.schedule is not None:
        spec["schedule"] = goal.schedule
    return spec


def onboarding_marker(snapshot: dict) -> Path | None:
    state_path = snapshot.get("state_path")
    if not state_path:
        return None
    return Path(state_path).with_name("onboarding.json")


def onboarding_complete(snapshot: dict) -> bool:
    path = onboarding_marker(snapshot)
    return bool(path and path.is_file())


def should_show_onboarding(snapshot: dict, *, completed: bool | None = None) -> bool:
    """Auto-show only on a genuinely clean first run."""
    if not snapshot:
        return False
    if snapshot.get("commitment"):
        return False
    if completed is None:
        completed = onboarding_complete(snapshot)
    return not completed and not snapshot.get("profiles")


def mark_onboarding_complete(snapshot: dict) -> bool:
    path = onboarding_marker(snapshot)
    if path is None:
        return False
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            json.dumps({"version": 1, "wizard": "first-run"}) + "\n",
            encoding="utf-8",
        )
    except OSError:
        return False
    return True


class StarterProfileCreator:
    """Create at most one starter profile for one wizard run.

    The guard is explicit so the acceptance rule "onboarding creates exactly one
    profile" is testable without constructing GTK widgets. A failed daemon call
    leaves the guard open so the user can correct the name and retry.
    """

    def __init__(self, engine, snapshot: dict) -> None:
        self.engine = engine
        self.snapshot = snapshot
        self.spec: dict | None = None
        self.calls = 0

    def create(
        self,
        goal: Goal,
        apps: list[DesktopApp],
        *,
        selected_apps: list[DesktopApp],
        name: str,
    ) -> dict | None:
        if self.spec is not None:
            return None
        spec = starter_profile(goal, apps, selected_apps=selected_apps, name=name)
        self.engine.create_profile(spec)
        self.calls += 1
        mark_onboarding_complete(self.snapshot)
        self.spec = spec
        return spec


class Onboarding(Adw.Dialog):
    """Five-step first-run wizard. It creates at most one profile per run."""

    def __init__(self, window, *, first_run: bool = False) -> None:
        super().__init__()
        self.window = window
        self.first_run = first_run
        self.goal: Goal | None = None
        self._installed = installed_apps()
        self._selected_apps: list[DesktopApp] = []
        self._site_rows: list[Gtk.Widget] = []
        self._keyword_rows: list[Gtk.Widget] = []
        self._profile_goal_key: str | None = None
        self._creator = StarterProfileCreator(window.engine, window.snapshot)
        self._build()

    # ------------------------------------------------------------------ build
    def _build(self) -> None:
        self.set_content_width(760)
        self.set_content_height(700)
        self.set_title("Welcome")

        toolbar = Adw.ToolbarView()
        self.header = Adw.HeaderBar()
        self.header.set_show_end_title_buttons(False)
        self.header.set_show_start_title_buttons(False)
        self.title_widget = Adw.WindowTitle(
            title=STEP_TITLES["welcome"], subtitle="Step 1 of 5"
        )
        self.header.set_title_widget(self.title_widget)
        self.header.pack_end(
            w.button(
                icon="window-close-symbolic",
                css="flat",
                tooltip="Close setup",
                on_click=lambda _button: self.close(),
            )
        )
        toolbar.add_top_bar(self.header)

        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.stack = Gtk.Stack()
        self.stack.set_transition_type(Gtk.StackTransitionType.SLIDE_LEFT_RIGHT)
        self.stack.set_transition_duration(180)
        self.stack.set_vexpand(True)
        self.stack.add_named(self._build_welcome(), "welcome")
        self.stack.add_named(self._build_goal(), "goal")
        self.stack.add_named(self._build_permissions(), "permissions")
        self.stack.add_named(self._build_profile(), "profile")
        self.stack.add_named(self._build_done(), "done")
        content.append(self.stack)

        footer = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        footer.set_margin_top(10)
        footer.set_margin_bottom(14)
        footer.set_margin_start(18)
        footer.set_margin_end(18)
        self.back_button = w.button(
            "Back",
            css="flat",
            tooltip="Go back one step",
            on_click=lambda _button: self._step(-1),
        )
        footer.append(self.back_button)
        spacer = Gtk.Box()
        spacer.set_hexpand(True)
        footer.append(spacer)
        self.skip_button = w.button(
            "Skip optional",
            css="flat",
            tooltip="Continue without enabling optional capabilities",
            on_click=lambda _button: self._show_step(3),
        )
        footer.append(self.skip_button)
        self.primary_button = w.button(
            "Continue",
            css="suggested-action",
            on_click=lambda _button: self._primary(),
        )
        footer.append(self.primary_button)
        content.append(footer)

        toolbar.set_content(content)
        self.set_child(toolbar)
        self.connect("closed", lambda _dialog: setattr(self.window, "onboarding", None))
        self._show_step(0)

    @staticmethod
    def _step_page(child: Gtk.Widget) -> Gtk.Widget:
        scroller = Gtk.ScrolledWindow()
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroller.set_vexpand(True)
        scroller.set_child(child)
        return scroller

    def _page(self) -> Gtk.Box:
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16)
        box.set_margin_top(26)
        box.set_margin_bottom(22)
        box.set_margin_start(26)
        box.set_margin_end(26)
        return box

    def _build_welcome(self) -> Gtk.Widget:
        page = self._page()
        icon = Gtk.Image.new_from_icon_name("preferences-system-time-symbolic")
        icon.set_pixel_size(72)
        icon.set_halign(Gtk.Align.CENTER)
        icon.set_margin_top(18)
        page.append(icon)
        title = w.label("Focus with intent", "hero-title")
        title.set_halign(Gtk.Align.CENTER)
        page.append(title)
        intro = w.label(
            "StayProductive blocks the apps and sites you choose, using local "
            "rules on this device. Five short steps and you are ready.",
            "muted",
        )
        intro.set_halign(Gtk.Align.CENTER)
        intro.set_justify(Gtk.Justification.CENTER)
        intro.set_max_width_chars(58)
        page.append(intro)

        note = w.Card(spacing=8)
        note.append(w.label("Not a parental-control tool", "card-title"))
        note.append(
            w.label(
                "You decide every rule, and you can change or remove them later. "
                "There is no account, purchase, telemetry, or hidden exit.",
                "muted",
            )
        )
        page.append(note)
        return self._step_page(page)

    def _build_goal(self) -> Gtk.Widget:
        page = self._page()
        page.append(w.label("Where do you want to focus first?", "page-title"))
        page.append(
            w.label(
                "Pick a starting point. The next steps show a profile you can "
                "edit before anything is saved.",
                "muted",
            )
        )
        self.goal_list = Gtk.ListBox()
        self.goal_list.set_selection_mode(Gtk.SelectionMode.SINGLE)
        self.goal_list.add_css_class("boxed-list")
        self.goal_list.set_margin_top(4)
        for goal in GOALS:
            row = Adw.ActionRow(
                title=goal.title, subtitle=goal.description, activatable=True
            )
            row.set_use_markup(False)
            row.add_prefix(w.icon_badge(goal.icon, 18))
            row.goal = goal  # type: ignore[attr-defined]
            self.goal_list.append(row)
        self.goal_list.connect("row-selected", self._on_goal_selected)
        page.append(self.goal_list)
        return self._step_page(page)

    def _build_permissions(self) -> Gtk.Widget:
        page = self._page()
        page.append(w.label("What this build can see and do", "page-title"))
        page.append(
            w.label(
                "These capabilities are listed with the reason they are needed. "
                "Nothing is enabled merely by opening this page.",
                "muted",
            )
        )

        required = Adw.PreferencesGroup(
            title="Required for enforcement",
            description=(
                "Foreground app visibility is how the engine knows which "
                "application is on screen. Without it, rules are decisions only."
            ),
        )
        self.enforcement_row = Adw.ActionRow(title="Foreground app visibility")
        self.enforcement_pill = w.status_pill("Checking", "neutral")
        self.enforcement_row.add_suffix(self.enforcement_pill)
        self.enforcement_button = w.button(
            "Set up",
            css="flat",
            tooltip="Install or enable the local desktop adapter",
            on_click=lambda _button: self._set_up_enforcement(),
        )
        self.enforcement_row.add_suffix(self.enforcement_button)
        required.add(self.enforcement_row)
        page.append(required)

        optional = Adw.PreferencesGroup(
            title="Optional capabilities",
            description=(
                "Autostart can be granted here. Notifications and the DNS helper "
                "are not available in this build; skipping them keeps the app "
                "local."
            ),
        )
        self.autostart_row = Adw.ActionRow(title="Start at login")
        self.autostart_pill = w.status_pill("Checking", "neutral")
        self.autostart_row.add_suffix(self.autostart_pill)
        self.autostart_button = w.button(
            "Set up",
            css="flat",
            tooltip="Enable the systemd user service at login",
            on_click=lambda _button: self._set_up_autostart(),
        )
        self.autostart_row.add_suffix(self.autostart_button)
        optional.add(self.autostart_row)
        notifications = Adw.ActionRow(
            title="Notifications",
            subtitle=(
                "Not available yet. Low-time alerts are planned but nothing is "
                "sent today."
            ),
        )
        notifications.add_suffix(w.status_pill("Not available", "neutral"))
        optional.add(notifications)
        dns = Adw.ActionRow(
            title="Local DNS helper",
            subtitle=(
                "Optional system-wide website blocking. Not available in this "
                "build; no resolver settings were touched."
            ),
        )
        dns.add_suffix(w.status_pill("Optional", "neutral"))
        optional.add(dns)
        page.append(optional)

        page.append(
            w.label(
                "You can change all of this later in Settings. Website and "
                "keyword enforcement is not active in this build; application "
                "blocks are.",
                "muted small",
            )
        )
        return self._step_page(page)

    def _build_profile(self) -> Gtk.Widget:
        page = self._page()
        page.append(w.label("Review and create", "page-title"))
        self.profile_intro = w.label("", "muted")
        page.append(self.profile_intro)

        profile_group = Adw.PreferencesGroup(title="Profile")
        self.name_row = Adw.EntryRow(title="Profile name")
        self.name_row.connect("changed", lambda _row: self._update_buttons())
        profile_group.add(self.name_row)
        self.schedule_row = Adw.ActionRow(title="Schedule")
        self.schedule_pill = w.status_pill("Manual", "accent")
        self.schedule_row.add_suffix(self.schedule_pill)
        profile_group.add(self.schedule_row)
        page.append(profile_group)

        apps_card = w.Card(spacing=10)
        apps_head = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        apps_head.append(w.label("Applications", "card-title"))
        spacer = Gtk.Box()
        spacer.set_hexpand(True)
        apps_head.append(spacer)
        apps_head.append(
            w.button(
                "Add app…",
                "list-add-symbolic",
                css="flat",
                tooltip="Choose an installed application",
                on_click=lambda _button: self._choose_app(),
            )
        )
        apps_card.append(apps_head)
        apps_card.append(
            w.label(
                "Pre-selected from your goal. Remove any you do not want, or add "
                "another installed application.",
                "muted small",
            )
        )
        self.apps_flow = Gtk.FlowBox()
        self.apps_flow.set_selection_mode(Gtk.SelectionMode.NONE)
        self.apps_flow.set_homogeneous(False)
        self.apps_flow.set_max_children_per_line(3)
        self.apps_flow.set_row_spacing(6)
        self.apps_flow.set_column_spacing(6)
        apps_card.append(self.apps_flow)
        page.append(apps_card)

        self.sites_group = Adw.PreferencesGroup(title="Websites")
        page.append(self.sites_group)

        self.keywords_group = Adw.PreferencesGroup(title="Keywords")
        page.append(self.keywords_group)

        page.append(
            w.label(
                "Website and keyword rules are saved, but this build only "
                "enforces application blocks. Site enforcement is still being "
                "built.",
                "muted small",
            )
        )
        return self._step_page(page)

    def _build_done(self) -> Gtk.Widget:
        page = self._page()
        icon = Gtk.Image.new_from_icon_name("emblem-ok-symbolic")
        icon.set_pixel_size(72)
        icon.set_halign(Gtk.Align.CENTER)
        icon.set_margin_top(30)
        page.append(icon)
        title = w.label("You're set", "hero-title")
        title.set_halign(Gtk.Align.CENTER)
        page.append(title)
        self.done_body = w.label("", "muted")
        self.done_body.set_halign(Gtk.Align.CENTER)
        self.done_body.set_justify(Gtk.Justification.CENTER)
        self.done_body.set_max_width_chars(58)
        page.append(self.done_body)

        note = w.Card(spacing=8)
        note.append(w.label("Everything stays on this device", "card-title"))
        note.append(
            w.label(
                "No account was created and no analytics consent was requested. "
                "You can edit the profile, add rules, or run this setup again "
                "from Settings → Help.",
                "muted",
            )
        )
        page.append(note)
        return self._step_page(page)

    # ------------------------------------------------------------- interaction
    def _show_step(self, index: int) -> None:
        index = max(0, min(index, len(STEP_NAMES) - 1))
        name = STEP_NAMES[index]
        self.stack.set_visible_child_name(name)
        self.title_widget.set_title(STEP_TITLES[name])
        self.title_widget.set_subtitle(
            "Setup complete" if name == "done" else f"Step {index + 1} of 5"
        )
        if name == "profile":
            self._refresh_profile_step()
        elif name == "permissions":
            self._refresh_capabilities()
        self._update_buttons()

    def _step(self, delta: int) -> None:
        current = self.stack.get_visible_child_name() or "welcome"
        self._show_step(STEP_NAMES.index(current) + delta)

    def _primary(self) -> None:
        current = self.stack.get_visible_child_name() or "welcome"
        if current == "welcome":
            self._show_step(1)
        elif current == "goal":
            if self.goal is not None:
                self._show_step(2)
        elif current == "permissions":
            self._show_step(3)
        elif current == "profile":
            self._create_profile()
        elif current == "done":
            self.window.switch_page("dashboard")
            self.close()

    def _update_buttons(self) -> None:
        current = self.stack.get_visible_child_name() or "welcome"
        self.back_button.set_visible(current not in ("welcome", "done"))
        self.skip_button.set_visible(current == "permissions")
        if current == "goal":
            self.primary_button.set_label("Continue")
            self.primary_button.set_sensitive(self.goal is not None)
        elif current == "profile":
            self.primary_button.set_label("Create profile")
            self.primary_button.set_sensitive(
                bool(self.name_row.get_text().strip())
                and self._creator.spec is None
            )
        elif current == "done":
            self.primary_button.set_label("Open dashboard")
            self.primary_button.set_sensitive(True)
        else:
            self.primary_button.set_label("Continue")
            self.primary_button.set_sensitive(True)

    def _on_goal_selected(self, _listbox: Gtk.ListBox, row: Gtk.ListBoxRow | None) -> None:
        self.goal = getattr(row, "goal", None) if row is not None else None
        self._update_buttons()

    def _apply_goal_to_profile(self) -> None:
        if self.goal is None:
            self.goal = GOAL_BY_KEY["custom"]
        self.name_row.set_text(self.goal.profile_name)
        self._selected_apps = apps_for_goal(
            self.goal, self._installed, limit=MAX_STARTER_APPS
        )
        self._render_apps()
        self._render_sites()
        self._render_keywords()
        schedule = self.goal.schedule
        self.schedule_row.set_subtitle(
            schedule_summary(schedule) if schedule else "Manual · you start it when you want"
        )
        self._set_pill(
            self.schedule_pill,
            "Scheduled" if schedule else "Manual",
            "accent" if schedule else "idle",
        )
        if self.goal.key == "custom":
            self.profile_intro.set_label(
                "Start empty, or add the applications you want to block now. "
                "You can add sites and keywords later in the editor."
            )
        else:
            self.profile_intro.set_label(
                f"Based on “{self.goal.title}”. Everything here can be edited "
                "before you create the profile."
            )
        self._profile_goal_key = self.goal.key

    def _refresh_profile_step(self) -> None:
        current = self.goal.key if self.goal else None
        if self._profile_goal_key != current:
            self._apply_goal_to_profile()

    def _render_apps(self) -> None:
        child = self.apps_flow.get_first_child()
        while child is not None:
            next_child = child.get_next_sibling()
            self.apps_flow.remove(child)
            child = next_child
        if not self._selected_apps:
            self.apps_flow.append(
                w.label(
                    "No applications selected. Add one, or leave this profile "
                    "without app rules.",
                    "muted small",
                )
            )
            return
        for app in self._selected_apps:
            self.apps_flow.append(self._app_chip(app))

    def _app_chip(self, app: DesktopApp) -> Gtk.Widget:
        chip = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=2)
        chip.add_css_class("app-chip")
        chip.set_tooltip_text(app.key)
        name = Gtk.Label(label=app.name)
        name.set_ellipsize(3)
        name.set_max_width_chars(22)
        chip.append(name)
        remove = w.button(
            icon="window-close-symbolic",
            css="flat",
            tooltip=f"Remove {app.name}",
            on_click=lambda _button, item=app: self._remove_app(item),
        )
        remove.add_css_class("chip-remove")
        chip.append(remove)
        return chip

    def _render_sites(self) -> None:
        for row in self._site_rows:
            self.sites_group.remove(row)
        self._site_rows = []
        assert self.goal is not None
        if not self.goal.sites:
            self.sites_group.set_visible(False)
            return
        self.sites_group.set_visible(True)
        row = Adw.ActionRow(
            title="Blocked sites",
            subtitle=" · ".join(self.goal.sites),
        )
        row.set_use_markup(False)
        self.sites_group.add(row)
        self._site_rows.append(row)

    def _render_keywords(self) -> None:
        for row in self._keyword_rows:
            self.keywords_group.remove(row)
        self._keyword_rows = []
        assert self.goal is not None
        if not self.goal.keywords:
            self.keywords_group.set_visible(False)
            return
        self.keywords_group.set_visible(True)
        row = Adw.ActionRow(
            title="Content keywords",
            subtitle=", ".join(self.goal.keywords),
        )
        row.set_use_markup(False)
        self.keywords_group.add(row)
        self._keyword_rows.append(row)

    @staticmethod
    def _set_pill(pill: Gtk.Label, text: str, kind: str) -> None:
        for value in ("active", "warning", "neutral", "accent", "idle"):
            pill.remove_css_class(f"pill-{value}")
        pill.add_css_class(f"pill-{kind}")
        pill.set_label(text.upper())

    def _choose_app(self) -> None:
        chosen = {app.key for app in self._selected_apps}
        available = [app for app in self._installed if app.key not in chosen]
        AppChooser(available, self._add_chosen_app).present(self)

    def _add_chosen_app(self, app: DesktopApp) -> None:
        if app.key not in {item.key for item in self._selected_apps}:
            self._selected_apps.append(app)
            self._selected_apps.sort(key=lambda item: (item.name.casefold(), item.key))
            self._render_apps()

    def _remove_app(self, app: DesktopApp) -> None:
        self._selected_apps = [item for item in self._selected_apps if item.key != app.key]
        self._render_apps()

    def _refresh_capabilities(self) -> None:
        gnome = desktop.is_gnome()
        installed = desktop.extension_installed()
        enabled = desktop.extension_enabled()
        enforcement = self.window.snapshot.get("enforcement", {})
        active = enforcement.get("state") == "active"
        adapter = enforcement.get("adapter")

        self.enforcement_button.set_visible(True)
        if active and adapter == "x11":
            self._set_pill(self.enforcement_pill, "Active", "active")
            self.enforcement_row.set_subtitle(
                "The daemon's X11 adapter is enforcing app blocks in this session."
            )
            self.enforcement_button.set_visible(False)
        elif not gnome:
            self._set_pill(self.enforcement_pill, "Unavailable", "warning")
            self.enforcement_row.set_subtitle(
                f"No desktop adapter is reporting for {desktop.session_desktop_name()} "
                "in this session. Rules still run and are recorded; windows cannot "
                "be blocked here yet."
            )
            self.enforcement_button.set_visible(False)
        elif not installed:
            self._set_pill(self.enforcement_pill, "Not set up", "warning")
            self.enforcement_row.set_subtitle(
                "Install the local GNOME Shell extension to report the focused "
                "application and apply app blocks."
            )
            self.enforcement_button.set_label("Install")
        elif not enabled:
            self._set_pill(self.enforcement_pill, "Disabled", "warning")
            self.enforcement_row.set_subtitle(
                "The extension is installed but disabled. Enable it, then log "
                "out and back in once so GNOME Shell loads it."
            )
            self.enforcement_button.set_label("Enable")
        elif active:
            self._set_pill(self.enforcement_pill, "Active", "active")
            self.enforcement_row.set_subtitle(
                "The extension is enabled and reporting. Application blocks are "
                "applied on native Wayland and XWayland windows."
            )
            self.enforcement_button.set_visible(False)
        else:
            self._set_pill(self.enforcement_pill, "Waiting", "warning")
            self.enforcement_row.set_subtitle(
                "Enabled, waiting for GNOME Shell. Log out and back in once after "
                "installation."
            )
            self.enforcement_button.set_label("Reinstall")
        self._refresh_autostart()

    def _refresh_autostart(self) -> None:
        state = service.service_state(self.window.snapshot.get("state_path"))
        if state == "unavailable":
            self._set_pill(self.autostart_pill, "Unavailable", "neutral")
            self.autostart_row.set_subtitle(
                "No systemd user manager in this session. Blocking still "
                "continues while the background service runs."
            )
            self.autostart_button.set_visible(False)
        elif state == "custom-state":
            self._set_pill(self.autostart_pill, "Unavailable", "neutral")
            self.autostart_row.set_subtitle(
                "Only offered for the default data file. This session uses a "
                "custom one."
            )
            self.autostart_button.set_visible(False)
        elif state == "enabled":
            self._set_pill(self.autostart_pill, "On", "active")
            self.autostart_row.set_subtitle(
                "Enabled. systemd starts the daemon at login and restarts it if "
                "it is killed."
            )
            self.autostart_button.set_visible(False)
        else:
            self._set_pill(self.autostart_pill, "Off", "neutral")
            self.autostart_row.set_subtitle(
                "Optional. Enabling it keeps the background service running "
                "after logout; nothing is enabled until you choose Set up."
            )
            self.autostart_button.set_visible(True)
            self.autostart_button.set_sensitive(True)

    def _set_up_autostart(self) -> None:
        try:
            service.enable(
                self.window.engine, self.window.snapshot.get("state_path")
            )
        except service.ServiceError as error:
            self.window.show_error("Could not enable autostart", str(error))
            self._refresh_autostart()
            return
        self.window.toast("Autostart enabled · the daemon starts at login")
        self.window.refresh()
        self._refresh_autostart()

    def _set_up_enforcement(self) -> None:
        try:
            desktop.copy_extension()
            if not desktop.extension_enabled():
                desktop.enable_extension()
        except desktop.DesktopError as error:
            self.window.show_error("Could not set up enforcement", str(error))
            return
        self.window.toast("GNOME extension installed · log out and back in once")
        self.window.refresh()
        self._refresh_capabilities()

    def _create_profile(self) -> None:
        if self._creator.spec is not None:
            return
        name = self.name_row.get_text().strip()
        if not name:
            self.window.show_error("Name the profile", "Enter a profile name first.")
            return
        self._creator.snapshot = self.window.snapshot
        try:
            spec = self._creator.create(
                self.goal or GOAL_BY_KEY["custom"],
                self._installed,
                selected_apps=self._selected_apps,
                name=name,
            )
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not create the profile", error_text(error))
            return
        if spec is None:
            return
        self.window.refresh()
        self._update_buttons()
        self.done_body.set_label(
            f"“{name}” is saved. Manual profiles start when you turn them on; "
            "scheduled profiles start at their next window. Everything stays on "
            "this device and no account was created."
        )
        self._show_step(4)
