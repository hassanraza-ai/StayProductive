"""Focus: strict sessions with a real cool-down."""

from __future__ import annotations

from gi.repository import Gtk

from .. import widgets as w
from ..formatting import countdown, error_text, local_time, relative, remaining_seconds

DURATIONS = [(25, "25 minutes"), (45, "45 minutes"), (60, "1 hour"), (120, "2 hours"), (240, "4 hours")]
COOLDOWNS = [(5, "5 minutes"), (10, "10 minutes"), (15, "15 minutes"), (30, "30 minutes")]

RESTRICTION_TEXT = {
    "block_edits": "Rules cannot be edited",
    "block_pause": "Pausing is blocked",
    "block_settings": "Settings are locked",
    "password_gate": "Password gate required",
}


class FocusPage:
    def __init__(self, window) -> None:
        self.window = window
        self.root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=20)
        self.root.set_margin_top(28)
        self.root.set_margin_bottom(40)
        self.root.set_margin_start(28)
        self.root.set_margin_end(28)
        self._build()
        self.widget = w.page_scroller(self.root, max_width=900)

    def _build(self) -> None:
        copy = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        copy.append(w.label("Strict session", "page-title"))
        copy.append(
            w.label(
                "A strict session commits you to the rules that are active right now. "
                "Starting one records a local commitment; ending it early requires a "
                "cool-down. Nothing is uploaded and no account is involved.",
                "muted",
            )
        )
        self.root.append(copy)

        self.banner = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        self.banner.add_css_class("banner-warning")
        self.banner.set_margin_top(2)
        self.banner_label = w.label("", "muted small")
        self.banner.append(self.banner_label)
        self.root.append(self.banner)

        self.commitment_card = w.Card(hero=True, spacing=16)
        self.commitment_area = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16)
        self.commitment_area.append(self.commitment_card)
        self.root.append(self.commitment_area)

        self.form = w.Card(spacing=14)
        header = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        header.append(w.icon_badge("channel-secure-symbolic", 20))
        header_text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        header_text.append(w.label("Start a strict session", "card-title"))
        header_text.append(
            w.label(
                "The commitment locks matching rules. The cool-down is the documented, "
                "working way out; it is not a root-level lock.",
                "muted",
            )
        )
        header_text.set_hexpand(True)
        header.append(header_text)
        self.form.append(header)

        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        row.append(w.label("Duration", "row-title"))
        self.duration = Gtk.DropDown.new_from_strings([label for _, label in DURATIONS])
        self.duration.set_selected(1)
        row.append(self.duration)
        spacer = Gtk.Box()
        spacer.set_hexpand(True)
        row.append(spacer)
        row.append(w.label("Cool-down", "row-title"))
        self.cooldown = Gtk.DropDown.new_from_strings([label for _, label in COOLDOWNS])
        self.cooldown.set_selected(1)
        row.append(self.cooldown)
        self.form.append(row)

        restrictions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        for text in ("Lock rules", "Block pausing", "Cool-down to exit"):
            restrictions.append(w.Pill(text, "neutral"))
        self.form.append(restrictions)

        self.start_button = w.button(
            "Start strict session",
            "channel-secure-symbolic",
            css="suggested-action",
            on_click=self._start,
        )
        self.form.append(self.start_button)
        self.form_hint = w.label("", "muted small")
        self.form.append(self.form_hint)
        self.root.append(self.form)

    def refresh(self, snapshot: dict) -> None:
        commitment = snapshot.get("commitment")
        active = snapshot.get("active_profiles", [])
        tz = snapshot.get("settings", {}).get("timezone", "UTC")
        now = snapshot.get("now")

        if active:
            names = ", ".join(profile.get("name", "?") for profile in active)
            self.banner.set_visible(False)
            self.form_hint.set_label(f"Locks the active rules from: {names}.")
            self.start_button.set_sensitive(True)
        elif commitment is not None:
            # An existing commitment owns the screen; a profile may legitimately
            # have left its schedule since the session started.
            self.banner.set_visible(False)
        else:
            self.banner.set_visible(True)
            self.banner_label.set_label(
                "No profile is active. Activate or schedule a profile before starting a "
                "strict session, otherwise there are no rules to lock."
            )
            self.form_hint.set_label("Activate a profile first to enable strict sessions.")
            self.start_button.set_sensitive(False)

        if commitment is None:
            self.commitment_area.set_visible(False)
            self.form.set_visible(True)
            return

        self.commitment_area.set_visible(True)
        self.form.set_visible(False)
        w.clear(self.commitment_card)

        top = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        top.append(w.icon_badge("channel-secure-symbolic", 22))
        state = commitment.get("state", "active")
        top.append(
            w.status_pill(
                "Cooling down" if state == "cooling_down" else "Strict active",
                "warning" if state == "cooling_down" else "active",
            )
        )
        spacer = Gtk.Box()
        spacer.set_hexpand(True)
        top.append(spacer)
        ends = commitment.get("ends_at")
        if ends:
            top.append(w.Pill(f"Ends {local_time(ends, tz)}"))
        self.commitment_card.append(top)

        if state == "cooling_down":
            self.commitment_card.append(
                w.label("Cool-down in progress", "hero-title")
            )
            cooldown_end = commitment.get("cooldown_ends_at")
            seconds = remaining_seconds(cooldown_end, now)
            remaining_commitment = remaining_seconds(ends, now)
            if seconds > 0 and remaining_commitment > seconds:
                self.commitment_card.append(
                    w.label(
                        f"You can end the session in {countdown(seconds)}. Until then, "
                        "the commitment keeps enforcing the locked rules.",
                        "muted",
                    )
                )
                self.commitment_card.append(
                    w.button(
                        f"End session in {countdown(seconds)}",
                        "channel-secure-symbolic",
                        css="suggested-action",
                    )
                )
            else:
                self.commitment_card.append(
                    w.label(
                        "The cool-down has finished. You can end the session now.",
                        "muted",
                    )
                )
                self.commitment_card.append(
                    w.button(
                        "End strict session",
                        "channel-secure-symbolic",
                        css="suggested-action",
                        on_click=self._finish,
                    )
                )
        else:
            self.commitment_card.append(w.label("Strict session in progress", "hero-title"))
            self.commitment_card.append(
                w.label(
                    f"The session ends {relative(ends, now)} at {local_time(ends, tz)} "
                    f"({tz}). Requesting an early exit starts the cool-down.",
                    "muted",
                )
            )
            self.commitment_card.append(
                w.button(
                    "Request early exit",
                    "process-stop-symbolic",
                    css="destructive-action",
                    on_click=self._request_end,
                )
            )

        restrictions = commitment.get("restrictions", [])
        if restrictions:
            chips = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
            for restriction in restrictions:
                chips.append(
                    w.Pill(RESTRICTION_TEXT.get(restriction, restriction), "neutral")
                )
            self.commitment_card.append(chips)

    def _start(self) -> None:
        minutes = DURATIONS[self.duration.get_selected()][0]
        cooldown = COOLDOWNS[self.cooldown.get_selected()][0]
        try:
            self.window.engine.start_strict(minutes, cooldown)
            self.window.refresh()
            self.window.toast("Strict session started")
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not start strict session", error_text(error))

    def _request_end(self) -> None:
        commitment = self.window.snapshot.get("commitment") or {}
        commitment_id = commitment.get("id")
        if not commitment_id:
            return

        def request() -> None:
            try:
                self.window.engine.request_commitment_end(commitment_id)
                self.window.refresh()
                self.window.toast("Cool-down started")
            except Exception as error:  # noqa: BLE001
                self.window.show_error("Could not start cool-down", error_text(error))

        w.confirm(
            self.window,
            "Request an early exit?",
            "This starts the committed cool-down. The rules stay locked until it "
            "finishes, and the session cannot be ended before then.",
            "Start cool-down",
            request,
            destructive=True,
        )

    def _finish(self) -> None:
        commitment = self.window.snapshot.get("commitment") or {}
        commitment_id = commitment.get("id")
        if not commitment_id:
            return
        try:
            self.window.engine.finish_commitment_end(commitment_id)
            self.window.refresh()
            self.window.toast("Strict session ended")
        except Exception as error:  # noqa: BLE001
            self.window.show_error("Could not end session", error_text(error))
