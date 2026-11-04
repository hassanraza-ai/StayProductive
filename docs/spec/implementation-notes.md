# Core engine implementation notes

Status: **M0 core implemented; runnable local UI and autonomous state service.**
The pure engine, scheduler, period calculator, usage ledger, and domain model
live in `crates/core`. `crates/daemon` owns state, persists it atomically as
JSON, and runs behind a same-user Unix socket under a systemd user unit.
`crates/ui` is a GTK 4 + libadwaita front end. Desktop adapters beyond GNOME,
SQLite, and D-Bus are not implemented yet; those are tracked in the parity
matrix.

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
attribution. The suite currently has 32 acceptance tests. The UI test suite
(`crates/ui/tests`) adds display-free tests for application discovery, onboarding
(`A-UX-01`), the socket client, and the systemd unit (`F-PLAT-01`).

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
only mutable `State` and speaks one JSON request/response per line. The greeting
is `{"event":"ready","state_path":...}`; every request carries an `id`, so a
caller can match replies.

Two transports share that protocol:

- `--socket [PATH]` is the desktop transport. The daemon binds a same-user Unix
  socket (mode `0600`, parent directory `0700`) at
  `$XDG_RUNTIME_DIR/stayproductive/daemon.sock`, or at the path from
  `--socket`/`$STAYPRODUCTIVE_SOCKET`. A non-default `$STAYPRODUCTIVE_STATE`
  hashes to a distinct `daemon-<fnv1a64>.sock` so test and portable instances
  never share the login daemon. A second instance detects the live socket and
  exits successfully instead of taking over; a stale socket file is removed.
- stdio (the default) remains for the protocol tests and one-shot tooling. Each
  process exits with its input.

The UI connects to the socket and never owns the daemon's lifetime: closing or
killing the window leaves enforcement running, and relaunching reconnects to the
same process. If no daemon is reachable, the UI starts `stayproductive.service`
through systemd when that is manageable, otherwise it spawns a detached session
(`start_new_session=True`) and logs to `<state dir>/daemon.log`. Explicit stop is
`systemctl --user stop stayproductive.service` or a `shutdown` request; the
daemon removes its socket on exit.

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

### Data export and deletion

`export_data` writes a versioned archive to
`<state dir>/exports/stayproductive-export-YYYYMMDD-HHMMSS.stayproductive.zip`
or to a `path` the caller supplies. The zip uses stored (uncompressed) entries
and a fixed header date, so the same state and clock produce the same bytes, and
the daemon needs no compression dependency. Entries follow
`docs/spec/domain-model.md` §13: `manifest.json` (`format`, `version`,
`exported_at`, `app_version`, `usage_included`), `profiles.json` (profiles with
rules and schedules, the manual activation, quick blocks, and schedule
extensions), `settings.json`, `commitments.json`, and — when the user keeps the
default — `usage/YYYY-MM.json`, one file per month with foreground intervals,
launches, pickups, and block reasons. The open foreground interval is exported
with `end = now`, because dropping it would hide the session in progress. History
is opt-in for callers (`include_usage:false`); the UI exports it by default
because the user is asking for a copy of their data.

`delete_history` requires `confirm:true`. It resets the usage ledger — closed and
open intervals, launches, pickups, block records, and the last heartbeat — and
persists the empty ledger, while profiles, schedules, quick blocks, settings, and
commitments are untouched: deleting history never weakens an active block or
ends a strict session. Both commands are also available from the Activity page,
which states the scope in the confirmation and keeps no copy.

An export is a privacy feature, so it is written with mode `0600` like the state
file. The archive is not encrypted and has no signature; encrypting it is left to
the user's filesystem or archive tool. Import (`A-UX-12`) is not implemented yet;
`manifest.json` exists so an importer can refuse an unknown `version` instead of
guessing.

### Usage tracking

