# Domain model

Normative model for the core crate. Names here are the Rust types; JSON/YAML
examples use the same names in `snake_case`. All timestamps are UTC instants;
all "local day" calculations go through the period calculator in §5.

## 1. Principles

1. **Local-first.** Everything a user creates lives in a local SQLite database.
   Optional services may read it, never own it.
2. **Events are the source of truth for usage.** Blocks are derived from events
   and rules; a timer never decrements a stored counter.
3. **Rules are declarative.** Enforcement is a pure function of state, event,
   and `now`. No hidden grace periods inside components.
4. **Identity is stable.** Every entity has a ULID; renames never change
   identity. Deleting a profile does not delete its history.
5. **Commitments are durable.** Strict/lock state is written before it takes
   effect and cannot be cleared by killing the UI or daemon.

## 2. Aggregate overview

```
Settings ──┐
           ├── Profile ── RuleSet ── AppRule / SiteRule / KeywordRule / AllowRule
Catalog ───┘        │
                    └── Schedule ── Window / DayPattern / DateRange
                              │
Commitment ── Challenge ── Approval
     │
UsageEvent ── Session ── PomodoroRun
```

`Catalog` caches installed applications and known browser/site metadata. It is
disposable: losing it must not change a decision (rules reference stable keys,
not display names).

## 3. Profile

```rust
struct Profile {
    id: Ulid,
    name: String,                 // 1..=64 grapheme clusters, unique case-insensitively
    color: Option<Rgb>,
    icon: Option<IconRef>,
    enabled: bool,
    priority: u8,                 // 0..=9, higher wins when two blocks overlap
    rules: RuleSet,
    schedule: Option<Schedule>,
    activation: Activation,       // Manual | Scheduled | Always
    pause: PauseState,
    strict_eligible: bool,        // included in strict-mode profile selection
    template: Option<TemplateId>,
    created_at: Instant,
    updated_at: Instant,
}
```

Invariants:

- A profile with `activation = Scheduled` must have a schedule.
- `name` is unique among non-deleted profiles.
- Deleting a profile cascades to its rules and schedule but not to usage events
  or commitment history.
- At most one profile may be **manually activated**; scheduled profiles can
  overlap and compose (§4).

## 4. Rules

```rust
struct RuleSet {
    apps: Vec<AppRule>,
    sites: Vec<SiteRule>,
    keywords: Vec<KeywordRule>,
    allow_apps: Vec<AppKey>,
    allow_sites: Vec<HostPattern>,
}

enum AppRule {
    Block,
    DailyLimit { limit: Duration },
    HourlyLimit { limit: Duration },
    LaunchLimit { count: u32, per: Period },
    WaitTimer { use_for: Duration, wait: Duration, adaptive: bool },
    Goal { earn_for: AppKey, required: Duration },
}

struct SiteRule {
    pattern: HostPattern,          // exact host or *.suffix
    action: SiteAction,            // Block | Limit(Duration) | Adult
}

struct KeywordRule {
    text: String,                  // matched case-insensitively, Unicode-folded
    scope: KeywordScope,           // Url | Search | Both
}
```

Rules reference `AppKey` (desktop entry ID plus executable identity) and
`HostPattern`; display names are derived. Duplicate rules merge with the
stricter outcome; the UI shows one row per key.

## 5. Time and periods

```rust
struct ClockSettings {
    start_of_day: NaiveTime,       // default 00:00
    timezone: Tz,                  // IANA, follows system changes
}
```

- **Daily period**: `[local_date at start_of_day, +1 day)` in the user's time
  zone. A start-of-day of 04:00 means "Saturday" runs 04:00 Sat → 04:00 Sun.
- **Hourly period**: the wall-clock hour; if start-of-day is non-midnight, hours
  still align to `:00`.
- **Elapsed usage** uses a monotonic clock; **rule evaluation** uses wall time.
- DST: a day may be 23 or 25 hours. Periods are computed from local dates, never
  by adding 86,400 s. A repeated hour is counted once; a skipped hour counts as
  zero.
- Clock jumps: if wall time moves backwards by more than 2 minutes, the engine
  recomputes active windows and **never** shortens an active commitment.

## 6. Schedule

```rust
struct Schedule {
    kind: ScheduleKind,
    windows: Vec<Window>,          // local time ranges
    days: DayPattern,              // EveryDay | Weekdays(set) | Custom(map<Weekday, Vec<Window>>)
    date_range: Option<DateRange>, // for one-time/custom
    start_of_day: Option<NaiveTime>, // override
}

enum ScheduleKind { Daily, Days, Intervals, FullDay, Hourly, OneTime }
```

`Window` is half-open `[start, end)`; `end <= start` means it crosses midnight
and belongs to the day it starts. Overlapping windows are allowed and merge.
An empty window list is invalid. Validation errors are surfaced before saving.

