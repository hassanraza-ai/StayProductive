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
| Desktop enforcement | **implemented for GNOME app windows** — opt-in Shell extension covering app blocks, quick blocks, and daily/hourly app usage limits; sites, keywords, and other compositors remain |
| Runtime parity verification | impossible against the reference without a device; parity is defined by the acceptance suite |

The current build is useful as a real, persistent focus planner: profiles,
schedules, limits, strict commitments, and state survive restarts. On GNOME it
can also block application windows and enforce daily/hourly application usage
limits once the bundled Shell extension is installed and enabled — the
extension reports the focused app, the daemon accumulates foreground time in
the engine's usage ledger, and a reached limit blocks like any other rule.
Website, keyword, and non-GNOME enforcement are still missing; the dashboard
and Settings page report the adapter state rather than pretending.

### Implemented today

- Rust core with a pure `(state, event, now)` decision function.
- Local state service over newline-delimited JSON on a same-user Unix socket at
  `$XDG_RUNTIME_DIR/stayproductive/daemon.sock` (stdio stays for tests); the UI
  never writes the state file directly.
- The daemon is independent of the window: closing the UI keeps schedules,
  enforcement policy refresh, and blocks running. A systemd user unit
  (`stayproductive.service`, `Restart=always`) restarts a killed daemon, and
  Settings offers opt-in autostart. The unit is written per user; nothing is
  enabled without an explicit action.
- Versioned state at
  `$XDG_DATA_HOME/stayproductive/state.json` (fallback
  `~/.local/share/stayproductive/state.json`), written atomically with mode
  `0600`, parent directory `0700`.
- GTK 4 + libadwaita UI: dashboard, profile CRUD and editor (installed-app
  chooser by display name, sites, keywords, allowlists, limits, pickup limits,
  six schedule kinds), schedules, quick blocks, strict sessions with a real
  cool-down, activity, and settings.
- Timezone defaults to the system zone through `iana-time-zone`; the start of day
  and timezone are editable.
- Strict commitments block profile edits and pauses in both the UI and the
  daemon, and survive restart.
- Foreground usage tracking for apps with rules: either adapter heartbeat (the
  GNOME extension or the X11 adapter) names the focused app, the daemon
  attributes intervals to the core usage ledger, and daily/hourly app limits
  enforce for real, even with the GTK window closed. Usage survives restart,
  stops behind the block overlay or a locked session, and is shown per app on
  the Activity page.
- Export and delete usage data from the Activity page. An export is a versioned
  zip of profiles, settings, commitments, and monthly usage, written with mode
  `0600`; deleting history asks first, removes intervals, launches, pickups, and
  block decisions, and keeps profiles plus any active strict session. Settings →
  Backup restores an export: a clean install is replaced, a used install is
  merged without shortening an active strict session, and usage counters are
  left in the archive rather than re-imported.
- GNOME Shell integration: a short-lived `enforcement.json` policy, an opt-in
  extension that overlays blocked application windows on Wayland and XWayland,
  a runtime heartbeat, and an install/status flow in Settings.
- The dashboard only says enforcement is active when the extension heartbeat is
  fresh and reports a loaded policy; otherwise it explains what is missing.
- A five-step first-run wizard: purpose, a goal (Social, Work, Study, Sleep,
  Custom), an honest capability list, one editable starter profile, and a done
  screen. It appears only on a clean state, creates exactly one profile, and can
  be re-run from Settings → Help.
- Blocking warnings are dismissible per session so a known gap does not shout on
  every refresh — the underlying state is unchanged and reappears after a
  restart or once enforcement recovers and drops again.

### Not implemented yet

Website and keyword enforcement, the browser extension, site time limits,
launch/pickup limits, non-GNOME compositor adapters, tray integration, DNS
helper, SQLite history/migrations, usage statistics beyond today's totals,
low-time notifications, restoring usage counters from a backup, full
translation catalogs, and a complete screen-reader audit. Notifications and the
DNS helper are listed in
onboarding but deliberately unavailable, so nothing is enabled by clicking
through the wizard. Autostart is implemented through the systemd user unit, but
only for the default data file with a running systemd user manager. These
remain tracked as `todo`/`wip`/`gap`
in [`docs/parity/feature-matrix.md`](docs/parity/feature-matrix.md).

## Run the desktop app

Prerequisites: Rust stable, Python 3.10+, PyGObject, GTK 4, and libadwaita 1.

```sh
./stayproductive
```

The launcher builds the release state service if needed, starts or connects to
the background daemon, then opens the GTK window. To use a different state file
(for testing or a portable profile):

```sh
STAYPRODUCTIVE_STATE=/tmp/stayproductive-test.json ./stayproductive
```

Closing the window no longer stops blocking: the daemon stays connected to the
same socket below `$XDG_RUNTIME_DIR/stayproductive/`. Relaunching reuses it. To
stop it explicitly, use **Settings → Background service** or:

```sh
systemctl --user stop stayproductive.service
```

If you started it without systemd, send `shutdown` over the socket or kill the
`stayproductive-daemon --socket` process; it removes its socket on exit. The
state is persisted either way. There is no account, network call, or telemetry.

Custom state files get a hashed socket name so a test instance never talks to the
login daemon. `STAYPRODUCTIVE_SOCKET` overrides the socket path entirely.

