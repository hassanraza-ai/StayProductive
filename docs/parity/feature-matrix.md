# Parity matrix

Single source of truth for **what must exist** and **how it is proven**. Every
row has an ID used by specs, tests, commits, and PRs.

Priorities: **P0** = required for first usable release; **P1** = required for
behavioural parity; **P2** = platform extra / honest gap.

Status values: `todo`, `wip`, `done`, `gap` (documented, deliberately not
implemented), `decision` (blocked on a product/architecture decision).

`wip` means some implementation exists (core, daemon, or GTK UI) but at least
one required layer or automated acceptance evidence is still outstanding. See
[`../spec/implementation-notes.md`](../spec/implementation-notes.md).

The runnable build (November 2026) has the core engine, an autonomous local JSON
state service behind a same-user Unix socket and an opt-in systemd user unit, a
GTK 4 + libadwaita UI, and a five-step first-run onboarding wizard. A supervised
X11 adapter enforces application `block` rules, application quick blocks, and
daily/hourly application usage limits on real windows; an opt-in GNOME Shell
extension does the same for native Wayland and XWayland. The observer reports
the focused app, the daemon attributes foreground intervals to the engine's
usage ledger, and a reached limit produces a policy entry like any other block.
It does **not** yet enforce websites, keywords, site time limits, launch/pickup
limits, or other compositors. Every row whose user-visible result depends on
those missing layers remains `wip` or `todo`. The app reports enforcement as
active only when an adapter heartbeat is fresh and has loaded a valid policy.

## 1. Profiles and rules

| ID | Behaviour | P | Linux mechanism | Spec | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F-PROF-01 | Create, rename, duplicate, delete profiles | P0 | core + SQLite | domain §3 | A-PROF-01 | wip |
| F-PROF-02 | Activate a profile manually; deactivate it | P0 | D-Bus + UI | sched §6 | A-PROF-02 | wip |
| F-PROF-03 | Only one manually activated profile at a time | P0 | engine invariant | domain §4 | A-PROF-03 | wip |
| F-PROF-04 | Profile templates (Focus, Sleep, Detox, Exam, …) | P1 | bundled presets | sched §7 | A-PROF-04 | wip |
| F-PROF-05 | Copy items from an existing profile | P1 | UI | ux §4 | A-PROF-05 | wip |
| F-PROF-06 | Mark profile as strict-mode eligible | P1 | engine | strict §3 | A-PROF-06 | wip |
| F-RULE-01 | Block selected apps | P0 | compositor adapter | block §5 | A-RULE-01 | wip |
| F-RULE-02 | Block selected websites | P0 | DNS + extension | content §2 | A-RULE-02 | wip |
| F-RULE-03 | Block selected keywords | P1 | extension + URL pipe | content §3 | A-RULE-03 | wip |
| F-RULE-04 | Allowlist apps (override non-strict rules) | P0 | engine | block §4 | A-RULE-04 | wip |
| F-RULE-05 | Allowlist websites | P1 | engine + resolver | content §2 | A-RULE-05 | wip |
| F-RULE-06 | Goal-based unlock (spend time to earn access) | P1 | engine counters | block §6 | A-RULE-06 | todo |
| F-RULE-07 | Wait timer after usage | P1 | engine timer | block §6 | A-RULE-07 | wip |
| F-RULE-08 | Quick block for N minutes | P1 | UI + engine | block §6 | A-RULE-08 | wip |

## 2. Limits and counters

| ID | Behaviour | P | Linux mechanism | Spec | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F-LIM-01 | Daily usage limit per app | P0 | event log + counters | block §4 | A-LIM-01 | wip |
| F-LIM-02 | Hourly usage limit per app | P0 | event log + counters | block §4 | A-LIM-02 | wip |
| F-LIM-03 | Daily/hourly launch limits | P1 | tracker | block §4 | A-LIM-03 | wip |
| F-LIM-04 | Daily/hourly pickup (unlock) limits | P1 | logind | block §4 | A-LIM-04 | wip |
| F-LIM-05 | Limits survive restart and crash | P0 | persisted events | block §7 | A-LIM-05 | wip |
| F-LIM-06 | Counter day boundary honours start-of-day | P0 | period calculator | sched §2 | A-LIM-06 | wip |
| F-LIM-07 | Low-time alerts at thresholds | P1 | notifications | block §8 | A-LIM-07 | todo |