Each adapter heartbeat is also a foreground observer. Besides the liveness
fields it carries `focus` (normalized app ids of the focused window, or `null`)
and `focus_since` (when that window became foreground). The GNOME extension and
the X11 adapter report the same shape; when both are fresh the extension wins
because an X11 GNOME session hands enforcement to it. On every command, and on
the daemon's own five-second pulse, the newest observer feeds the core
`UsageLedger`:

- a fresh heartbeat with an attributable focus opens or continues a foreground
  interval for the profile's own app key (both trailing-component alias
  directions are matched, `org.mozilla.firefox` ↔ `firefox`);
- a stale or missing heartbeat, a block overlay (`blocked: true`), or
  `focus: null` closes the open interval: no usage accrues without an observer
  or behind a block screen;
- an interval never starts before the last observation, so time the daemon
  already closed (a finished block, a lost observer) cannot be re-attributed
  from an older `focus_since`. With no prior observation the observer's
  `focus_since` is trusted;
- only apps a profile rules over are tracked. Unconfigured apps produce no
  intervals, which keeps the ledger bounded and the observation minimal.

Sampling follows the UI's five-second poll and the daemon's own five-second
pulse when no client is attached, so usage keeps accruing while the GTK window
is closed. It is under-counted by at most one sampling interval and never
over-counted; the engine additionally counts the open interval up to `now`, so
a limit decision at the boundary does not lag. The ledger's in-memory event log
is capped (`EVENT_LOG_CAP`) and not persisted; intervals, launches, pickups,
blocks, and the last heartbeat are.

Because the ledger feeds `decide`, daily and hourly application limits now
produce real policy entries (`R-LIMIT-DAILY`/`R-LIMIT-HOURY`) once usage
reaches the limit. Blocks that newly engage are recorded in the usage history
with their reason and end time; the first policy after a daemon start only
seeds the comparison so a restart does not duplicate records. The snapshot
exposes `usage_today` (seconds per app for the current logical day), which the
Activity page renders.

After every successful command the service also publishes
`enforcement.json` beside the state file. It contains only applications the
engine would block at that instant: the app key, `R-*` reason, optional end
time, profile name, and a `valid_until` 30 seconds in the future. The file is
atomic and mode `0600`. Expiry is intentional: if the daemon stops refreshing
the policy, a consumer must stop enforcing rather than preserve a stale block.

In socket mode the daemon refreshes that policy itself every five seconds, not
only after a client command. Schedule starts, ends, and committed blocks
therefore keep working with no window open.

The service reports enforcement as `active` only from a fresh adapter
heartbeat under `$XDG_RUNTIME_DIR/stayproductive/`. It reads
`x11-adapter.json` first and then `gnome-extension.json`; the file must declare
the expected `adapter`, say `active:true` and `policy_loaded:true`, and have a
`checked_at` no more than five seconds old. The X11 entry wins so an X11 session
reports the adapter that is actually operating. Without a fresh heartbeat the
snapshot says `unavailable` and names the adapter expected for the session. This
keeps “active” an observation, not a configuration claim.

### Daemon autonomy and autostart (`F-PLAT-01`)

`crates/ui/stayproductive/service.py` owns the systemd user unit
`stayproductive.service`:

- `ExecStart="<daemon>" --socket`, `Restart=always`, `RestartSec=2`,
  `NoNewPrivileges=yes`, `PassEnvironment=DISPLAY XAUTHORITY WAYLAND_DISPLAY
  XDG_SESSION_TYPE`, `WantedBy=default.target`, and an explicit
  `Documentation=` line;
- `start()` writes the unit, reloads systemd, imports the graphical-session
  variables into the user manager, and starts it for this session;
- `enable()` is the autostart grant: it stops a directly spawned daemon first so
  the unit wins the single-instance socket check, then `enable`s and `start`s;
- `disable()` runs `systemctl --user disable` and leaves this session's daemon
  running; blocking continues until logout or an explicit stop.

