"""Presentation helpers shared by the GTK pages."""

from __future__ import annotations

from datetime import datetime, timezone
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from .engine import EngineError

WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
ACTIVATION_LABELS = {
    "manual": "Manual",
    "scheduled": "Scheduled",
    "always": "Always on",
}
SCHEDULE_LABELS = {
    "daily": "Every day",
    "days": "Selected days",
    "intervals": "Split windows",
    "full_day": "Full day",
    "hourly": "Hourly cycle",
    "one_time": "Date range",
}
ACTION_LABELS = {
    "block": "Block",
    "adult": "Block adult",
    "daily_limit": "Daily limit",
    "hourly_limit": "Hourly limit",
    "launch_limit": "Launch limit",
    "wait_timer": "Wait timer",
    "goal": "Goal",
}


def parse_time(value: str | None) -> datetime | None:
    if not value:
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def timezone_for(name: str) -> ZoneInfo:
    try:
        return ZoneInfo(name)
    except (ZoneInfoNotFoundError, ValueError):
        return ZoneInfo("UTC")


def local_time(value: str | None, tz_name: str, include_date: bool = False) -> str:
    moment = parse_time(value)
    if moment is None:
        return "—"
    local = moment.astimezone(timezone_for(tz_name))
    if include_date:
        return local.strftime("%a %d %b · %H:%M")
    return local.strftime("%H:%M")


def local_day(value: str | None, tz_name: str) -> str:
    moment = parse_time(value)
    if moment is None:
        return "—"
    return moment.astimezone(timezone_for(tz_name)).strftime("%a %d %b")


def relative(value: str | None, now_raw: str | None) -> str:
    moment = parse_time(value)
    now = parse_time(now_raw) or datetime.now(timezone.utc)
    if moment is None:
        return "—"
    seconds = (moment - now).total_seconds()
    if seconds <= 0:
        return "now"
    minutes = int(seconds // 60)
    if minutes < 1:
        return "less than a minute"
    if minutes < 60:
        return f"in {minutes} min"
    hours, minutes = divmod(minutes, 60)
    if hours < 24:
        return f"in {hours} h {minutes:02d} min" if minutes else f"in {hours} h"
    days, hours = divmod(hours, 24)
    return f"in {days} d {hours} h" if hours else f"in {days} d"


def remaining_seconds(value: str | None, now_raw: str | None) -> int:
    moment = parse_time(value)
    now = parse_time(now_raw) or datetime.now(timezone.utc)
    if moment is None:
        return 0
    return max(0, int((moment - now).total_seconds()))


def countdown(seconds: int) -> str:
    seconds = max(0, seconds)
    hours, rest = divmod(seconds, 3600)
    minutes, seconds = divmod(rest, 60)
    if hours:
        return f"{hours}:{minutes:02d}:{seconds:02d}"
    return f"{minutes}:{seconds:02d}"


def schedule_summary(schedule: dict | None) -> str:
    if not schedule:
        return "No schedule"
    kind = schedule.get("kind", "daily")
    label = SCHEDULE_LABELS.get(kind, kind.replace("_", " ").title())
    days = schedule.get("days") or []
    if kind in ("daily", "intervals", "one_time"):
        day_text = ""
    elif kind == "one_time":
        day_text = ""
    elif days:
        if set(days) == {"Mon", "Tue", "Wed", "Thu", "Fri"}:
            day_text = "Mon–Fri"
        elif set(days) == set(WEEKDAYS):
            day_text = "Every day"
        else:
            day_text = ", ".join(days)
    else:
        day_text = "Every day"

    if kind == "full_day":
        return f"{label} · {day_text}" if day_text else label
    if kind == "hourly":
        on = schedule.get("on_minutes")
        off = schedule.get("off_minutes")
        cycle = f"{on} min on / {off} min off" if on is not None else label
        return f"{cycle} · {day_text}" if day_text else cycle
    if kind == "one_time":
        start = schedule.get("start_date") or "?"
        end = schedule.get("end_date") or "?"
        windows = _windows_summary(schedule)
        return f"{start} → {end} · {windows}"
    windows = _windows_summary(schedule)
    return f"{day_text} · {windows}" if day_text else windows


def _windows_summary(schedule: dict) -> str:
    windows = schedule.get("windows") or []
    if not windows:
        return "No windows"
    return " · ".join(
        f"{window.get('start', '?')}–{window.get('end', '?')}" for window in windows
    )


def rule_count(profile: dict) -> int:
    return (
        len(profile.get("apps") or [])
        + len(profile.get("sites") or [])
        + len(profile.get("keywords") or [])
        + (1 if profile.get("pickup_limit") else 0)
    )


def profile_subtitle(profile: dict) -> str:
    activation = ACTIVATION_LABELS.get(profile.get("activation", "manual"), "Manual")
    rules = rule_count(profile)
    rule_text = f"{rules} rule{'s' if rules != 1 else ''}"
    if profile.get("schedule"):
        return f"{activation} · {rule_text} · {schedule_summary(profile['schedule'])}"
    if profile.get("activation") == "scheduled":
        return f"Schedule missing · {rule_text}"
    return f"{activation} · {rule_text}"


def rule_description(rule: dict) -> str:
    action = rule.get("action", "block")
    label = ACTION_LABELS.get(action, action.replace("_", " ").title())
    if action in ("daily_limit", "hourly_limit"):
        return f"{label} · {rule.get('minutes', '?')} min"
    if action == "launch_limit":
        return f"{label} · {rule.get('count', '?')}× {rule.get('per', 'daily')}"
    if action == "wait_timer":
        return (
            f"Use {rule.get('use_for_minutes', '?')} min, "
            f"wait {rule.get('wait_minutes', '?')} min"
        )
    return label


def commitment_remaining(commitment: dict, now_raw: str | None) -> str:
    return countdown(remaining_seconds(commitment.get("ends_at"), now_raw))


def error_text(error: Exception) -> str:
    if isinstance(error, EngineError):
        return str(error)
    return f"Unexpected error: {error}"
