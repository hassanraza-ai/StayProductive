"""Tests for onboarding goal/Profile logic; no display required."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from stayproductive.apps import DesktopApp
from stayproductive.onboarding import (
    GOAL_BY_KEY,
    GOALS,
    StarterProfileCreator,
    app_aliases,
    apps_for_goal,
    mark_onboarding_complete,
    onboarding_complete,
    onboarding_marker,
    should_show_onboarding,
    starter_profile,
)


def app(key: str, name: str, desktop_id: str) -> DesktopApp:
    return DesktopApp(key=key, name=name, desktop_id=desktop_id)


STEAM = app("steam", "Steam", "steam.desktop")
APP_CENTER = app("snap-store", "App Center", "snap-store_snap-store.desktop")
BRIDGE = app(
    "gog_com-bridge_constructor_1",
    "Bridge Constructor",
    "gog_com-Bridge_Constructor_1.desktop",
)
BACKUPS = app("org.gnome.dejadup", "Backups", "org.gnome.DejaDup.desktop")
FIREFOX = app("firefox_firefox", "Firefox", "firefox_firefox.desktop")


class OnboardingGoalsTest(unittest.TestCase):
    def test_goal_definitions_are_consistent(self) -> None:
        names = {goal.profile_name for goal in GOALS}
        self.assertEqual(len(names), len(GOALS))
        for goal in GOALS:
            self.assertIn(goal.activation, ("manual", "scheduled", "always"))
            if goal.activation == "scheduled":
                self.assertIsNotNone(goal.schedule, goal.key)
            else:
                self.assertIsNone(goal.schedule, goal.key)
            spec = starter_profile(goal, [])
            self.assertEqual(spec["name"], goal.profile_name)
            self.assertEqual(spec["activation"], goal.activation)
            self.assertEqual(len(spec["sites"]), len(goal.sites))

    def test_custom_goal_starts_empty(self) -> None:
        spec = starter_profile(GOAL_BY_KEY["custom"], [STEAM, FIREFOX])
        self.assertEqual(spec["apps"], [])
        self.assertEqual(spec["sites"], [])
        self.assertEqual(spec["keywords"], [])
        self.assertEqual(spec["activation"], "manual")
        self.assertNotIn("schedule", spec)

    def test_sleep_profile_crosses_midnight(self) -> None:
        spec = starter_profile(GOAL_BY_KEY["sleep"], [])
        window = spec["schedule"]["windows"][0]
        self.assertEqual(window, {"start": "22:00", "end": "07:00"})


class AppMatchTest(unittest.TestCase):
    def test_desktop_snap_and_gog_forms_share_aliases(self) -> None:
        self.assertIn("steam", app_aliases(STEAM))
        self.assertIn("snap-store", app_aliases(APP_CENTER))
        self.assertIn("bridge_constructor", app_aliases(BRIDGE))
        self.assertIn("dejadup", app_aliases(BACKUPS))
        self.assertIn("firefox", app_aliases(FIREFOX))

    def test_numeric_suffix_is_not_a_generic_alias(self) -> None:
        self.assertNotIn("1", app_aliases(BRIDGE))

    def test_goal_selects_installed_apps_only(self) -> None:
        installed = [STEAM, APP_CENTER, BRIDGE, BACKUPS, FIREFOX]
        selected = apps_for_goal(GOAL_BY_KEY["work"], installed)
        keys = [item.key for item in selected]
        self.assertIn(STEAM.key, keys)
        self.assertIn(BRIDGE.key, keys)
        self.assertNotIn(APP_CENTER.key, keys)
        self.assertNotIn(BACKUPS.key, keys)
        self.assertNotIn(FIREFOX.key, keys)

    def test_social_goal_selects_chat_apps_by_alias(self) -> None:
        discord = app("discord", "Discord", "discord.desktop")
        signal = app("signal-desktop", "Signal", "signal-desktop.desktop")
        selected = apps_for_goal(GOAL_BY_KEY["social"], [discord, signal, STEAM])
        self.assertEqual({item.key for item in selected}, {"discord", "signal-desktop"})

    def test_goal_selection_follows_hint_priority_and_limit(self) -> None:
        selected = apps_for_goal(
            GOAL_BY_KEY["work"], [BRIDGE, STEAM, FIREFOX], limit=1
        )
        self.assertEqual([item.key for item in selected], [STEAM.key])


class StarterProfileTest(unittest.TestCase):
    def test_selected_apps_are_deduplicated_and_ordered(self) -> None:
        spec = starter_profile(
            GOAL_BY_KEY["study"],
            [STEAM, FIREFOX],
            selected_apps=[FIREFOX, STEAM, FIREFOX],
        )
        self.assertEqual(
            spec["apps"],
            [
                {"key": "firefox_firefox", "action": "block"},
                {"key": "steam", "action": "block"},
            ],
        )

    def test_name_override_is_trimmed(self) -> None:
        spec = starter_profile(GOAL_BY_KEY["work"], [], name="  Morning focus  ")
        self.assertEqual(spec["name"], "Morning focus")

    def test_blank_name_falls_back_to_the_goal_name(self) -> None:
        spec = starter_profile(GOAL_BY_KEY["work"], [], name="   ")
        self.assertEqual(spec["name"], "Work hours")


class OnboardingStateTest(unittest.TestCase):
    def test_only_a_clean_snapshot_auto_shows_onboarding(self) -> None:
        self.assertTrue(
            should_show_onboarding({"profiles": [], "commitment": None}, completed=False)
        )
        self.assertFalse(
            should_show_onboarding({"profiles": [], "commitment": None}, completed=True)
        )
        self.assertFalse(
            should_show_onboarding(
                {"profiles": [{"name": "Work"}], "commitment": None}, completed=False
            )
        )
        self.assertFalse(
            should_show_onboarding(
                {"profiles": [], "commitment": {"id": "c1"}}, completed=False
            )
        )
        self.assertFalse(should_show_onboarding({}, completed=False))

    def test_marker_lives_next_to_the_state_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            state = Path(temp) / "state.json"
            snapshot = {"state_path": str(state)}
            self.assertEqual(onboarding_marker(snapshot), Path(temp) / "onboarding.json")
            self.assertFalse(onboarding_complete(snapshot))
            self.assertTrue(mark_onboarding_complete(snapshot))
            self.assertTrue(onboarding_complete(snapshot))
            payload = json.loads((Path(temp) / "onboarding.json").read_text())
            self.assertEqual(payload["version"], 1)
            self.assertFalse(
                should_show_onboarding({"state_path": str(state), "profiles": []})
            )

    def test_marker_write_is_best_effort_without_a_state_path(self) -> None:
        self.assertIsNone(onboarding_marker({}))
        self.assertFalse(mark_onboarding_complete({}))


class FakeEngine:
    def __init__(self, fail: bool = False) -> None:
        self.created: list[dict] = []
        self.fail = fail

    def create_profile(self, spec: dict) -> dict:
        if self.fail:
            raise RuntimeError("daemon refused")
        self.created.append(spec)
        return spec


class StarterProfileCreatorTest(unittest.TestCase):
    def test_a_ux_01_creates_exactly_one_profile_and_marks_onboarding(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            snapshot = {"state_path": str(Path(temp) / "state.json")}
            engine = FakeEngine()
            creator = StarterProfileCreator(engine, snapshot)
            first = creator.create(
                GOAL_BY_KEY["custom"], [], selected_apps=[], name="Focus"
            )
            second = creator.create(
                GOAL_BY_KEY["custom"], [], selected_apps=[], name="Focus"
            )
            self.assertIsNotNone(first)
            self.assertIsNone(second)
            self.assertEqual(creator.calls, 1)
            self.assertEqual(len(engine.created), 1)
            self.assertTrue(onboarding_complete(snapshot))

    def test_a_ux_01_a_failed_creation_can_be_retried(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            snapshot = {"state_path": str(Path(temp) / "state.json")}
            creator = StarterProfileCreator(FakeEngine(fail=True), snapshot)
            with self.assertRaises(RuntimeError):
                creator.create(
                    GOAL_BY_KEY["custom"], [], selected_apps=[], name="Focus"
                )
            self.assertEqual(creator.calls, 0)
            self.assertIsNone(creator.spec)
            self.assertFalse(onboarding_complete(snapshot))

    def test_creator_does_not_enable_optional_capabilities(self) -> None:
        # The starter profile is data only. Onboarding's optional capabilities
        # are unavailable and have no code path that turns them on.
        spec = starter_profile(GOAL_BY_KEY["work"], [STEAM], selected_apps=[STEAM])
        self.assertFalse(spec["strict_eligible"])
        self.assertEqual(spec["allow_apps"], [])
        self.assertEqual(spec["allow_sites"], [])
        self.assertIsNone(spec["pickup_limit"])


if __name__ == "__main__":
    unittest.main()