Autostart is offered only when a systemd user manager answers and the session
uses the default data file. `service_state()` reports `unavailable`,
`custom-state`, `enabled`, `installed`, or `off`, and both Settings and the
onboarding capability step render that state rather than assuming success.
Nothing is enabled silently: the switch and the “Set up” button are explicit
user actions.

Settings → Background service also states that the daemon is independent of the
window and names the stop command. `Engine.close()` only detaches;
`Engine.shutdown_service()` is reserved for the systemd handover. Engine writes
are not retried automatically, but `snapshot()` retries once after a connection
failure so a daemon restart does not force the user to reopen the window.

Evidence:

- `crates/daemon/tests/protocol.rs` — socket privacy and mode, single instance,
  stale-socket takeover avoidance, and a policy refreshed with no client.
  `export_and_delete_usage_data` builds a real archive, parses its central
  directory, checks the manifest, profile, settings, and monthly usage entries,
  refuses an unconfirmed delete, and verifies the deletion survives a restart.
  The zip was also opened with Python's `zipfile` (`testzip()` clean).
- `crates/ui/tests/test_engine.py` — round trip, disconnect on close, reconnect
  after restart, mismatched state-file refusal, and the explicit
  `include_usage`/`path`/`confirm` intent of export and delete.
- `crates/ui/tests/test_service.py` — unit text, quoting, socket naming, service
  states, and the systemd handover order.
- Manual end-to-end under Xvfb: UI spawned a detached daemon; `SIGKILL`ing the
  UI left it alive and the policy timestamp advanced; relaunching reused the same
  daemon PID; an explicit shutdown removed process and socket.
- Manual restart on a transient systemd unit: `kill -9` of `MainPID` produced a
  new PID and the unit stayed active.

Remaining limitation: the Settings toggle has not been exercised with a real
`systemctl --user enable` against an installed unit; that needs a real login
session. There is no D-Bus name yet, and the unit is not installed by any
package manager.

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
- the heartbeat reports `policy_loaded`, `blocked`, the Shell version, and the
  focused app ids with `focus_since` for usage tracking. It contains no window
  titles or per-app history.

The extension covers application windows and reports focus for application
usage limits. It cannot stop a process that runs in the background, cannot read
browser tabs, and cannot enforce site, keyword, site-limit, launch-limit, or
pickup decisions. Those remain separate adapters. The socket daemon refreshes
the policy independently, so closing the window no longer ends gentle
enforcement or usage tracking; killing the daemon still stops it, and
autostart/restart is the systemd unit described below.

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

### X11 enforcement adapter (`F-PLAT-02`)

`crates/ui/stayproductive/x11_adapter.py` is a standalone process started and
supervised by the daemon in an X11 session (`DISPLAY` set, no
`WAYLAND_DISPLAY`, `XDG_SESSION_TYPE` not `wayland`). The
`STAYPRODUCTIVE_X11_ADAPTER` environment variable is both an override and an
adapter path; `0`/`off` disables the adapter. GTK 4 draws the overlay and a
small `ctypes` libX11 binding (`x11.py`) performs window operations. There is no
Python Xlib or libwnck dependency, so the adapter shares GTK 4 with the front
end.

- **Focus** — `_NET_ACTIVE_WINDOW`, falling back to `XGetInputFocus` plus a
  `XQueryTree` walk to the toplevel when no EWMH window manager is present.
- **Identity** — `WM_CLASS` name and class, `_GTK_APPLICATION_ID`,
  `_GTK_APP_ID`, `_BAMF_DESKTOP_FILE_HINT`, `_NET_WM_DESKTOP_FILE`, and the
  PID's `comm`, executable basename, and first `cmdline` basename. A rule key
  is normalized (lower-case, `.desktop` removed) and the last dotted and
  underscored component are added as aliases, matching the GNOME helper.