## 3. Schedules and activation

| ID | Behaviour | P | Linux mechanism | Spec | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F-SCH-01 | Daily time window | P0 | scheduler | sched §3 | A-SCH-01 | wip |
| F-SCH-02 | Day-of-week windows (per-day) | P0 | scheduler | sched §3 | A-SCH-02 | wip |
| F-SCH-03 | Multiple intervals per day | P1 | scheduler | sched §3 | A-SCH-03 | wip |
| F-SCH-04 | Full-day and one-time/custom windows | P1 | scheduler | sched §3 | A-SCH-04 | wip |
| F-SCH-05 | Hourly pattern | P1 | scheduler | sched §3 | A-SCH-05 | wip |
| F-SCH-06 | Start-of-day offset | P0 | period calculator | sched §2 | A-SCH-06 | wip |
| F-SCH-07 | Pause profile for 1/5/10/15/30 min, 1 h, today | P0 | engine state | sched §5 | A-SCH-07 | wip |
| F-SCH-08 | Indefinite pause (no duration) | P1 | engine state | sched §5 | A-SCH-08 | wip |
| F-SCH-09 | Extend an active block | P1 | engine state | sched §5 | A-SCH-09 | wip |
| F-SCH-10 | Schedule change detection (no restart needed) | P0 | scheduler invalidation | sched §4 | A-SCH-10 | wip |
| F-SCH-11 | Location activation | P2 | GeoClue2 | sched §6 | A-SCH-11 | decision |

## 4. Strict mode and lock mode

| ID | Behaviour | P | Linux mechanism | Spec | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F-STR-01 | Activate strict mode until an expiry time | P0 | commitment state | strict §4 | A-STR-01 | wip |
| F-STR-02 | Activate strict mode for a schedule | P1 | commitment state | strict §4 | A-STR-02 | todo |
| F-STR-03 | Cool-down deactivation with visible timer | P0 | timer + state | strict §5 | A-STR-03 | wip |
| F-STR-04 | Random-text challenge | P1 | challenge | strict §5 | A-STR-04 | todo |
| F-STR-05 | QR challenge using a saved code | P1 | challenge | strict §5 | A-STR-05 | todo |
| F-STR-06 | Friend approval (optional, self-hosted) | P2 | HTTP + state | strict §5 | A-STR-06 | decision |
| F-STR-07 | Password gates app/settings/edits | P1 | argon2id + policy | strict §6 | A-STR-07 | todo |
| F-STR-08 | Block phone-settings access while locked | P1 | portal/overlay | strict §6 | A-STR-08 | todo |
| F-STR-09 | Block restriction edits while locked | P0 | policy | strict §6 | A-STR-09 | wip |
| F-STR-10 | Lock mode with normal-mode exit | P1 | state machine | strict §6 | A-STR-10 | todo |
| F-STR-11 | Survive restart; commitment cannot be silently cleared | P0 | atomic store | strict §7 | A-STR-11 | wip |
| F-STR-12 | Uninstall/settings hardening | — | no safe equivalent | strict §8 | — | gap |
| F-STR-13 | Strict alarm with challenge | P2 | timer + challenge | strict §9 | A-STR-13 | todo |

## 5. Content filtering

| ID | Behaviour | P | Linux mechanism | Spec | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F-CON-01 | Detect and block short-video surfaces in web clients | P1 | extension DOM rules | content §4 | A-CON-01 | todo |
| F-CON-02 | Detect and block Stories/Spotlight/Reels surfaces | P1 | extension | content §4 | A-CON-02 | todo |
| F-CON-03 | Per-surface toggles (Shorts, Reels, Stories, …) | P1 | settings | content §4 | A-CON-03 | todo |
| F-CON-04 | Adult-domain filtering | P1 | DNS lists | content §5 | A-CON-04 | todo |
| F-CON-05 | Block unsupported browsers (policy) | P2 | engine | content §6 | A-CON-05 | todo |
| F-CON-06 | Keyword matching is case-insensitive | P0 | engine | content §3 | A-CON-06 | wip |
| F-CON-07 | Native mobile surfaces | — | not reproducible | content §7 | — | gap |
| F-CON-08 | In-app browser webviews | — | not inspectable | content §7 | — | gap |

