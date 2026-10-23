# StayProductive

An open-source focus and self-control app for Linux, built to be **behaviourally
compatible** with Stay Focused 13 on Android: app and website blocking,
time limits, schedules, profiles, strict mode, content filtering, usage
statistics, and the companion tools around them — reimplemented natively for the
Linux desktop.

This repository contains the **specification and a runnable local application**.
The behavioural model, parity matrix, Linux architecture, and acceptance criteria
define what to build; `crates/core` implements the deterministic engine,
`crates/daemon` owns local state, and `crates/ui` is a GTK 4 + libadwaita desktop
front end. Everything is written clean-room from these documents; no code,
resources, or assets from the reference app are included or licensed.

## Status

| Phase | State |
| --- | --- |
| Behavioural analysis (static) | complete, see `docs/parity/` |
| Product specification | complete for P0/P1, see `docs/spec/` |
| Linux architecture | decided, see `docs/architecture/` |
| Core engine | **implemented** — profiles, rules, limits, schedules, commitments, 32 acceptance scenarios |
| Local state service | **implemented** — versioned atomic JSON, profiles, quick blocks, strict cool-down, settings |
| Desktop UI | **implemented** — dashboard, profiles and editor, schedules, strict session, activity, settings |
| Desktop enforcement | **implemented for GNOME app windows** — opt-in Shell extension; websites, limits, and other compositors remain |
| Runtime parity verification | impossible against the reference without a device; parity is defined by the acceptance suite |

The current build is useful as a real, persistent focus planner: profiles,
schedules, limits, strict commitments, and state survive restarts. On GNOME it
can also block application windows once the bundled Shell extension is installed
and enabled. Website, keyword, usage-limit, and non-GNOME enforcement are still
missing; the dashboard and Settings page report the adapter state rather than
pretending.

### Implemented today

- Rust core with a pure `(state, event, now)` decision function.
- Local state service over newline-delimited JSON on stdio; the UI never writes
  the state file directly.
- Versioned state at
  `$XDG_DATA_HOME/stayproductive/state.json` (fallback
  `~/.local/share/stayproductive/state.json`), written atomically with mode
  `0600`, parent directory `0700`.
- GTK 4 + libadwaita UI: dashboard, profile CRUD and editor (apps, sites,
  keywords, allowlists, limits, pickup limits, six schedule kinds), schedules,
  quick blocks, strict sessions with a real cool-down, activity, and settings.
- Timezone defaults to the system zone through `iana-time-zone`; the start of day
  and timezone are editable.
- Strict commitments block profile edits and pauses in both the UI and the
  daemon, and survive restart.
- GNOME Shell integration: a short-lived `enforcement.json` policy, an opt-in
  extension that overlays blocked application windows on Wayland and XWayland,
  a runtime heartbeat, and an install/status flow in Settings.
- The dashboard only says enforcement is active when the extension heartbeat is
  fresh and reports a loaded policy; otherwise it explains what is missing.

### Not implemented yet

Website and keyword enforcement, the browser extension, usage-limit tracking,
launch/pickup limits, non-GNOME compositor adapters, tray integration, DNS
helper, SQLite history/migrations, usage statistics, onboarding, low-time
notifications, backup/restore, full translation catalogs, and a complete
screen-reader audit. These remain tracked as `todo`/`wip`/`gap` in
[`docs/parity/feature-matrix.md`](docs/parity/feature-matrix.md).

## Run the desktop app

Prerequisites: Rust stable, Python 3.10+, PyGObject, GTK 4, and libadwaita 1.

```sh
./stayproductive
```

The launcher builds the release state service if needed and starts the GTK
window. To use a different state file (for testing or a portable profile):

```sh
STAYPRODUCTIVE_STATE=/tmp/stayproductive-test.json ./stayproductive
```

The state service is a child of the UI. Closing the window shuts it down; the
state is already persisted. There is no account, network call, or telemetry.

On distributions where the Python GTK bindings are packaged separately, install
`python3-gi`, `gir1.2-gtk-4.0`, and `gir1.2-adw-1` first. The app deliberately
does not use `pip` for PyGObject.

### Enable desktop blocking on GNOME

1. Open **Settings → Desktop integration** and choose **Install**.
2. Log out and back in once. GNOME Shell only loads a newly installed extension
   after a Shell restart.
3. Open StayProductive again. The sidebar and Settings page should show
   **Enforcement active**, and blocking an application in an active profile
   should cover its window.

The extension is optional and local. It enforces application `block` rules and
application quick blocks only; sites, keywords, and usage limits still need the
browser extension and tracker. Closing StayProductive invalidates the policy
within 30 seconds. See [`integrations/gnome/README.md`](integrations/gnome/README.md)
for the policy format, emergency unblock, privacy notes, and uninstall steps.

### Verify the engine

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The acceptance suite is `crates/core/tests/acceptance.rs`; every test names the
`A-*` identifier from `docs/acceptance/parity-tests.md`. The end-to-end service
test is `crates/daemon/tests/protocol.rs`; it drives the real daemon binary over
stdio and checks strict locks, the cool-down refusal, restart persistence,
state-file permissions, the generated enforcement policy, and the extension
heartbeat gate. The GNOME matching helpers have a compositor-free test:

