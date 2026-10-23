# Core engine implementation notes

Status: **M0 core implemented; runnable local UI and state service.** The pure
engine, scheduler, period calculator, usage ledger, and domain model live in
`crates/core`. `crates/daemon` owns state and persists it atomically as JSON.
`crates/ui` is a GTK 4 + libadwaita front end. Desktop adapters, SQLite, and
D-Bus are not implemented yet; those are tracked in the parity matrix.

These notes record decisions the existing specs left implicit. Anything marked
**TODO(spec)** is deliberately unfinished and must not be read as implemented.

## Implemented and covered by tests

`crates/core/tests/acceptance.rs` mirrors the acceptance IDs:

| Area | Acceptance IDs |
| --- | --- |
| Profiles | `A-PROF-01`, `A-PROF-02`, `A-PROF-03`, `A-PROF-05`, `A-PROF-06` |
| Rules | `A-RULE-01` … `A-RULE-05`, `A-RULE-07`, `A-RULE-08` |
| Limits | `A-LIM-01` … `A-LIM-06` |
| Schedules | `A-SCH-01` … `A-SCH-10` |
| Content | `A-CON-06` |
| Data | `A-DATA-02` (backwards clock jump, commitment end preserved) |
| Commitments | cool-down request/finish, locked edits and pauses |

Unit tests additionally cover DST day lengths, the local-date period
calculator, schedule gap/merge behaviour, keyword folding, and ledger
attribution. The suite currently has 32 acceptance tests.

## Decisions made in code

### Rule identity

Rules carry a ULID (`AppRuleEntry`, `SiteRule`, `KeywordRule`) even though the
domain-model snippets show bare rule values. `Decision.rule` points at it, so
the UI can explain a block by rule. This is additive and does not change the
behavioural model.

### Launch limits

A limit of *N* allows *N* launches in the period and blocks the *(N+1)*-th.
The anti-thrash window is measured as time **away** from the app: leaving and
returning within 5 s does not count as a new launch. This matches `A-LIM-03`
(“focused twice inside 5 s … counts once; a third launch after 5 s is blocked”).

Usage limits are different: they block when `usage >= limit`, exactly as
`A-LIM-01` requires.

### Wait timers

A wait timer is periodic inside the logical day. Each time cumulative usage
reaches another multiple of `use_for`, access is denied for `wait` from the
crossing instant. With `adaptive`, once half of the wait has elapsed the
remaining wait is halved once. This is the normative definition in
`docs/spec/blocking-engine.md` §4; the adaptive sentence remains **inferred**
from the reference.

### Commitments and strict profiles

An active commitment locks the rules that are active at that instant; it does
not invent new rules. An explicit commitment allowlist excludes the target from
the strict block. Otherwise `R-STRICT` wins over allowlists, pauses, and
expired limits. A strict-eligible profile that is inside its window produces
`R-STRICT` for any target it has a rule for, including when another profile is
paused.

### Start-of-day and DST gaps

`logical_date(now) = date(local_now - start_of_day)`, then period boundaries are
resolved from local dates. A local time inside a DST gap is shifted forward by
the gap length; a window that both starts and ends inside the gap collapses to
zero length and does not run. Ambiguous fall-back times resolve to the earliest
occurrence, so the repeated hour is evaluated once.

### Pickup limits

`RuleSet` carries an optional `pickup_limit` (`CountLimit`). It is not in the
domain-model snippet but is required by `A-LIM-04`; it applies globally to the
profile's targets. The next decision after the limit is reached reports
`R-PICKUP`.

### Local state service

`crates/daemon` is the first implementation of the daemon boundary. It owns the
only mutable `State`, exposes one command per line of JSON on stdin, and writes
one response per line on stdout. The greeting is
`{"event":"ready","state_path":...}`; every request carries an `id`, so a caller
can match replies. The UI starts the process, waits for the greeting, and sends
`shutdown` on exit.

Persistence is deliberately simpler than the target SQLite store:

- `$STAYPRODUCTIVE_STATE`, then
  `$XDG_DATA_HOME/stayproductive/state.json`, then
  `$HOME/.local/share/stayproductive/state.json`;