## 6. Awareness and statistics

| ID | Behaviour | P | Linux mechanism | Spec | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F-STA-01 | Record foreground intervals per app/site | P0 | tracker + SQLite | block §7 | A-STA-01 | wip |
| F-STA-02 | Today / week / month totals | P1 | queries | ux §7 | A-STA-02 | todo |
| F-STA-03 | Top distractions ranking | P1 | queries | ux §7 | A-STA-03 | todo |
| F-STA-04 | Usage timeline | P2 | queries + chart | ux §7 | A-STA-04 | todo |
| F-STA-05 | Pickup and launch counts | P1 | logind/tracker | ux §7 | A-STA-05 | todo |
| F-STA-06 | Seven-day report and comparison | P2 | report view | ux §7 | A-STA-06 | todo |
| F-STA-07 | Block/deny history with reasons | P1 | event log | block §7 | A-STA-07 | todo |
| F-STA-08 | Export and delete usage data | P0 | store | ux §8 | A-STA-08 | done |

## 7. UX, onboarding, settings

| ID | Behaviour | P | Linux mechanism | Spec | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F-UX-01 | Onboarding: purpose, permissions, first profile | P0 | GTK wizard | ux §2 | A-UX-01 | wip |
| F-UX-02 | Dashboard: status, profile, remaining time, quick actions | P0 | GTK | ux §3 | A-UX-02 | wip |
| F-UX-03 | Block screen explains reason and end time | P0 | GNOME overlay | ux §5 | A-UX-03 | wip |
| F-UX-04 | Break countdown screen | P1 | overlay | ux §5 | A-UX-04 | todo |
| F-UX-05 | Profile editor with apps, sites, keywords, limits | P0 | GTK | ux §4 | A-UX-05 | wip |
| F-UX-06 | Schedule editor per schedule type | P0 | GTK | ux §4 | A-UX-06 | wip |
| F-UX-07 | Strict-mode setup and deactivation flows | P1 | GTK | ux §6 | A-UX-07 | wip |
| F-UX-08 | Settings: start of day, notifications, autostart, theme | P1 | GTK | ux §8 | A-UX-08 | wip |
| F-UX-09 | Tray icon with pause/resume/quick block | P1 | StatusNotifier | ux §9 | A-UX-09 | todo |
| F-UX-10 | Keyboard-first navigation and screen-reader labels | P0 | GTK/a11y | ux §10 | A-UX-10 | todo |
| F-UX-11 | Localisation-ready strings | P1 | gettext | ux §10 | A-UX-11 | todo |
| F-UX-12 | Backup/restore of all state | P1 | JSON archive | ux §8 | A-UX-12 | wip |

## 8. Platform integration

| ID | Behaviour | P | Linux mechanism | Spec | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F-PLAT-01 | Daemon autostart and restart | P0 | systemd user unit | arch §5 | A-PLAT-01 | wip |
| F-PLAT-02 | X11 foreground detection and enforcement | P0 | EWMH | arch §4 | A-PLAT-02 | wip |
| F-PLAT-03 | sway/i3 adapter | P1 | IPC | arch §4 | A-PLAT-03 | todo |
| F-PLAT-04 | Hyprland adapter | P1 | IPC | arch §4 | A-PLAT-04 | todo |
| F-PLAT-05 | GNOME Shell adapter | P2 | extension + policy file | arch §4 | A-PLAT-05 | wip |
| F-PLAT-06 | KDE/KWin adapter | P2 | script + D-Bus | arch §4 | A-PLAT-06 | todo |
| F-PLAT-07 | System-wide DNS filtering (optional helper) | P1 | polkit + resolver | arch §7 | A-PLAT-07 | decision |
| F-PLAT-08 | Browser extension (URL + DOM signals) | P1 | WebExtension | content §2 | A-PLAT-08 | todo |
| F-PLAT-09 | Idle/lock/suspend awareness | P0 | logind | arch §4 | A-PLAT-09 | wip |
| F-PLAT-10 | Multi-monitor lock overlay | P0 | compositor adapter | arch §6 | A-PLAT-10 | wip |
| F-PLAT-11 | Notifications and low-time alerts | P1 | D-Bus | arch §5 | A-PLAT-11 | todo |

## 9. Data and lifecycle