## 7. Activation, pause, break, extend

```rust
enum Activation { Manual, Scheduled, Always }

struct PauseState {
    paused_until: Option<Instant>, // None + paused=true => indefinite
    paused: bool,
    reason: PauseReason,           // User | Break | StrictCooldown | System
}

struct BreakSession {
    id: Ulid,
    scope: BreakScope,             // Profile(id) | Global
    starts_at: Instant,
    ends_at: Instant,
    allow: AllowSnapshot,          // optional allowlist during the break
}
```

- `paused` suppresses the profile's **rules** but not commitments (§ strict).
- A break is a pause with a visible countdown and optional allowlist.
- Extend moves an active block's end time forward; it never shortens.
- All three are logged as events so statistics can distinguish blocked time
  from allowed time.

## 8. Commitments (strict and lock)

```rust
struct Commitment {
    id: Ulid,
    kind: CommitmentKind,          // Strict | Lock
    state: CommitmentState,        // Armed | Active | CoolingDown | ReadyToEnd | Ended
    starts_at: Instant,
    ends_at: Option<Instant>,      // None = until schedule/manual
    schedule: Option<ScheduleId>,
    method: DeactivationMethod,    // Cooldown(Duration) | Text | Qr(QrId) | Approval(FriendId)
    restrictions: RestrictionSet,  // BlockSettings | BlockEdits | PasswordGate | ...
    password_hash: Option<Argon2Hash>,
    challenge: Option<Challenge>,
    created_at: Instant,
    history: Vec<CommitmentEvent>,
}
```

Invariants:

- `Armed → Active` is the only transition that may add restrictions.
- Restrictions can never be removed while `state ∈ {Active, CoolingDown}`.
- `ReadyToEnd → Ended` requires the configured method to have been satisfied
  within its validity window.
- A commitment survives daemon restart and OS reboot; time spent powered off
  counts toward wall-clock expiry but not toward cool-down timers (see §9).

## 9. Timers across suspend

- **Commitment expiry** is wall-clock: if the machine was off, an expired
  commitment is ended on next start.
- **Cool-down and wait timers** are monotonic: suspend pauses them. This
  prevents "close the lid" from satisfying a cool-down.
- Any timer persists `(started_at_wall, duration, elapsed_monotonic_at_pause)`
  so a restart can resume without granting or losing time incorrectly.

## 10. Usage events

```rust
struct UsageEvent {
    id: u64,
    at: Instant,                   // UTC
    kind: UsageKind,               // Foreground | Background | Launch | Pickup |
                                   // SiteVisit | Blocked | Paused | Resumed
    app: Option<AppKey>,
    site: Option<Host>,
    window: Option<WindowKey>,     // ephemeral, not exported by default
    reason: Option<ReasonCode>,    // for Blocked/Paused
    source: EventSource,           // X11 | Sway | Hyprland | Extension | Manual
}
```

Events are append-only and compacted after 90 days into hourly aggregates.
Export defaults to aggregates plus block reasons, never window titles.

## 11. Reason codes

Every decision carries one:

| Code | Meaning |
| --- | --- |
| `R-STRICT` | active commitment or strict profile |
| `R-PROFILE` | profile window is active |
| `R-LIMIT-DAILY` / `R-LIMIT-HOURLY` | usage limit exhausted |
| `R-LAUNCH` / `R-PICKUP` | count limit exhausted |
| `R-WAIT` | wait timer running |
| `R-KEYWORD` / `R-SITE` | keyword or site rule |
| `R-SURFACE` | content surface rule |
| `R-QUICK` | ad-hoc quick block |
| `R-ALLOW` | allowlist override |
| `R-PAUSE` | paused by user/break |
| `R-NO-RULE` | default allow |

The UI renders these into sentences; codes are stable for tests and logs.

## 12. Persistence

SQLite (WAL) with tables: `profiles`, `rules`, `schedules`, `commitments`,
`challenges`, `saved_qr`, `usage_events`, `usage_hourly`, `pauses`, `breaks`,
`settings`, `schema_version`, `audit_log`.

- Foreign keys on; deletes are soft where history matters.
- Migrations are numbered, forward-only, and run in a transaction with a backup
  copy of the database file.
- Writes that change enforcement (commitments, pause, limit overrides) are
  committed **before** the in-memory state is published.
- A checksummed `state.json` snapshot is written on clean shutdown; it is a
  recovery aid, not the source of truth.

## 13. Export format

A versioned archive (`.stayproductive.zip`):

```
manifest.json        {version, exported_at, app_version}
profiles.json        profiles + rules + schedules (no history)
settings.json
usage/YYYY-MM.json   aggregates and block reasons (opt-in)
commitments.json     current state + history
```

Import validates `version`, never overwrites an active commitment with a weaker
one, and refuses to import a commitment whose password hash it cannot verify.
