# AGENTS.md — working agreement for this repository

You are implementing **StayProductive**, an open-source Linux focus app whose
behaviour is specified in `docs/`. Read this file, `docs/legal/clean-room.md`,
and `docs/parity/feature-matrix.md` before changing code.

## Mission

Ship a Linux desktop app that a Stay Focused 13.0.2 user would recognise as the
same product: same blocking decisions, same schedules, same limits, same strict
mode semantics, same flows and explanations — implemented with native Linux
mechanisms and clean-room code.

"Identical behaviour" is judged against `docs/spec/` and
`docs/acceptance/parity-tests.md`, not against Android internals. When the two
conflict, the spec wins and the conflict is recorded.

## Non-negotiable rules

1. **Clean room.** Do not copy, port, translate, or paste code, smali, resource
   files, strings, icons, sounds, or assets from the reference APK or from any
   decompiled output. Specs describe behaviour; you write new code and new
   copy. See `docs/legal/clean-room.md`.
2. **No proprietary artifacts in git.** APKs, decompiled trees, decoded smali
   projects, native libraries, and extracted assets are git-ignored and live
   under `reference/` only. Never commit them.
3. **Evidence labels.** Analysis documents use *observed*, *derived*, and
   *inferred*. Do not upgrade an inference to a requirement without saying so.
   Unknowns stay unknown; add a `TODO(spec)` entry instead of guessing silently.
4. **The spec is the contract.** Implement against IDs (`F-*`, `A-*`), not
   against your memory of the Android app. If the spec is silent, write the
   missing section first, then implement.
5. **Local-first.** No account, network, telemetry, or cloud call may be
   required for any user-facing feature. Optional services must degrade
   gracefully and be off by default.
6. **Never make the machine unrecoverable.** Blocking may be strict, but a user
   must always retain a documented, working deactivation path that does not
   require reinstalling the OS. Root-only bypass is out of scope.

## Repository map (target layout)

```
crates/
  core/        domain model, rule engine, scheduler, counters, storage
  daemon/      session service, trackers, enforcers, IPC (D-Bus)
  ui/          GTK4 + libadwaita application
  cli/         `stayproductive` command-line control
integrations/
  x11/ sway/ hyprland/ gnome/ kwin/    compositor adapters
  browser-extension/                   Manifest V3 companion
  dns-helper/                          optional privileged DNS sinkhole
packaging/                             flatpak, deb, rpm, arch, systemd units
docs/                                  this specification
tests/                                 cross-crate and end-to-end tests
```

Layering: `core` has no UI, no compositor, no D-Bus dependency. Adapters depend
on `core`, never the reverse. Time, randomness, and process/desktop observation
enter through traits so tests run deterministically.

## How to pick up work

1. Choose the highest-priority `TODO` row in `docs/parity/feature-matrix.md`.
2. Read the linked spec section and the acceptance scenarios (`A-*`).
3. Write failing tests first: unit tests in the owning crate, scenario tests in
   `tests/`.
4. Implement the smallest change that satisfies the scenarios.
5. Update the matrix (status + test IDs) and any spec text your change refined.
6. Run `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, and the
   relevant integration suite; record what you actually ran.

## Engineering rules

- **Deterministic decisions.** The rule engine is a pure function of
  `(state, event, now)`. No wall-clock reads inside decision code; pass `now`.
- **Precedence is explicit.** Strict/lock > active profile schedule > limit
  exhaustion > keyword/site rule > allowlist. Every override is logged with a
  reason code (`R-*` in the spec).
- **Fail closed for enforcement, fail open for recovery.** If a tracker dies,
  keep the last known block active and surface a degraded state. If the app
  crashes, the daemon keeps enforcing; the UI is optional.
- **Idempotent migrations.** SQLite schema changes ship with forward migrations
  and a backup. Never destroy user rules, history, or commitment state.
- **Bounded work.** Debounce high-frequency events; cap event queues; never
  block the UI thread on storage or compositor calls.
- **Explain every block.** The UI must be able to say what rule fired, which
  limit was crossed, and when the block ends.
- **Accessibility and i18n from day one.** All user-visible text goes through
  translation catalogs; no string concatenation for sentences; keyboard
  navigation and screen-reader labels are part of "done".

## Security and privacy

- Default install is unprivileged. System-wide DNS blocking requires an
  explicit, separately packaged helper with a narrow D-Bus/polkit interface.
- Never log URLs, window titles, or per-app usage at a level the user did not
  opt into; local history is user data and must be exportable and deletable.
- The daemon must resist casual tampering (kill, disable service) to the same
  *product* degree as strict mode: delays and commitments, not rootkits.
  Document the threat model honestly.

## Definition of done

A change is done when:

- [ ] the acceptance scenarios for its IDs pass, including edge cases named in
      the spec (clock change, restart, multi-monitor, screen lock, suspend);
- [ ] no proprietary material was introduced;
- [ ] `cargo fmt`, `clippy -D warnings`, and tests pass in CI;
- [ ] user-visible strings are translatable and the block reason is visible;
- [ ] the parity matrix and spec are updated;
- [ ] limitations are stated in the PR description, not hidden.

## What not to do

- Do not reproduce Android-only hacks (accessibility-service view IDs, home-key
  interception, power-menu blocking, device-admin tricks). Map the **intent** to
  Linux mechanisms described in `docs/architecture/linux-design.md`.
- Do not claim runtime parity you did not measure. Static analysis of the
  reference is not runtime verification.
- Do not add a paywall, ads, analytics, or an account gate.
- Do not "fix" a spec conflict silently. Record it.
