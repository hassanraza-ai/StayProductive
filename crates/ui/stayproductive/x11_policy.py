"""Pure policy matching and block planning for the X11 adapter.

This module has no GTK or Xlib imports so the important decisions can be tested
without a display. The adapter process feeds it one ``FocusEvent`` per poll and
executes the returned actions; the planner never talks to X11 itself.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Iterable, Sequence

# Reason text is original copy, not extracted from any reference artifact.
REASON_TEXT = {
    "R-PROFILE": "A focus profile is active.",
    "R-STRICT": "A strict session is active.",
    "R-QUICK": "A quick block is active.",
    "R-LIMIT": "A usage limit has been reached.",
    "R-LIMIT-DAILY": "The daily limit for this app has been reached.",
    "R-LIMIT-HOURLY": "The hourly limit for this app has been reached.",
    "R-LAUNCH": "This app was launched during a focus session.",
    "R-PICKUP": "This app is blocked while pickup protection is active.",
    "R-WAIT": "This app is blocked for the configured waiting period.",
    "R-KEYWORD": "A keyword rule matched.",
    "R-SITE": "A site rule matched.",
    "R-SURFACE": "This surface is blocked.",
}


def reason_text(code: str | None) -> str:
    return REASON_TEXT.get(code or "", "A StayProductive rule applies.")


def normalize_app(value: str | None) -> str:
    """Lower-case an app id and drop a single trailing ``.desktop``."""

    if not isinstance(value, str):
        return ""
    key = value.strip().lower()
    if key.endswith(".desktop"):
        key = key[: -len(".desktop")]
    if not key or any(character.isspace() or ord(character) < 32 for character in key):
        return ""
    return key


def app_aliases(values: Iterable[str | None]) -> set[str]:
    """Window identifiers that may match a rule key.

    A rule key is usually a desktop id (``org.mozilla.firefox``) or a
    ``StartupWMClass`` value (``firefox``). Window managers report WM_CLASS,
    application ids, or process names, so the last dotted and underscored
    components are added as aliases. This mirrors the GNOME helper's matching
    rules without importing a compositor.
    """

    aliases: set[str] = set()
    for value in values:
        key = normalize_app(value)
        if not key:
            continue
        aliases.add(key)
        for separator in (".", "_"):
            tail = key.rsplit(separator, 1)[-1]
            if tail and tail != key:
                aliases.add(tail)
    return aliases


@dataclass(frozen=True)
class PolicyEntry:
    app: str
    reason: str
    until: str | None = None
    profile: str | None = None


@dataclass(frozen=True)
class Policy:
    valid_until: datetime
    apps: tuple[PolicyEntry, ...]
    aliases: dict[str, PolicyEntry] = field(default_factory=dict)

    def find(self, identifiers: Iterable[str]) -> PolicyEntry | None:
        for alias in app_aliases(identifiers):
            entry = self.aliases.get(alias)
            if entry is not None:
                return entry
        return None


def parse_timestamp(value: Any) -> datetime | None:
    if not isinstance(value, str) or not value:
        return None
    text = value.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def policy_from_json(value: Any, now: datetime | None = None) -> Policy | None:
    """Parse a policy envelope, returning ``None`` when it is unusable or stale."""

    if not isinstance(value, dict) or value.get("version") != 1:
        return None
    valid_until = parse_timestamp(value.get("valid_until"))
    if valid_until is None:
        return None
    now = (now or datetime.now(timezone.utc)).astimezone(timezone.utc)
    if valid_until < now:
        return None
    entries: list[PolicyEntry] = []
    aliases: dict[str, PolicyEntry] = {}
    raw_apps = value.get("apps")
    if not isinstance(raw_apps, list):
        return None
    for raw in raw_apps:
        if not isinstance(raw, dict):
            continue
        app = normalize_app(raw.get("app"))
        if not app:
            continue
        entry = PolicyEntry(
            app=app,
            reason=str(raw.get("reason") or ""),
            until=str(raw["until"]) if raw.get("until") else None,
            profile=str(raw["profile"]) if raw.get("profile") else None,
        )
        entries.append(entry)
        for alias in app_aliases([app]):
            aliases.setdefault(alias, entry)
    return Policy(valid_until=valid_until, apps=tuple(entries), aliases=aliases)


@dataclass(frozen=True)
class Rect:
    x: int
    y: int
    width: int
    height: int


def _intersection_area(first: Rect, second: Rect) -> int:
    width = (
        min(first.x + first.width, second.x + second.width) - max(first.x, second.x)
    )
    height = (
        min(first.y + first.height, second.y + second.height) - max(first.y, second.y)
    )
    return max(0, width) * max(0, height)


def preferred_monitor(window: Rect | None, monitors: Sequence[Rect]) -> int:
    """Index of the monitor that should show the block card.

    The card belongs where the user is looking, so the monitor sharing the
    most area with the blocked window wins. A window that is entirely off
    screen, or a missing geometry, falls back to the first monitor: every
    monitor still gets covered, so enforcement never weakens.
    """

    if not monitors:
        return -1
    best = -1
    best_area = 0
    for index, monitor in enumerate(monitors):
        if window is None:
            break
        area = _intersection_area(window, monitor)
        if area > best_area:
            best_area = area
            best = index
    if best >= 0:
        return best
    return 0


@dataclass(frozen=True)
class FocusEvent:
    window: int
    identifiers: frozenset[str]
    label: str = "This app"
    normal: bool = True


class FocusReporter:
    """Tracks the focused app and when it became foreground.

    Usage limits accrue only while an application window is focused and no
    block overlay is up. The interval stays stable while the same app keeps
    focus; the reporter forgets it while the overlay is visible or the session
    is paused, so time behind a block can never be re-attributed afterwards.
    """

    def __init__(self) -> None:
        self.aliases: tuple[str, ...] = ()
        self.since: datetime | None = None

    def observe(
        self,
        event: FocusEvent | None,
        now: datetime,
        *,
        blocked: bool = False,
    ) -> None:
        if blocked or event is None or not event.normal:
            self.reset()
            return
        aliases = tuple(sorted(app_aliases(event.identifiers)))
        if not aliases:
            self.reset()
            return
        if aliases != self.aliases:
            self.aliases = aliases
            self.since = now

    def reset(self) -> None:
        self.aliases = ()
        self.since = None


@dataclass(frozen=True)
class Action:
    kind: str
    window: int | None = None
    entry: PolicyEntry | None = None
    label: str = ""


class Planner:
    """Turns focus events into hide/overlay/focus actions, in a fixed order.

    The planner is deliberately stateful only about what it has already
    enforced, so a repeated focus event is a no-op and a policy refresh does not
    re-hide an already hidden window.
    """

    def __init__(self) -> None:
        self.overlay_visible = False
        self.blocked_window: int | None = None
        self.blocked_label = ""
        self.blocked_entry: PolicyEntry | None = None
        self.previous_allowed: int | None = None
        self.blocks = 0
        self.paused = False

    def update(self, event: FocusEvent | None, policy: Policy | None) -> list[Action]:
        if self.paused:
            return []
        if policy is None:
            return self.clear()
        if event is None:
            return []
        if not event.normal:
            # Panels, docks and menus are not applications; keep any current
            # block rather than treating the panel as an allowed app.
            return []

        entry = policy.find(event.identifiers)
        if entry is None:
            self.previous_allowed = event.window
            if self.overlay_visible:
                self._reset_block()
                return [Action("hide_overlay")]
            return []

        if (
            self.overlay_visible
            and self.blocked_window == event.window
            and self.blocked_entry == entry
        ):
            self.blocked_label = event.label
            return []

        actions: list[Action] = []
        if self.blocked_window != event.window:
            # Hiding the old blocked window is not needed when the user has
            # already focused a different blocked app; the old one stays hidden.
            actions.append(Action("hide_window", window=event.window))
        actions.append(
            Action(
                "show_overlay",
                window=event.window,
                entry=entry,
                label=event.label,
            )
        )
        if not self.overlay_visible or self.blocked_window != event.window:
            self.blocks += 1
        self.overlay_visible = True
        self.blocked_window = event.window
        self.blocked_label = event.label
        self.blocked_entry = entry
        return actions

    def clear(self) -> list[Action]:
        actions: list[Action] = []
        if self.overlay_visible:
            actions.append(Action("hide_overlay"))
        if self.blocked_window is not None:
            # The policy disappeared (daemon stopped or the block expired).
            # Enforcement must stop, and the window we unmapped/iconified must
            # become reachable again instead of staying hidden.
            actions.append(Action("restore_window", window=self.blocked_window))
        self._reset_block()
        return actions

    def back(self) -> list[Action]:
        """Leave the overlay for the last allowed window, if there is one."""

        if self.paused or not self.overlay_visible or self.previous_allowed is None:
            return []
        actions = [Action("focus_window", window=self.previous_allowed)]
        actions.append(Action("hide_overlay"))
        self._reset_block()
        return actions

    def pause(self) -> list[Action]:
        """Release the overlay for a lock screen or suspend.

        The blocked window stays hidden; only the explanation goes away so it
        never covers a lock screen or a security prompt.
        """

        if self.paused:
            return []
        self.paused = True
        if not self.overlay_visible:
            return []
        self.overlay_visible = False
        return [Action("hide_overlay")]

    def resume(self) -> None:
        """Let the next focus event re-block the still-hidden window."""

        self.paused = False

    def _reset_block(self) -> None:
        self.overlay_visible = False
        self.blocked_window = None
        self.blocked_label = ""
        self.blocked_entry = None