- `{"version":1,"state":{...}}` wrapper;
- write to a sibling `json.tmp`, set mode `0600`, then rename over the target;
- parent directory created and set to `0700` on Unix;
- a newer or unreadable wrapper is renamed to `state.invalid-<timestamp>.json`
  and a fresh state starts. This preserves the bytes but is not yet a user-facing
  migration/repair flow, so it remains a TODO.

The service has no network listener, no root privileges, and no D-Bus name yet.
Those belong to M1. Keeping the transport narrow makes the transition to D-Bus a
replacement rather than a rewrite of the decision path.

After every successful command the service also publishes
`enforcement.json` beside the state file. It contains only applications the
engine would block at that instant: the app key, `R-*` reason, optional end
time, profile name, and a `valid_until` 30 seconds in the future. The file is
atomic and mode `0600`. Expiry is intentional: if the app stops refreshing the
policy, a consumer must stop enforcing rather than preserve a stale block.

The service reports enforcement as `active` only while
`$XDG_RUNTIME_DIR/stayproductive/gnome-extension.json` says `active:true` and
`policy_loaded:true`, and has a `checked_at` no more than five seconds old. The
extension writes that heartbeat every two seconds. Without a fresh heartbeat the snapshot says
`unavailable` and names the missing adapter. This keeps “active” an observation,
not a configuration claim.

### GNOME enforcement adapter

`integrations/gnome/stayproductive@hassanraza.ai` is a GNOME Shell 45–49
extension. It is a user-level extension, not a privileged component:

- every second it reads the short-lived policy and discards expired or
  wrong-version files;
- it watches `global.display.focus_window` and collects `WM_CLASS`, the
  WM_CLASS instance, the GApplication id, and the sandboxed app id, normalizing
  each and adding the trailing component as an alias (`org.mozilla.firefox` →
  `firefox`);
- a matching entry shows a full-screen `St.BoxLayout` chrome actor on the whole
  stage, with the reason, blocked app title, end time, and local-only copy;
- the overlay takes key focus and stops button/key events; switching to an
  allowed window hides it, and returning to the blocked window shows it again;
- the heartbeat reports `policy_loaded`, `blocked`, and the Shell version. It
  contains no window titles or per-app history.

The extension only covers application windows. It cannot stop a process that
runs in the background, cannot read browser tabs, and cannot enforce site,
keyword, or usage-limit decisions. Those remain separate adapters. Because the
policy is refreshed by the running app and expires after 30 seconds, closing
StayProductive ends desktop enforcement; the daemon is not yet autonomous.

Extension loading, policy parsing, heartbeat writes, and expiry were checked
in a headless GNOME Shell 49 session. The headless compositor has no seat, so
`focus_window` is null there; the visible overlay, focus matching, and
multi-monitor layout could not be exercised without a logged-in session.
Runtime visual verification therefore still needs a real GNOME login. The pure
app-id/matching helpers are covered by
`integrations/gnome/tests/matching.test.js`.

The Settings page copies the extension into
`$XDG_DATA_HOME/gnome-shell/extensions/`, adds it to
`org.gnome.shell enabled-extensions`, and explains that a first-time install
needs one log out / log in. It does not silently restart the Shell.

### GTK frontend

`crates/ui` is GTK 4 + libadwaita through PyGObject. It contains no blocking
decision logic; every mutation is a command to the service, and every screen is
rendered from the returned snapshot. Pages:

- **Dashboard** — active profiles, next boundary, rule count, commitment,
  honest enforcement state, quick block form, recent decisions.
- **Profiles** — CRUD, manual activation, pause/resume, delete confirmation,
  and three newly written starter profiles (Work hours, Evening wind-down,
  Deep work).
- **Profile editor** — activation, enabled/strict flags, priority, six schedule
  kinds (daily, selected days, split windows, full day, hourly cycle, date
  range), app/site rules with limits, keywords, allowlists, and pickup limits.
- **Schedules** — schedule summary and active/waiting/next-start state.
- **Strict session** — start, visible commitment, request early exit, cool-down
  countdown, and end when the cool-down is ready.
