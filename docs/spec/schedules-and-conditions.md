# Schedules and conditions

Defines when a profile's rules are active, how pauses interact, and how the
period calculator behaves. Implementation lives in `core::schedule` and
`core::period`.

## 1. Schedule kinds

Every schedule compiles to a set of local `(date, window)` pairs for a given
period; the engine asks "is `now` inside any window?" and "when does the current
window end?".

| Kind | Meaning | Example |
| --- | --- | --- |
| `Daily` | same window every day | every day 22:00–06:00 |
| `Days` | chosen weekdays, one window set | Mon–Fri 08:00–17:00 |
| `Intervals` | multiple windows in a day | 09:00–12:00 and 14:00–18:00 |
| `FullDay` | entire local day | Sundays, all day |
| `Hourly` | repeating within-hour window | 50 minutes on, 10 off |
| `OneTime` | dated range | 2026-11-03 09:00 → 2026-11-03 17:00 |

`Custom` in the UI is a presentation of `Days` with per-day windows or a
`OneTime` range; it is not a separate storage kind.

## 2. Start-of-day

- Default `00:00`; configurable per profile, with a global default in settings.
- The **logical date** of an instant is `date(now - start_of_day)`; a 04:00
  start means 02:00 on Tuesday belongs to Monday.
- Daily limits, daily launch counts, and daily reports all use the logical date.
- Changing start-of-day never retroactively rewrites stored events; it changes
  period boundaries from the next boundary onward and is logged.

## 3. Window semantics

- Half-open `[start, end)`: 09:00–17:00 includes 09:00:00 and excludes
  17:00:00.
- `end <= start` crosses midnight and belongs to the day it starts (e.g.
  Fri 22:00–06:00 is a Friday window).
- Windows may overlap; they merge.
- Adjacent windows (`…–12:00`, `12:00–…`) are treated as continuous.
- An empty list is invalid; `FullDay` is `00:00–24:00`.
- Resolution is one second; the scheduler re-evaluates on `Tick` while a
  boundary is within 60 s, otherwise on the next boundary.

## 4. Activation composition

Profiles have `activation ∈ {Manual, Scheduled, Always}`.

- `Manual`: only while the user activates it; exclusive with other manual
  profiles.
- `Scheduled`: active whenever its schedule says so; several may overlap.
- `Always`: active until manually disabled (used by "Always block" profiles).

Composition rules:

1. All active profiles contribute rules; the engine merges them (§ blocking
   engine).
2. Pausing one profile does not pause another.
3. A strict-eligible profile contributes at `R-STRICT` even if another profile
   is paused.
4. Editing a schedule takes effect immediately for future windows; an
   already-started block keeps its original end unless extended.
5. Removing a profile's schedule while active ends the block at the next
   boundary or immediately, as chosen in the edit dialog (default: immediately,
   with a confirmation).

## 5. Pause, break, extend

| Action | Effect | Constraints |
| --- | --- | --- |
| Pause with duration | profile rules suppressed until `now + d` | not during a commitment for the same profile |
| Pause indefinitely | suppressed until manual resume | requires an explicit toggle; UI shows "Paused" |
| Break | pause + visible countdown + optional allowlist | 1 min … 1 week; commitment blocks it |
| Extend | move the current block's end by `d` | only forward; commitments unaffected |
| Resume | clear pause and re-evaluate immediately | always available when not committed |

Durations offered in the reference UI: 1, 5, 10, 15, 30 minutes, 1 hour, and
"today". Break durations: 30 min, 1/4 hours, 1/2/5/7/30 days. StayProductive
accepts any duration from 1 minute to 30 days in the custom picker and offers
the same presets.

Pauses and breaks are persisted with both wall-clock and monotonic anchors so
that suspend does not consume them (except `paused_until` for "today", which is
wall-clock by definition).

## 6. Activation conditions beyond time

- **Manual** — dashboard toggle, tray, CLI (`stayproductive profile activate`).
- **Location** — optional; GeoClue2 via the desktop portal. Fail-closed option:
  if location is unavailable, the profile stays active (the reference exposes
  this as "Block if location is unavailable"). Requires explicit consent; no
  coordinates leave the machine.
- **Schedule** — as above.
- **Always** — equivalent to a full-week schedule with no end.

Location activation is P2 and must never be required for core blocking.

## 7. Templates

Bundled templates only pre-fill an editable profile; they are not special
runtime types. Minimum set (names are ours, behaviours map to the reference):

| Template | Shape |
| --- | --- |
| Focus Hours | weekdays 08:00–17:00, social apps blocked |
| Deep Work | 90-minute Pomodoro-friendly block, work apps allowed |
| Sleep | every day 22:00–06:00, all apps except phone/alarm |
| Digital Detox | Sunday full day, only essentials |
| Exam Prep | weekdays 16:00–21:00, study apps allowed |
| Social Limit | 30 min/day across social apps |
| Gaming Limit | 45 min/day across games |
| Phone Limit | 90 min/day total with essentials allowlisted |
| Meal Time | 12:00–13:00 daily |
| Bedtime Reading | 21:00–22:00, reading apps allowed |

Creating from a template copies rules into a normal profile; later template
updates do not mutate existing profiles.

## 8. Schedule validation

Reject with a field-level message when:

- start == end inside a single window (unless the user selected full day);
- a day has zero windows while "custom days" is selected;
- a one-time range ends in the past;
- a window exceeds 24 h or has sub-minute precision;
- two windows for the same day overlap in a way the UI cannot represent
  (merge automatically instead);
- start-of-day equals a window boundary in a way that makes the day ambiguous
  (warn, do not reject).

## 9. Time changes

- **Timezone change**: windows follow wall time in the new zone. An active block
  keeps its absolute end; the UI explains the shift.
- **DST spring forward**: a window that falls entirely in the skipped hour does
  not run that day; a window straddling the gap runs from the new local time.
- **DST fall back**: a repeated hour is evaluated once; usage in it is counted
  once.
- **Manual clock change**: recompute windows; do not shorten commitments;
  log `ClockJumped`.

## 10. Persistence and evaluation

- Schedules are stored normalized (§ domain model), not as UI strings.
- On startup the daemon computes the next boundary and arms one timer; it does
  not poll every second when nothing is near.
- Every settings change invalidates compiled schedules; the next decision uses
  the new schedule.
- A missed boundary during suspend is handled on resume by evaluating the
  current state, not by replaying every intermediate second.

## 11. Test matrix (unit level)

| Case | Expected |
| --- | --- |
| 09:00–17:00, evaluate 08:59:59 / 09:00:00 / 16:59:59 / 17:00:00 | false / true / true / false |
| 22:00–06:00 on Friday, evaluate Saturday 02:00 | true (belongs to Friday) |
| start_of_day 04:00, evaluate Tuesday 02:00 | logical Monday |
| overlapping windows | merged, single block |
| DST spring gap window 02:00–03:00 | does not run |
| DST fall window 01:00–02:00 | runs once |
| timezone change while active | absolute end preserved |
| pause ends during suspend | active on resume |