```sh
gjs -m integrations/gnome/tests/matching.test.js
```

## What "identical behaviour" means here

The goal is feature and behaviour parity as a user experiences it, not
source-level similarity:

- the same **decisions** — what is blocked, when, for how long, and why;
- the same **flows** — onboarding, profile creation, schedule editing, breaks,
  strict-mode setup, deactivation;
- the same **edge cases** — pause/override, cooldowns, grace periods, clock
  changes, app restarts, multiple monitors, screen lock;
- the same **explanations** — the block screen says why and until when.

Android-only mechanisms are mapped to honest Linux equivalents; where no safe
equivalent exists, the spec says so instead of pretending. See
[`docs/architecture/linux-design.md`](docs/architecture/linux-design.md).

## Feature pillars

1. **Blocking engine** — per-app and per-site rules, allowlists, keywords, and
   in-app content surfaces (Reels, Shorts, Stories, …).
2. **Limits and schedules** — daily/hourly usage limits, launch limits, time
   windows, day-of-week schedules, start-of-day offset.
3. **Profiles** — named bundles of rules and schedules, activatable manually or
   by condition, with templates.
4. **Strict mode and lock mode** — commitments: delayed deactivation, QR or
   text challenges, friend approval, password-gated settings.
5. **Awareness** — usage stats, top distractions, pickups, weekly reports,
   low-time alerts.
6. **Focus tools** — Pomodoro sessions, break timers, ambient sounds, quick
   block.
7. **Desktop integration** — tray, notifications, GNOME/KDE/sway/Hyprland
   adapters, browser extension, optional system DNS filtering.

## Documentation map

| Document | Purpose |
| --- | --- |
| [`AGENTS.md`](AGENTS.md) | how an agent works in this repo; rules and definition of done |
| [`docs/parity/app-analysis.md`](docs/parity/app-analysis.md) | behaviour-focused analysis of the reference app |
| [`docs/parity/feature-inventory.md`](docs/parity/feature-inventory.md) | subsystem inventory with confidence labels |
| [`docs/parity/feature-matrix.md`](docs/parity/feature-matrix.md) | every feature → priority → Linux mechanism → acceptance ID |
| [`docs/spec/domain-model.md`](docs/spec/domain-model.md) | entities, fields, invariants, storage |
| [`docs/spec/blocking-engine.md`](docs/spec/blocking-engine.md) | detection, decision precedence, enforcement, timers |
| [`docs/spec/schedules-and-conditions.md`](docs/spec/schedules-and-conditions.md) | schedules, activation conditions, pause/extend/break |
| [`docs/spec/strict-and-lock.md`](docs/spec/strict-and-lock.md) | commitments, deactivation methods, lock mode |
| [`docs/spec/content-filtering.md`](docs/spec/content-filtering.md) | sites, keywords, in-app surfaces, adult filtering |
| [`docs/spec/ux-flows.md`](docs/spec/ux-flows.md) | screens and state transitions |
| [`docs/architecture/linux-design.md`](docs/architecture/linux-design.md) | components, stack, desktop support, threat model |
| [`docs/acceptance/parity-tests.md`](docs/acceptance/parity-tests.md) | Given/When/Then acceptance suite |
| [`docs/legal/clean-room.md`](docs/legal/clean-room.md) | IP rules for contributors |

## Implementation stack

- **Core**: Rust, no runtime, no desktop dependency.
- **Local state service**: Rust binary that owns `State` and persists versioned
  JSON atomically. The current boundary is newline-delimited JSON over stdio;
  the D-Bus surface in the architecture document replaces it without moving
  decision logic out of `core`.
- **Desktop UI**: GTK 4 + libadwaita through PyGObject. The Python layer only
  renders state and sends commands; it contains no blocking decisions.
- **Desktop integration**: an opt-in GNOME Shell extension for application
  windows, driven by a short-lived policy file and a runtime heartbeat. Further
  adapters (X11 EWMH, sway/i3 IPC, Hyprland IPC, KWin script), the browser
  extension (Manifest V3), and the optional privileged DNS helper are planned.
- **Planned packaging**: Flatpak (user mode, no system-wide blocking), `.deb`,
  `.rpm`, Arch package, and a systemd user unit.

The architecture document still specifies SQLite, D-Bus, and gtk-rs as the
target M1 stack. The runnable build uses JSON and PyGObject as a deliberate,
documented first step; both are behind the same daemon boundary and can be
replaced without changing the core engine.

All features are free. There is no paywall, account requirement, telemetry, or
cloud dependency; sync and accountability are optional and self-hostable.

## Reference material

The analysis was performed on a locally held APK that is **not** part of this
repository. Heavy artifacts (decompiled sources, the decoded project, native
libraries) stay outside git; `scripts/link-reference.sh` links them into
`reference/` for local use. See [`reference/README.md`](reference/README.md).

## Contributing

Read [`AGENTS.md`](AGENTS.md) and [`docs/legal/clean-room.md`](docs/legal/clean-room.md)
first. Every behaviour change must update the parity matrix and the acceptance
suite.

## License

Apache-2.0. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).

StayProductive is not affiliated with, endorsed by, or derived from the source
code of any commercial product. Product names mentioned in analysis documents
are trademarks of their owners and are used only to describe compatibility
targets.
