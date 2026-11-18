"""Display-free tests for X11 policy matching and the block planner."""

from __future__ import annotations

import unittest
from datetime import datetime, timedelta, timezone

from stayproductive.x11_policy import (
    Action,
    FocusEvent,
    FocusReporter,
    Planner,
    Rect,
    app_aliases,
    policy_from_json,
    preferred_monitor,
    reason_text,
)


def policy(now: datetime, apps: list[dict], seconds: int = 60):
    return policy_from_json(
        {
            "version": 1,
            "generated_at": now.isoformat().replace("+00:00", "Z"),
            "valid_until": (now + timedelta(seconds=seconds))
            .isoformat()
            .replace("+00:00", "Z"),
            "apps": apps,
        },
        now,
    )


class AliasTest(unittest.TestCase):
    def test_desktop_ids_and_wm_class_forms_share_aliases(self) -> None:
        self.assertEqual(
            app_aliases(["org.mozilla.firefox.desktop", "Navigator"]),
            {"org.mozilla.firefox", "firefox", "navigator"},
        )
        self.assertEqual(
            app_aliases(["org.gnome.Nautilus", "org.gnome.Nautilus"]),
            {"org.gnome.nautilus", "nautilus"},
        )
        self.assertEqual(
            app_aliases(["snap.code.code"]), {"snap.code.code", "code"}
        )

    def test_empty_and_control_values_are_ignored(self) -> None:
        self.assertEqual(app_aliases(["", "  ", "bad\tkey"]), set())


class PolicyTest(unittest.TestCase):
    def setUp(self) -> None:
        self.now = datetime(2026, 11, 3, 12, 0, tzinfo=timezone.utc)

    def test_matching_uses_window_aliases(self) -> None:
        parsed = policy(
            self.now,
            [
                {
                    "app": "org.mozilla.firefox",
                    "reason": "R-PROFILE",
                    "until": "2026-11-03T13:00:00Z",
                    "profile": "Work",
                }
            ],
        )
        assert parsed is not None
        self.assertEqual(parsed.find(["Navigator", "firefox"]).app, "org.mozilla.firefox")
        self.assertIsNone(parsed.find(["code"]))

    def test_expired_or_malformed_policy_is_not_used(self) -> None:
        self.assertIsNone(policy(self.now, [], seconds=-1))
        self.assertIsNone(policy_from_json({"version": 2}, self.now))
        self.assertIsNone(policy_from_json("not a policy", self.now))
        self.assertIsNone(
            policy_from_json(
                {"version": 1, "valid_until": "not-a-date", "apps": []}, self.now
            )
        )

    def test_unknown_reason_has_generic_copy(self) -> None:
        self.assertIn("rule applies", reason_text("R-SOMETHING"))
        self.assertIn("strict", reason_text("R-STRICT"))