- **Decision order** — `x11_policy.py` is display-free. `Planner` emits
  `hide_window` then `show_overlay`; repeated focus is a no-op; switching to an
  allowed window hides the overlay; Back focuses the previous allowed window
  and then hides; a policy refresh updates the copy without re-hiding. When the
  policy expires or disappears, `clear()` hides the overlay and maps/focuses
  the blocked window so enforcement cannot leave a window unreachable. Dock,
  panel, menu, notification, and other non-application window types are
  ignored, and the last known block is kept while they have focus.
- **Overlay** — one override-redirect GTK window per `Gdk.Monitor`, positioned
  and raised through Xlib and given keyboard focus. Every monitor is covered;
  the explanation card is only shown on the monitor that shares the most area
  with the blocked window (`preferred_monitor`), and keyboard focus goes to
  that window so the Back button is where the user is looking. A window with
  no known geometry or one entirely off screen falls back to the first
  monitor, so enforcement never weakens. The card names the app, the reason
  from the `R-*` code, the end time, a Back action, and local-only copy. The
  overlay is created before it is mapped so the window manager does not
  decorate or reposition it.
- **Lock and suspend** — `session.py` watches systemd-logind over the system
  bus: `Manager.PrepareForSleep`, `Session.Lock`/`Unlock`, and a throttled
  (two-second) `LockedHint` property read as a fallback when a signal is
  missed. While locked or suspended the planner is paused: the overlay is
  released so it never covers a lock screen or a security prompt, focus
  changes are ignored so the lock screen cannot be mistaken for an allowed
  application, and the blocked window stays hidden. Unlocking resumes the
  planner, and the next focus event re-blocks the still-hidden window; a
  policy that expired during the pause is cleared on resume, which restores
  the window. Focus reporting is cleared too, so a locked or suspended session
  never accrues app usage. When logind is unreachable the adapter reports
  `session_aware:false` and keeps enforcing without pausing.
- **Hide** — ICCCM `WM_CHANGE_STATE` `IconicState` when
  `_NET_SUPPORTING_WM_CHECK` exists; a direct `XUnmapWindow` when there is no
  window manager (headless/Xvfb).
- **Heartbeat** — `$XDG_RUNTIME_DIR/stayproductive/x11-adapter.json` is written
  after every action and at least once a second. It contains `adapter:"x11"`,
  `active`, `policy_loaded`, `blocked`, `overlay_visible`, `blocks`, `focus`
  (normalized aliases of the focused application window, empty behind the
  overlay or while paused), `focus_since`, `monitors`, `card_monitor`,
  `locked`, `suspended`, `session_aware`, `display`, and `checked_at`; no
  window titles or per-app history are recorded.
- **Supervision** — the daemon finds the launcher from an explicit
  `STAYPRODUCTIVE_X11_ADAPTER`, `stayproductive-x11` on `PATH`, or the checkout
  next to the binary, then starts it with `--state`, `--heartbeat`, and
  `--parent-pid`. Output goes to
  `$XDG_RUNTIME_DIR/stayproductive/x11-adapter.log`. Every second the daemon
  polls the child, removes a dead adapter's heartbeat immediately so the
  snapshot stops claiming enforcement, and restarts it after a two-second
  backoff. The adapter also exits when its parent PID disappears. A fresh GNOME
  extension heartbeat takes precedence and stops the X11 adapter; if the
  extension goes stale the daemon starts the X11 adapter again.
- **systemd** — the user unit passes `DISPLAY`, `XAUTHORITY`, `WAYLAND_DISPLAY`,
  and `XDG_SESSION_TYPE` through `PassEnvironment`; `start()` and `enable()`
  import the same variables into the systemd user manager first, so a unit
  started before the desktop session still knows it can run the adapter.

Evidence:

- `crates/ui/tests/test_x11_policy.py` — aliases, policy expiry, hide/overlay
  ordering, dedupe, re-block, allowed-window hide, and Back with and without a
  previous window. `PreferredMonitorTest` covers single-monitor, left/right
  monitor, spanning-window majority, off-screen and missing geometry, and no
  monitors; `PlannerPauseTest` covers pause hiding the overlay, updates and
  Back being ignored while paused, resume re-blocking the still-hidden window,
  and a policy that expires during the pause being restored on resume;
  `FocusReporterTest` covers the reported aliases, a stable `focus_since` for
  an unchanged window, a restart on focus change, and no focus while blocked or
  paused.
- `crates/daemon/tests/protocol.rs::x11_focus_reports_feed_usage_limits` — an
  X11 heartbeat is the only observer: reported focus crosses a daily limit,
  produces `R-LIMIT-DAILY`, and stops accruing while the overlay is up.
  `socket_service_tracks_usage_without_a_client` checks the same attribution
  through the daemon's own five-second pulse.
- `crates/daemon/tests/protocol.rs::socket_service_supervises_the_x11_adapter`
  — a fake adapter writes a heartbeat, is `SIGKILL`ed, and is restarted; the
  snapshot reports `active`/`x11`, and shutdown leaves no orphan.
- `crates/ui/tests/test_x11_live.py` (opt in with
  `STAYPRODUCTIVE_X11_LIVE=1`) — Xvfb end-to-end: a focused blocked window is
  unmapped, switching to an allowed window lowers the block, refocusing
  re-blocks and increments `blocks`, an expiring policy maps the window again,
  and the heartbeat reports `monitors`, `card_monitor`, `locked`,
  `suspended`, `session_aware`, `focus`, and `focus_since` (empty behind the
  overlay, the allowed window otherwise).
- Manual Xvfb end-to-end with the real daemon and real adapter: an active
  profile produced the policy, the adapter blocked the focused window, and the
  snapshot reported `active`/`x11`.

Limitations: Xvfb has no window manager, so the measured run exercised the
`XUnmapWindow` fallback rather than WM iconification, real decorations, or a
compositor's focus rules. Xvfb also cannot present two RandR monitors to GDK,
so the multi-monitor card choice is covered by display-free unit tests only;
overlay pixels, the card position, and focus behaviour on a real multi-monitor
X11 session still need a logged-in desktop. The logind wiring is exercised
against the live system bus in the heartbeat (`session_aware`), but locking a
real session to watch the release/resume handshake has not been done and is
left to a manual check. When logind is unreachable the adapter keeps the
block active and may cover a lock screen, which is the documented trade-off
between enforcement and recovery. Dynamic workspaces and compositors that
refuse override-redirect focus remain open (`A-PLAT-09`, `A-PLAT-10`).
Autostart at login relies on the graphical session
exporting `DISPLAY`/`XAUTHORITY` into the systemd user manager; importing them
when autostart is enabled covers the current manager, but a session that never
exports them will start the daemon without an adapter and report `unavailable`.
The adapter covers application windows only; sites and keywords still need the
browser extension, and launch/pickup limits are not tracked.

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
  range), a searchable installed-app chooser that shows display names, app/site
  rules with limits, keywords, allowlists, and pickup limits. Raw application
  IDs remain available under “Advanced” for apps without a desktop entry.
- **Schedules** — schedule summary and active/waiting/next-start state.
- **Strict session** — start, visible commitment, request early exit, cool-down
  countdown, and end when the cool-down is ready.
- **Activity** — active quick blocks and recorded block decisions.
- **Settings** — start of day, IANA timezone, desktop integration install and
  status, policy path, capability status, data path, copy path, open folder, a
  Help group that re-runs onboarding, and About dialog.

The front end refreshes from the service every five seconds and after each
mutation. It shows “Enforcement active” only when an adapter heartbeat is fresh
and reports a loaded policy; otherwise it shows “Enforcement offline” and
explains which adapter is missing. The sidebar and enforcement card name the
reporting adapter (X11 or GNOME).

### First-run onboarding