| ID | Behaviour | P | Linux mechanism | Spec | Test | Status |
| --- | --- | --- | --- | --- | --- | --- |
| F-DATA-01 | Atomic migrations; never lose rules/history | P0 | SQLite | domain §6 | A-DATA-01 | todo |
| F-DATA-02 | Clock change and DST handled without double counting | P0 | monotonic + wall clock | block §7 | A-DATA-02 | wip |
| F-DATA-03 | Daemon/UI version skew tolerated | P1 | IPC versioning | arch §8 | A-DATA-03 | todo |
| F-DATA-04 | Full local reset with confirmation | P1 | store | ux §8 | A-DATA-04 | todo |

## Current build evidence (November 2026)

| Check | Result |
| --- | --- |
| `cargo fmt --check` | pass |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `cargo test` | pass, including 32 acceptance tests and the daemon protocol tests |
| `cargo build --release -p stayproductive-daemon` | pass |
| GTK app launch | pass on X11/XWayland with GTK 4 and libadwaita |
| Dashboard, profiles, schedules, strict, activity, settings | rendered from live daemon state and manually inspected |
| Profile editor round-trip | created and updated a scheduled profile with apps, site daily limit, keywords, allowlist, and multiple windows; persisted state re-read correctly |
| Strict cool-down | core test rejects `finish_commitment_end` before the cool-down and accepts it afterwards; locked edits/pauses are rejected |
| Enforcement policy | daemon protocol test checks active/inactive app entries, policy path, and `0600` mode |
| Enforcement heartbeat | daemon protocol test checks active only with a fresh `active:true`/`policy_loaded:true` heartbeat with the expected `adapter`, and refuses stale, inactive, policy-less, or foreign files |
| Usage tracking → limits | daemon protocol tests drive the real binary: reported focus over a daily limit blocks with `R-LIMIT-DAILY` and records history; the block overlay suspends accrual; counters survive restart (A-LIM-05 seed); the same works from either the GNOME extension or the X11 adapter heartbeat, including the daemon's own five-second pulse with no client attached |
| Export and delete (`F-STA-08`) | `export_and_delete_usage_data` in `crates/daemon/tests/protocol.rs` exports a versioned zip, parses its central directory (offsets and CRC), checks the manifest version, profiles, settings, and monthly usage entries, refuses an unconfirmed delete, and proves the deletion survives restart; the archive also opens cleanly in Python's `zipfile`; `test_engine.py` checks the client's `include_usage`/`path`/`confirm` intent; the Activity page shows both actions with a destructive confirmation |
| Restore (`F-UX-12`) | `a_clean_install_restores_an_export` exports from one daemon and imports into a second clean one, checking the restored profile, clock settings, and active strict session; a non-archive is refused with no state change; the restore survives restart. Unit tests refuse a newer manifest version and keep the longer active commitment when merging into a used install. Usage counters are **not** re-imported, so the row stays `wip` |
| X11 adapter matching (`F-PLAT-02`) | `crates/ui/tests/test_x11_policy.py` covers aliases, expiry, hide-then-overlay ordering, dedupe, re-block, allowed-window hide, and Back |
| X11 adapter supervision (`F-PLAT-02`) | `crates/daemon/tests/protocol.rs` kills a fake adapter and checks that the daemon removes its stale heartbeat, restarts it, and reports `active`/`x11`; shutdown leaves no orphan |
| X11 adapter live (`F-PLAT-02`) | opt-in `crates/ui/tests/test_x11_live.py` under Xvfb: blocked window unmapped, allowed window lowers the block, refocusing re-blocks, expiring policy restores the window, and the heartbeat carries `monitors`, `card_monitor`, `locked`, `suspended`, and `session_aware` |
| X11 adapter end-to-end (`F-PLAT-02`) | real daemon + real adapter under Xvfb: active profile produced the policy, the focused window was blocked, snapshot reported `active`/`x11` |
| X11 card placement (`F-PLAT-10`) | `PreferredMonitorTest` in `crates/ui/tests/test_x11_policy.py` covers single monitor, left/right monitor, a spanning window's majority, off-screen and missing geometry, and no monitors; live Xvfb run asserts one monitor and card on monitor 0 |
| X11 lock/suspend (`F-PLAT-09`) | `PlannerPauseTest` covers pause hiding the overlay, focus/Back ignored while paused, resume re-blocking the still-hidden window, and restoring a policy that expired during the pause; `SessionMonitor` reads logind `Lock`/`Unlock`/`PrepareForSleep` plus a throttled `LockedHint` fallback and reports `session_aware` |
| X11 multi-monitor visuals (`F-PLAT-10`) | **not measured**: Xvfb rejects RandR monitor objects, so GDK always sees one monitor; card position and focus on a real two-monitor X11 session still need a logged-in desktop |
| X11 real lock/suspend (`F-PLAT-09`) | **not measured** by locking a live session; logind wiring is exercised against the system bus in the heartbeat, and the release/resume handshake is unit-tested |
| X11 WM iconification and overlay visuals | **not measured**: Xvfb has no window manager and no logged-in X11 session was available |
| Socket transport (`F-PLAT-01`) | daemon protocol tests check `0600` socket / `0700` dir, single instance, clean socket removal, and a policy refreshed with no client |
| Daemon autonomy (`F-PLAT-01`) | Xvfb end-to-end: `SIGKILL`ed the UI, daemon stayed alive and advanced the policy timestamp; relaunch reused the same daemon PID; explicit shutdown removed process and socket |
| systemd unit and restart (`F-PLAT-01`) | `crates/ui/tests/test_service.py` covers unit text, quoting, socket naming, service states, and handover order; manual transient unit restarted a `kill -9`ed daemon with a new PID and stayed active |
| Autostart toggle | **not measured** with a real `systemctl --user enable` + logout/login; requires a human session |
| GNOME matching helpers | `gjs -m integrations/gnome/tests/matching.test.js` pass, including the `FocusTracker` interval rules |
| Installed-app chooser | `PYTHONPATH=crates/ui python3 -m unittest discover -s crates/ui/tests` pass; GTK construction smoke-tested under Xvfb |
| First-run onboarding | five steps implemented; goal/app selection, starter profile, marker, and the “exactly one profile” guard covered by `crates/ui/tests/test_onboarding.py`; GTK flow construction-smoke-tested under Xvfb. Autostart, notifications, and the DNS helper are listed as unavailable and cannot be enabled from the wizard. |
| GNOME install flow | exercised with an isolated `XDG_DATA_HOME` and keyfile settings backend; copied the extension and enabled it without touching the host session |
| GNOME extension load | pass in headless GNOME Shell 49: loads, reads policy, writes heartbeat, drops expired/removed policy |
| GNOME overlay focus/visuals | **not measured** in headless Shell because it has no seat; requires a logged-in session |

