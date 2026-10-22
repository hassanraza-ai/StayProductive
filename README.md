# StayProductive

An open-source focus and self-control app for Linux, built to be **behaviourally
compatible** with Stay Focused 13 on Android: app and website blocking,
time limits, schedules, profiles, strict mode, content filtering, usage
statistics, and the companion tools around them — reimplemented natively for the
Linux desktop.

This repository currently contains the **specification and handoff pack**: the
behavioural model, parity matrix, Linux architecture, and acceptance criteria
needed to implement the app. The code is written clean-room from these
documents; no code, resources, or assets from the reference app are included or
licensed.

## Status

| Phase | State |
| --- | --- |
| Behavioural analysis (static) | complete, see `docs/parity/` |
| Product specification | complete for P0/P1, see `docs/spec/` |
| Linux architecture | decided, see `docs/architecture/` |
| Implementation | **not started** |
| Runtime parity verification | impossible against the reference without a device; parity is defined by the acceptance suite |

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

## Planned stack

- **Core daemon**: Rust — deterministic rule engine, scheduler, SQLite store,
  D-Bus service.
- **Desktop UI**: GTK 4 + libadwaita (primary), with a documented adapter
  boundary so a Qt/KDE front end can be added.
- **Desktop integration**: compositor adapters (X11 EWMH, sway/i3 IPC,
  Hyprland IPC, GNOME Shell extension, KWin script), browser extension
  (Manifest V3), optional privileged helper for system DNS.
- **Packaging**: Flatpak (user mode, no system-wide blocking), `.deb`, `.rpm`,
  Arch package, and a systemd user unit.

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