`crates/ui/stayproductive/onboarding.py` implements `F-UX-01` as an
`Adw.Dialog` with five steps: welcome and purpose, goal, capabilities, review,
and done. Goal logic is pure: `Goal` definitions, alias-based app matching,
`apps_for_goal`, and `starter_profile` are testable without a display.

- Goals preselect at most `MAX_STARTER_APPS` (6) installed applications in hint
  order. The selection is editable in step 4; applications are shown by display
  name and raw IDs stay out of the main flow.
- Step 3 lists foreground app visibility (with the real adapter state), offers
  “Set up” for autostart when systemd can manage it, and continues to name
  notifications and the DNS helper as unavailable. Autostart is only enabled by
  that explicit button; nothing is granted by clicking through the wizard.
- `StarterProfileCreator` creates at most one profile per wizard run and writes
  `onboarding.json` next to `state.json` only after the daemon accepts the
  profile. A rejected create leaves the guard open so the user can fix the name
  and retry.
- The wizard auto-opens only when there is no commitment, no profile, and no
  marker. Settings → Help re-runs it without resetting anything.
- The dashboard's missing-enforcement banner has a session-scoped dismiss
  button, as `ux-flows.md` §2 requires; the banner returns next launch.

Known limits: notifications and the DNS helper cannot be granted from the
capability step because those integrations do not exist yet, so `F-UX-01` stays
`wip`. Autostart is real, but it is offered only for the default data file and a
running systemd user manager. The wizard's GTK flow is smoke-tested under Xvfb
(construction, step navigation, app add/remove, profile creation), but not
automated in CI as a UI test.

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

`crates/daemon/tests/protocol.rs` drives the real service binary over stdio and
over the Unix socket. It covers locked profile mutations, rejected pause, refused
early finish, restart persistence, the private state-file mode, socket privacy
and single-instance behaviour, and a policy refreshed with no client.

### What is still inert

The engine stores and explains site daily limits, but does not yet decrement
them because there is no per-site interval tracker (the browser extension must
first report site visits). Launch limits are enforced by the engine once
launches are counted, but the extension does not yet report background
launches, only foreground entries; pickup limits likewise need the
logind/tracker adapter. Application daily and hourly limits are now enforced
end-to-end through the GNOME extension or the X11 adapter observer. The UI
labels the rest as configuration rather than claiming they are enforced. The
browser extension, other compositors, and launch/pickup tracking remain the
largest missing pieces.

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
- **Desktop enforcement beyond X11 and GNOME** — the supervised X11 adapter and
  the GNOME extension cover application windows only; sway/i3, Hyprland, KWin,
  website, keyword, and site-limit enforcement still need adapters. The UI
  labels the adapter and its scope.
- **D-Bus name and tray/CLI** — the boundary is newline JSON over a Unix socket;
  systemd can own the daemon, but there is no `org.stayproductive.Daemon1` name,
  tray, or CLI yet.
- **Usage statistics views, notifications, backup/restore, localisation catalogs,
  full accessibility audit**. Export and history deletion exist (see *Data export
  and deletion*); the Week/Month totals, timeline, and seven-day report do not.
- **Low-time alerts** (`A-LIM-07` / `A-PLAT-11`).
- **Optional onboarding capabilities** — notifications and the DNS helper are
  explained in the wizard but not implemented. Autostart is implemented as the
  systemd user unit above.

## Test commands

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
gjs -m integrations/gnome/tests/matching.test.js
PYTHONPATH=crates/ui python3 -m unittest discover -s crates/ui/tests
gnome-extensions pack integrations/gnome/stayproductive@hassanraza.ai --out-dir /tmp
cargo build --release -p stayproductive-daemon
./stayproductive
```

The Python suite (`crates/ui/tests`) covers installed-app discovery and the
onboarding goal, alias matching, starter-profile construction, marker handling,
and the one-profile-per-run guard. It needs no display. The GTK wizard is
additionally construction-smoke-tested under Xvfb; that check is manual, not
part of the suite.