Autostart is opt-in: turn on **Settings → Background service → Start at login**,
or choose **Set up** in the onboarding capability step. It requires a running
systemd user manager and the default data file, and reports why it is
unavailable otherwise.

On a clean state the first-run wizard opens automatically and offers a goal-based
starter profile. Completing it writes `onboarding.json` next to `state.json`; to
see the wizard again, either run it from **Settings → Help** or delete that
marker. Re-running never resets profiles or commitments.

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

The extension is optional and local. It enforces application `block` rules,
application quick blocks, and daily/hourly application usage limits (by
reporting the focused app so the daemon can attribute foreground time); sites,
keywords, launch and pickup limits still need the browser extension and a
logind hookup. The daemon keeps refreshing the policy while it runs, including
when the window is closed, so enforcement continues until the daemon is stopped.
See [`integrations/gnome/README.md`](integrations/gnome/README.md)
for the policy format, usage tracking, emergency unblock, privacy notes, and
uninstall steps.

### Enable desktop blocking on X11

On a plain X11 session there is nothing to install. When the environment has
`DISPLAY`, no `WAYLAND_DISPLAY`, and `XDG_SESSION_TYPE` is not `wayland`, the
daemon starts `stayproductive-x11` itself and restarts it if it exits. The
adapter watches EWMH focus, hides a blocked window, and covers every monitor
with the block explanation. The explanation card is shown on the monitor that
holds the blocked window, and keyboard focus follows it there. When
systemd-logind reports a screen lock or suspend, the overlay is released so it
cannot cover a lock screen or security prompt; the blocked window stays hidden
and the block resumes on unlock. It is a separate process so closing the
StayProductive window does not stop enforcement.

If the daemon cannot find the launcher, point it at the checkout script
explicitly (useful for a portable install or development build):

```sh
STAYPRODUCTIVE_X11_ADAPTER="$PWD/stayproductive-x11" ./stayproductive
```

`STAYPRODUCTIVE_X11_ADAPTER=0` disables the adapter. Diagnostics:

```sh
cat "${XDG_RUNTIME_DIR:-/tmp}/stayproductive/x11-adapter.json"
cat "${XDG_RUNTIME_DIR:-/tmp}/stayproductive/x11-adapter.log"
```

The adapter covers application windows only. It also reports the focused app so
daily and hourly application usage limits work on X11, including while the GTK
window is closed. A compositor without EWMH support and dynamic workspace
changes are not handled yet. Card placement across monitors and the lock/suspend
handshake are implemented, but Xvfb cannot present two monitors to GDK or lock a
real session, so those visuals still need a logged-in X11 desktop; the live test
asserts the single-monitor heartbeat fields.
Xvfb test runs use a direct unmap because there is no window manager; a real X11
login is still needed to judge iconification and overlay visuals. Start at login
also depends on the session exporting `DISPLAY`/`XAUTHORITY` to the systemd user
manager; enabling autostart from Settings imports them for the current manager,
and the unit passes them through with `PassEnvironment`. See
[`docs/spec/implementation-notes.md`](docs/spec/implementation-notes.md).

### Verify the engine

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The acceptance suite is `crates/core/tests/acceptance.rs`; every test names the
`A-*` identifier from `docs/acceptance/parity-tests.md`. The end-to-end service
test is `crates/daemon/tests/protocol.rs`; it drives the real daemon binary over
stdio and over the Unix socket, and checks strict locks, the cool-down refusal,
restart persistence, state-file permissions, socket privacy and single-instance
behaviour, the generated enforcement policy, and the extension heartbeat gate.
The GNOME matching helpers have a compositor-free test:

```sh
gjs -m integrations/gnome/tests/matching.test.js
PYTHONPATH=crates/ui python3 -m unittest discover -s crates/ui/tests
```

The X11 adapter has a live check against a throwaway Xvfb display; it is opt-in
because it starts an X server and a second GTK process:

```sh
STAYPRODUCTIVE_X11_LIVE=1 PYTHONPATH=crates/ui \
  python3 -m unittest crates.ui.tests.test_x11_live
```

The Python suite covers desktop-entry discovery, the onboarding goal/starter
profile logic, the socket client (detach, reconnect, state mismatch), the
systemd unit text and service-state logic, and the display-free X11 matching,
monitor-card choice, and planner pause/resume rules. The GTK wizard itself is
construction-smoke-tested under Xvfb; visual inspection is not an acceptance
test.

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
  JSON atomically. The desktop transport is newline-delimited JSON over a
  same-user Unix socket, owned by a systemd user unit; stdio remains for tests.
  The D-Bus surface in the architecture document replaces it without moving
  decision logic out of `core`.
- **Desktop UI**: GTK 4 + libadwaita through PyGObject. The Python layer only
  renders state and sends commands; it contains no blocking decisions.
- **Desktop integration**: an X11 EWMH adapter (a supervised standalone process
  that hides blocked windows and draws a per-monitor overlay) and an opt-in
  GNOME Shell extension for application windows, both driven by a short-lived
  policy file and a runtime heartbeat. Further adapters (sway/i3 IPC, Hyprland
  IPC, KWin script), the browser extension (Manifest V3), and the optional
  privileged DNS helper are planned.
- **Packaging**: the systemd user unit is written by the app today; Flatpak (user
  mode, no system-wide blocking), `.deb`, `.rpm`, and Arch packages are planned.

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