class PlannerTest(unittest.TestCase):
    def setUp(self) -> None:
        self.now = datetime(2026, 11, 3, 12, 0, tzinfo=timezone.utc)
        self.policy = policy(
            self.now,
            [
                {"app": "firefox", "reason": "R-PROFILE", "until": None},
                {"app": "code", "reason": "R-QUICK", "until": None},
            ],
        )
        assert self.policy is not None
        self.planner = Planner()

    def event(self, window: int, name: str) -> FocusEvent:
        return FocusEvent(window=window, identifiers=frozenset({name}), label=name)

    def test_allowed_focus_does_nothing(self) -> None:
        self.assertEqual(self.planner.update(self.event(10, "nautilus"), self.policy), [])
        self.assertFalse(self.planner.overlay_visible)

    def test_block_hides_the_window_before_showing_the_overlay(self) -> None:
        actions = self.planner.update(self.event(11, "firefox"), self.policy)
        self.assertEqual([action.kind for action in actions], ["hide_window", "show_overlay"])
        self.assertEqual(actions[0].window, 11)
        self.assertEqual(actions[1].entry.app, "firefox")
        self.assertTrue(self.planner.overlay_visible)
        self.assertEqual(self.planner.blocks, 1)

    def test_repeated_focus_is_deduped(self) -> None:
        self.planner.update(self.event(11, "firefox"), self.policy)
        self.assertEqual(self.planner.update(self.event(11, "firefox"), self.policy), [])
        self.assertEqual(self.planner.blocks, 1)

    def test_switching_to_an_allowed_window_hides_the_overlay(self) -> None:
        self.planner.update(self.event(11, "firefox"), self.policy)
        actions = self.planner.update(self.event(10, "nautilus"), self.policy)
        self.assertEqual([action.kind for action in actions], ["hide_overlay"])
        self.assertFalse(self.planner.overlay_visible)
        self.assertEqual(self.planner.previous_allowed, 10)

    def test_switch_to_another_blocked_app_hides_it_and_replaces_overlay(self) -> None:
        self.planner.update(self.event(11, "firefox"), self.policy)
        actions = self.planner.update(self.event(12, "code"), self.policy)
        self.assertEqual([action.kind for action in actions], ["hide_window", "show_overlay"])
        self.assertEqual(actions[0].window, 12)
        self.assertEqual(self.planner.blocks, 2)

    def test_back_focuses_the_previous_allowed_window_then_hides(self) -> None:
        self.planner.update(self.event(10, "nautilus"), self.policy)
        self.planner.update(self.event(11, "firefox"), self.policy)
        actions = self.planner.back()
        self.assertEqual([action.kind for action in actions], ["focus_window", "hide_overlay"])
        self.assertEqual(actions[0].window, 10)
        self.assertFalse(self.planner.overlay_visible)

    def test_back_without_a_previous_window_keeps_the_overlay(self) -> None:
        self.planner.update(self.event(11, "firefox"), self.policy)
        self.assertEqual(self.planner.back(), [])
        self.assertTrue(self.planner.overlay_visible)

    def test_expired_policy_or_unsupported_window_clears_the_overlay(self) -> None:
        self.planner.update(self.event(11, "firefox"), self.policy)
        actions = self.planner.update(None, None)
        self.assertEqual(
            [action.kind for action in actions], ["hide_overlay", "restore_window"]
        )
        self.assertEqual(actions[1].window, 11)
        self.assertFalse(self.planner.overlay_visible)

        self.planner.update(self.event(11, "firefox"), self.policy)
        panel = FocusEvent(20, frozenset({"gnome-shell"}), normal=False)
        self.assertEqual(self.planner.update(panel, self.policy), [])
        self.assertTrue(self.planner.overlay_visible)

    def test_clear_restores_the_blocked_window_and_is_idempotent(self) -> None:
        self.planner.update(self.event(11, "firefox"), self.policy)
        self.assertEqual(
            [action.kind for action in self.planner.clear()],
            ["hide_overlay", "restore_window"],
        )
        self.assertEqual(self.planner.clear(), [])

    def test_policy_update_refreshes_the_overlay_without_rehiding(self) -> None:
        self.planner.update(self.event(11, "firefox"), self.policy)
        changed = policy(
            self.now,
            [{"app": "firefox", "reason": "R-STRICT", "until": None}],
        )
        actions = self.planner.update(self.event(11, "firefox"), changed)
        self.assertEqual([action.kind for action in actions], ["show_overlay"])
        self.assertEqual(self.planner.blocked_entry.reason, "R-STRICT")

    def test_actions_are_comparable_dataclasses(self) -> None:
        self.assertEqual(Action("hide_overlay"), Action("hide_overlay"))


class PreferredMonitorTest(unittest.TestCase):
    """The card belongs on the monitor the blocked window was on."""

    LEFT = Rect(0, 0, 1920, 1080)
    RIGHT = Rect(1920, 0, 1920, 1080)

    def test_single_monitor(self) -> None:
        self.assertEqual(
            preferred_monitor(Rect(100, 100, 800, 600), [self.LEFT]),
            0,
        )

    def test_window_on_the_right_monitor(self) -> None:
        window = Rect(2400, 100, 800, 600)
        self.assertEqual(
            preferred_monitor(window, [self.LEFT, self.RIGHT]),
            1,
        )

    def test_window_on_the_left_monitor(self) -> None:
        window = Rect(200, 100, 800, 600)
        self.assertEqual(
            preferred_monitor(window, [self.LEFT, self.RIGHT]),
            0,
        )

    def test_spanning_window_uses_the_larger_share(self) -> None:
        window = Rect(1200, 0, 1400, 1080)
        self.assertEqual(
            preferred_monitor(window, [self.LEFT, self.RIGHT]),
            0,
        )
        self.assertEqual(
            preferred_monitor(Rect(1200, 0, 1500, 1080), [self.LEFT, self.RIGHT]),
            1,
        )

    def test_off_screen_window_falls_back_to_the_first_monitor(self) -> None:
        window = Rect(5000, 4000, 800, 600)
        self.assertEqual(
            preferred_monitor(window, [self.LEFT, self.RIGHT]),
            0,
        )

    def test_missing_geometry_falls_back_to_the_first_monitor(self) -> None:
        self.assertEqual(
            preferred_monitor(None, [self.LEFT, self.RIGHT]),
            0,
        )

    def test_no_monitors_reports_no_card(self) -> None:
        self.assertEqual(preferred_monitor(self.LEFT, []), -1)
        self.assertEqual(preferred_monitor(None, []), -1)