Manual UI inspection is not a substitute for the acceptance suite. Rows that
depend on the UI remain `wip` until their `A-UX-*` scenarios are automated.

| Milestone | Contents | Exit criterion |
| --- | --- | --- |
| **M0 — engine** | core model, scheduler, counters, decision function, SQLite, fake clock | all `block`/`sched` unit scenarios green |
| **M1 — usable** | daemon, X11 adapter, lock overlay, GTK profiles/limits, tray | P0 rows done; blocks a real app on X11 |
| **M2 — parity** | strict/lock, keywords, extension, DNS helper, stats, onboarding | P1 rows done or `gap` |
| **M3 — desktop breadth** | sway/Hyprland, GNOME/KDE adapters, packaging | adapter conformance suite green |
| **M4 — extras** | widgets/tray stats, Pomodoro, sounds, location, reports | P2 rows done or `gap` |

## Parity definition

Parity is **not** "looks similar". A row is `done` only when:

1. its acceptance scenarios pass, including the listed edge cases;
2. the block explanation is visible to the user;
3. the behaviour survives daemon restart and screen lock;
4. the matrix links to a passing test by ID.

## Open decisions

| ID | Question | Blocking | Owner decision needed |
| --- | --- | --- | --- |
| D-1 | Is system-wide DNS filtering in the default package, or a separate opt-in helper? | F-PLAT-07 | packaging/security |
| D-2 | Which compositors are tier-1 for the first release? | F-PLAT-02..06 | product |
| D-3 | Is friend approval self-hosted only, or also peer-to-peer? | F-STR-06 | product/privacy |
| D-4 | QR challenge camera stack (portal vs pipewire vs phone companion) | F-STR-05 | platform |
