# Blocking engine

The engine turns observations into a **decision**, then the daemon enforces it.
It is the only place that decides whether something is blocked.

## 1. Pipeline

```
observers ──► normalized Event ──► debounce/dedupe ──► resolve rules
                                                          │
                                     ┌────────────────────┴──────────────┐
                                     │ pure decision(state, event, now) │
                                     └────────────────────┬──────────────┘
                                                          ▼
                                       enforce (adapter) + log (reason)
```

- **Observers** are thin adapters (compositor, extension, logind, timer wheel).
  They report facts; they never decide.
- **Normalization** produces one event type with stable keys:
  `AppKey`, `Host`, `WindowKey`, `EventSource`.
- **Debounce**: at most one decision per `(app, surface)` per 250 ms. Trailing
  edge wins, so a burst of focus events produces one decision. The reference
  value is 250 ms; it is configurable but never below 100 ms.
- **Dedupe**: an unchanged `(verdict, reason, until)` is not re-enforced or
  re-logged.
- Decisions are **idempotent**: enforcing an already-active block is a no-op.

## 2. Events

| Event | Fields | Emitted by |
| --- | --- | --- |
| `ForegroundChanged` | app, window, surface hints | compositor adapter |
| `UrlChanged` | browser, url, title | extension |
| `SurfaceChanged` | app, surface id, visible | extension / adapter |
| `Tick` | now | timer wheel (1 s while a window/timer is active) |
| `ScreenLocked` / `ScreenUnlocked` | — | logind |
| `SessionSuspended` / `Resumed` | — | logind |
| `SettingsChanged` | changed keys | core |
| `CommitmentChanged` | id, state | core |
| `DaemonStarted` | now | daemon |
| `ClockJumped` | old, new | clock watcher |

`DaemonStarted` and `ClockJumped` force a full re-evaluation of the active
window and all limits.

## 3. Decision function

```rust
fn decide(state: &State, ev: &Event, now: Instant) -> Decision

struct Decision {
    verdict: Verdict,        // Allow | Block | BlockUntil | AllowWithNotice
    reason: ReasonCode,
    rule: Option<RuleId>,
    until: Option<Instant>,  // known end of the current block/pause
    message: MessageKey,     // for the UI
}
```

Rules are evaluated in fixed order; the first terminal verdict wins:

| # | Check | Verdict | Reason |
| --- | --- | --- | --- |
| 1 | Active `Commitment` forbids the app/site | Block | `R-STRICT` |
| 2 | Profile marked `strict_eligible` and its window is active | Block | `R-STRICT` |
| 3 | App/site is in a **commitment** allowlist | Allow | `R-ALLOW` |
| 4 | Profile is paused (user pause or break) | Allow | `R-PAUSE` |
| 5 | App/site is in the profile allowlist | Allow | `R-ALLOW` |
| 6 | Quick block covers the app/site | Block until | `R-QUICK` |
| 7 | Wait timer for the app is running | Block until | `R-WAIT` |
| 8 | Daily or hourly usage limit exhausted | Block until period end | `R-LIMIT-*` |
| 9 | Launch/pickup count limit exhausted | Block until period end | `R-LAUNCH`/`R-PICKUP` |
| 10 | Schedule window of a profile with a matching rule is active | Block | `R-PROFILE` |
| 11 | Keyword rule matches the URL/search text | Block | `R-KEYWORD` |
| 12 | Site rule matches | Block | `R-SITE` |
| 13 | Content-surface rule matches | Block | `R-SURFACE` |
| 14 | Nothing matches | Allow | `R-NO-RULE` |

Notes:

- Steps 1–2 are commitments and cannot be overridden by anything, including
  allowlists, pauses, quick allows, or an expired limit. This is the point of
  strict mode.
- `AllowWithNotice` is used for goal-based access: the app opens but shows
  remaining earned time.
- Two profiles blocking the same app merge to the **earlier** end time for the
  UI and the **stricter** reason for logging (`R-STRICT` > `R-PROFILE`).
- A rule for an app that is not installed is retained but inert; it never
  produces a block or an error.

## 4. Limits and counters

Counters are queries over `usage_events`, cached per period:

```
usage(app, period)      = Σ foreground_duration(app) within period
launches(app, period)   = count(ForegroundChanged → app) within period
pickups(period)         = count(ScreenUnlocked) within period
```

- Foreground intervals are closed on `ForegroundChanged`, screen lock, suspend,
  and daemon shutdown. An interval open across a crash is closed at the last
  heartbeat (≤ 5 s of undercount, never overcount).
- A limit is exhausted when `usage >= limit`. Exactly-at-limit blocks.
- **Hourly limit** resets at `:00`; the block lasts until the next hour.
- **Daily limit** resets at the next `start_of_day` boundary; the block lasts
  until then.
- **Launch limit** counts the first foreground of an app per period; switching
  back within 5 s is not a new launch (anti-thrash). This 5 s window is a
  StayProductive decision, not a measured reference value.