class PlannerPauseTest(unittest.TestCase):
    def setUp(self) -> None:
        self.now = datetime(2026, 11, 3, 12, 0, tzinfo=timezone.utc)
        self.policy = policy(
            self.now,
            [{"app": "firefox", "reason": "R-PROFILE", "until": None}],
        )
        assert self.policy is not None
        self.planner = Planner()

    def event(self, window: int, name: str) -> FocusEvent:
        return FocusEvent(window=window, identifiers=frozenset({name}), label=name)

    def test_pause_hides_the_overlay(self) -> None:
        self.planner.update(self.event(11, "firefox"), self.policy)
        self.assertEqual(
            [action.kind for action in self.planner.pause()],
            ["hide_overlay"],
        )
        self.assertTrue(self.planner.paused)
        self.assertFalse(self.planner.overlay_visible)

    def test_pause_without_an_overlay_does_nothing(self) -> None:
        self.assertEqual(self.planner.pause(), [])
        self.assertEqual(self.planner.pause(), [])

    def test_updates_are_ignored_while_paused(self) -> None:
        self.planner.update(self.event(11, "firefox"), self.policy)
        self.planner.pause()
        self.assertEqual(
            self.planner.update(self.event(11, "firefox"), self.policy), []
        )
        self.assertEqual(self.planner.update(self.event(10, "nautilus"), self.policy), [])
        self.assertEqual(self.planner.update(self.event(11, "firefox"), None), [])

    def test_back_is_ignored_while_paused(self) -> None:
        self.planner.update(self.event(10, "nautilus"), self.policy)
        self.planner.update(self.event(11, "firefox"), self.policy)
        self.planner.pause()
        self.assertEqual(self.planner.back(), [])
        self.assertEqual(self.planner.back(), [])

    def test_resume_reblocks_the_still_hidden_window(self) -> None:
        self.planner.update(self.event(11, "firefox"), self.policy)
        self.planner.pause()
        self.planner.resume()
        self.assertFalse(self.planner.paused)
        actions = self.planner.update(self.event(11, "firefox"), self.policy)
        # The window is already hidden; only the explanation comes back.
        self.assertEqual([action.kind for action in actions], ["show_overlay"])
        self.assertTrue(self.planner.overlay_visible)

    def test_policy_expiry_while_paused_restores_after_resume(self) -> None:
        self.planner.update(self.event(11, "firefox"), self.policy)
        self.planner.pause()
        self.planner.resume()
        # The overlay was already hidden by pause(), so only the restore is
        # left to do: the window must not stay hidden forever.
        self.assertEqual(
            [action.kind for action in self.planner.update(self.event(11, "firefox"), None)],
            ["restore_window"],
        )


class FocusReporterTest(unittest.TestCase):
    def setUp(self) -> None:
        self.now = datetime(2026, 11, 3, 12, 0, tzinfo=timezone.utc)
        self.reporter = FocusReporter()

    def event(self, *identifiers: str, normal: bool = True) -> FocusEvent:
        return FocusEvent(
            window=10,
            identifiers=frozenset(identifiers),
            label="Test",
            normal=normal,
        )

    def test_no_focus_reports_nothing(self) -> None:
        self.reporter.observe(None, self.now)
        self.assertEqual(self.reporter.aliases, ())
        self.assertIsNone(self.reporter.since)

    def test_focus_reports_sorted_aliases_and_start(self) -> None:
        self.reporter.observe(
            self.event("Navigator", "org.mozilla.firefox"), self.now
        )
        self.assertEqual(
            self.reporter.aliases, ("firefox", "navigator", "org.mozilla.firefox")
        )
        self.assertEqual(self.reporter.since, self.now)

    def test_same_focus_keeps_the_original_start(self) -> None:
        self.reporter.observe(self.event("firefox"), self.now)
        later = self.now + timedelta(minutes=5)
        self.reporter.observe(self.event("firefox"), later)
        self.assertEqual(self.reporter.since, self.now)

    def test_focus_change_restarts_the_interval(self) -> None:
        self.reporter.observe(self.event("firefox"), self.now)
        later = self.now + timedelta(minutes=5)
        self.reporter.observe(self.event("code"), later)
        self.assertEqual(self.reporter.aliases, ("code",))
        self.assertEqual(self.reporter.since, later)

    def test_blocked_or_non_application_focus_is_ignored(self) -> None:
        self.reporter.observe(self.event("firefox"), self.now)
        self.reporter.observe(self.event("firefox"), self.now, blocked=True)
        self.assertEqual(self.reporter.aliases, ())
        self.assertIsNone(self.reporter.since)

        self.reporter.observe(self.event("firefox"), self.now)
        self.reporter.observe(self.event("gnome-shell", normal=False), self.now)
        self.assertEqual(self.reporter.aliases, ())

    def test_empty_identifiers_reset_the_interval(self) -> None:
        self.reporter.observe(self.event("firefox"), self.now)
        self.reporter.observe(self.event(), self.now)
        self.assertEqual(self.reporter.aliases, ())
        self.assertIsNone(self.reporter.since)


if __name__ == "__main__":
    unittest.main()