- **Activity** — active quick blocks and recorded block decisions.
- **Settings** — start of day, IANA timezone, desktop integration install and
  status, policy path, capability status, data path, copy path, open folder, and
  About dialog.

The front end refreshes from the service every five seconds and after each
mutation. It shows “Enforcement active” only when the extension heartbeat is
fresh and reports a loaded policy; otherwise it shows “Enforcement offline” and
explains how to install or load the adapter.

### Strict commitments and cool-down

`start_strict` creates a commitment with `Restriction::BlockEdits` and
`Restriction::BlockPause`. `State::edits_blocked(now)` is the single policy
check used by the daemon before creating, updating, deleting, activating, or
deactivating a profile. The UI also hides those actions when they would be
refused, so the user sees an explanation rather than a silent failure.

An early exit has two steps:

1. `request_commitment_end` enters `CoolingDown` and returns the absolute
   cool-down end.
2. `finish_commitment_end` succeeds only when `now >= cooling_down_end`; before
   that it returns `CommitmentNotReady`.

The commitment keeps enforcing the rules that are active while it runs. It does
not invent rules for a profile that has left its schedule. The state is
persisted before the UI refreshes, so a restart does not shorten the
commitment.

Known gaps in this first strict implementation:

- the cool-down deadline is wall-clock, so it does **not** pause during suspend
  even though `strict-and-lock.md` §5 requires that; implementing that needs a
  logind/suspend signal and stored active-cool-down time;
- only the cooldown method is accepted; text, QR and approval challenges return
  `UnsupportedCommitmentMethod` instead of pretending to work;
- strict mode is a commitment device, not DRM. The JSON file is readable and
  editable by the same local user; permissions keep out other accounts, not the
  owner. Package removal or root access ends every commitment.

`crates/daemon/tests/protocol.rs` drives the real service binary over stdio. It
covers locked profile mutations, rejected pause, refused early finish, restart
persistence, and the private state-file mode.

### What is still inert

The engine stores and explains site daily limits, but does not yet decrement
them because there is no per-site interval tracker. Pickup limits likewise need
the logind/tracker adapter. The UI labels both as configuration rather than
claiming they are enforced. The GNOME adapter now enforces app windows, but the
browser extension, other compositors, and the usage tracker remain the largest
missing pieces.

## Not implemented yet

- **`A-RULE-06` / `AppRule::Goal`** — credit earning and `AllowWithNotice` need a
  mutable credit ledger owned by the daemon. TODO(spec): define rounding, credit
  expiry and what is shown while credit remains.
- **`A-PROF-04` templates** — the UI ships three ordinary starter profiles, but
  the reference template set and acceptance scenario are not yet specified.
  TODO(spec).
- **Site time limits** — the extension must first record per-site intervals;
  `SiteAction::Limit` is stored and explained but inert rather than guessed at.
  TODO(spec).
- **Content surfaces** (`A-CON-01`…`A-CON-05`) — the engine has no surface rule
  set or surface matching yet.
- **SQLite persistence, migrations, and history** (`F-DATA-01`) — the current
  store is versioned JSON with atomic replace; it has no queryable ledger yet.
- **Desktop enforcement beyond GNOME app windows** — the GNOME extension covers
  application windows only; X11 standalone (EWMH), sway/i3, Hyprland, KWin,
  website, keyword, and limit enforcement still need adapters. The UI labels the
  adapter and its scope.
- **D-Bus, systemd user unit, tray, CLI** — the current boundary is stdio JSON
  and the UI launches the service itself.
- **Usage statistics, notifications, onboarding, export/delete, backup/restore,
  localisation catalogs, full accessibility audit**.
- **Low-time alerts** (`A-LIM-07` / `A-PLAT-11`).

## Test commands

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
gjs -m integrations/gnome/tests/matching.test.js
gnome-extensions pack integrations/gnome/stayproductive@hassanraza.ai --out-dir /tmp
cargo build --release -p stayproductive-daemon
./stayproductive
```

The Python frontend has no separate test runner yet; UI verification currently
uses a harness that starts the real app, exercises the state service through the
same calls the buttons use, and inspects persisted state. Turning those checks
into an automated UI suite is tracked in the parity matrix.