- **Goal rule**: time in `earn_for` accumulates only while that app is
  foreground and the target app is blocked; earned time is consumed while the
  target is used. Unused credit expires at period end.
- **Wait timer**: after `use_for` within a period, access is denied for `wait`.
  With `adaptive`, if the app was not used for the first half of the wait, the
  remaining wait is halved once. (`adaptive` semantics are **inferred** from the
  reference description; this definition is normative here.)

## 5. Allowlists

- App allowlists accept an `AppKey`; site allowlists accept a `HostPattern`.
- An allowlist entry **does not** override commitments (step 1–2) and does not
  override the strict-eligible profile (step 2).
- Allowing a site does not allow its subresources from other hosts; the
  extension and resolver evaluate each request host.
- Removing an allowlist entry re-evaluates the current foreground app
  immediately.

## 6. Pause, break, extend

- Pausing a profile suppresses rules 6–13 for that profile only. Limits keep
  accumulating unless the pause explicitly says otherwise (reference behaviour:
  usage continues to be tracked while paused — **derived**).
- A break is a pause with a mandatory end time and an optional allowlist. During
  a break the lock screen shows the countdown; the tray shows the remaining
  time.
- During a commitment, pause/break controls are disabled (the UI shows why).
- **Extend** moves the end of the current profile block by the chosen duration.
  Extending a commitment is allowed only by the commitment's configured method,
  never by a plain button.
- If a pause ends while the machine is off, rules apply immediately on the next
  start.

## 7. Counters across restarts, clock changes, suspend

| Situation | Required behaviour |
| --- | --- |
| Daemon restart | recompute open intervals from the last heartbeat; keep limits |
| UI crash | daemon unaffected; no state loss |
| Clock moved backwards | do not re-open elapsed periods; keep active commitments |
| Clock moved forwards | close intervals at the new time; evaluate new windows |
| DST start/end | periods follow local dates; no double count |
| Suspend | wall-clock commitment expiry continues; cool-down/wait pause |
| Screen locked | foreground tracking stops; pickups counted on unlock |
| Multi-monitor | one foreground app across the session; overlay on every monitor |

## 8. Enforcement

| Verdict target | Action |
| --- | --- |
| App | focus the lock overlay on all monitors; ask the compositor to hide/minimise the blocked window; optionally raise the overlay above full-screen windows |
| Site (DNS mode) | sinkhole to a local block page; the extension shows the reason |
| Site (extension mode) | replace the page with an interstitial; never silently fail |
| Keyword | same as site, with the matched keyword shown |
| Surface | overlay the surface inside the page (extension) |
| Commitment | lock overlay cannot be dismissed without the configured method |

Rules:

1. **Never destroy user data.** Do not kill processes or force-quit apps; hide
   or cover them. Killing is an explicit opt-in setting, never a default.
2. **Always offer an exit that matches the commitment.** A normal block can be
   dismissed after the configured pause; a commitment cannot.
3. **Explain.** The overlay shows the reason, the rule name, and the end time
   when known.
4. **Degrade loudly.** If an adapter cannot enforce (unsupported compositor),
   the daemon reports `Degraded` and the UI says plainly what is not enforced;
   it does not pretend to block.
5. **No block while locked.** A screen-locked session does not produce block
   overlays; enforcement resumes on unlock.

## 9. Notifications and alerts

- Low-time alerts fire once per threshold per period (10, 5, 1 minute by
  default, configurable).
- Limit-exhausted and block-start notifications are rate-limited to one per
  app per 5 minutes.
- Notifications never contain URLs or window titles unless the user opts in.

## 10. Logging and privacy

- Every block writes one `audit_log` row: time, rule id, reason code, target
  key, decision end. No window titles by default.
- Logs are queryable by the UI (history) and exportable; retention is 90 days
  for raw events and indefinite for daily aggregates.
- Debug logging is off by default, opt-in, and redacts hosts to their registrable
  domain.

## 11. Testability

The engine must run without a desktop:

```rust
let mut s = State::fixture();
let d = decide(&s, &Event::foreground("firefox"), t(9, 30));
assert_eq!(d.reason, ReasonCode::Profile);
```

Requirements:

- injectable clock (`Clock` trait), no `SystemTime::now()` in decision code;
- scripted event streams replayed against golden decision tables;
- property tests for period math (DST, start-of-day, leap seconds ignored);
- fuzz tests for malformed adapter events (unknown app, empty host, huge URL);
- one integration test per adapter using a recorded event trace.

## 12. Defaults (normative)

| Setting | Default | Source |
| --- | --- | --- |
| Debounce per app/surface | 250 ms | observed in reference |
| Heartbeat interval | 5 s | StayProductive decision |
| Block overlay raise delay | 120 ms | StayProductive decision |
| Low-time thresholds | 10 / 5 / 1 min | observed vocabulary |
| Launch anti-thrash window | 5 s | StayProductive decision |
| Raw event retention | 90 days | StayProductive decision |
| Notification rate limit | 1 / app / 5 min | StayProductive decision |
