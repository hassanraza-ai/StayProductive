# Reference behaviour analysis

Target: **Stay Focused 13.0.2** (`com.stayfocused`, versionCode 656, minSdk 24),
analysed statically from a locally held APK.

| Property | Value |
| --- | --- |
| Method | static only — decompiled sources, decoded manifest/resources, no execution |
| Authority | shipped artifact |
| Classes | 8,503 in the app DEX; 7,807 under an obfuscated package |
| Resources | 1,719 entries, 8,866 named resources |
| Confidence labels | **observed** (read directly), **derived** (computed), **inferred** (hypothesis) |

This document is behaviour-focused: it describes what the product does, not how
its code is organised. No source, resources, or assets are reproduced here.
Everything below is **observed** unless marked otherwise; end-user runtime
behaviour was never measured, so timing and OS interactions that depend on the
Android framework are **derived**.

## 1. Product model

The app is a rule engine wrapped in an Android UI. A user creates **profiles**;
a profile contains any combination of:

- blocked apps (per-app daily/hourly limits or hard block),
- blocked websites,
- blocked keywords,
- allowlists (apps/sites that stay available),
- a schedule that decides when the profile is active,
- in-app surface filters (Reels/Shorts/Stories and similar),
- break/pause settings.

Only one profile is "active" at a time in the user-facing sense, but schedules
can activate and expire independently, and blocks compose: a strict-mode block
outranks a profile pause, a limit outranks a manual break, and an allowlist
never overrides strict mode.

*(Derived: the composition order follows from the settings surface and the
enforcement code paths; see §4 for the precedence implemented by this project.)*

## 2. Component surface (observed)

| Area | Components |
| --- | --- |
| Onboarding/home | splash, main dashboard, transparent trampoline |
| Profiles | create/edit profile, per-app detail, per-app usage, activation conditions |
| Blocking UX | lock activity, locked activity, blur lock overlay, break timer |
| Strict mode | setup, friend approval, QR capture, alarm ring activity/service, lock mode, cheat-code screen |
| Billing | three generations of premium screens (not reproduced; all features free here) |
| Account/sync | email auth, device registration, sync services (optional in this project) |
| Utilities | backup/restore, storage manager, feedback, about, help |
| Widgets | blocked-app widget, break widget, today-stats widget, quick-settings tile |
| Background | accessibility service, usage tracker, notification listener, boot receivers, alarm receivers, geofence receiver |

## 3. The blocking pipeline (observed)

The Android implementation is event-driven and deliberately decoupled:

```
platform event (window change / content change / focus)
        │
        ▼
accessibility service  ── accepts only 3 event types, copies into a queue
        │                 (never blocks the platform thread)
        ▼
worker thread
        │
        ├─ debounce: at most one decision per package per 250 ms
        ├─ resolve the active rule set for that package
        ├─ evaluate feature rules (view-ID + visibility)
        ├─ evaluate keyword rules against URL / search text
        └─ decide → enforce or allow
```

Properties worth preserving on Linux:

1. **Asynchronous evaluation.** Observation and decision are separated so the
   UI/compositor is never stalled.
2. **Debounce.** 250 ms per application is the reference value; it prevents
   thrash when a screen redraws. Treat it as the default, not a law.
3. **Surface rules are visibility-based.** A rule matches a specific surface
   (e.g. a full-screen short-video player), not the whole application. This is
   why the product can block Reels while leaving messages usable.
4. **Text rules read the address bar / search field.** Keyword blocking covers
   typed or navigated text, not arbitrary screen content.
5. **Bounded traversal.** The reference caps tree walks at depth 60 and skips
   embedded web views; both are acknowledged blind spots, not features to copy.

## 4. Decision precedence used by this project

The reference exposes the ingredients but not a written priority table. The
following order is **derived** from its settings surface and enforcement paths,
and is normative for StayProductive (`docs/spec/blocking-engine.md`):

```
1. hard lock            (lock mode, strict mode with active commitment)
2. strict profile rule  (profile flagged "add to strict mode")
3. allowlist            (app/site explicitly allowed — but never above 1)
4. active schedule block
5. exhausted limit      (daily/hourly usage, launches, pickups)
6. keyword / site rule
7. content-surface rule (Reels, Shorts, …)
8. default allow
```

Ties are resolved by the more specific rule, then by the stricter outcome. Every
decision carries a reason code so the UI can explain it.

## 5. Blocks, limits and counters (observed)

| Control | Semantics |
| --- | --- |
| Hard block | app/site unavailable while the owning profile is active |
| Daily usage limit | accumulate foreground time per local day; block when exceeded |
| Hourly usage limit | accumulate within a rolling/clock hour; block when exceeded |
| Launch limit | count foreground launches per day or per hour |
| Pickup/unlock limit | count device unlocks per day or per hour |
| Goal-based access | unlock a blocked app after spending time in a chosen "productive" app |
| Wait timer | after N minutes of use, force a cooldown before access resumes |
| Adaptive timer | reduce the cooldown if the app was not used during it |
| Break / pause | time-boxed suspension of a profile (1, 5, 10, 15, 30 min, 1 h, today) |
| Extend | lengthen an active block without editing the profile |
| Quick block | ad-hoc block for the next N minutes |

Counters are **derived from usage events**, not from app-side timers: a crash or
restart must not reset a limit. Day boundaries follow the user's configured
"start of day", not midnight (observed setting).

## 6. Schedules and activation (observed)

The schedule editor offers, at minimum:

- **Daily** — a time window repeated every day.
- **Specific days** — chosen weekdays, optionally different windows per day.
- **Hourly** — repeating within-hour pattern (used with hourly limits).
- **Time intervals** — multiple windows in a day.
- **Full day** — 24 hours.
- **Custom / one-time** — a dated window.
- **Start of day** — offset that shifts when the daily counter resets.

Activation conditions beyond time: a location ("whenever I am on location"),
and manual activation from the dashboard or widget. Profiles can be paused,
resumed, or extended, and a pause can be indefinite when the user opts out of
choosing a duration.

## 7. Strict mode and lock mode (observed)

Strict mode is a **commitment device**, not a stronger block. At setup the user
chooses:

- an **end condition**: expiry time, an active schedule, or manual;
- a **deactivation method**: cool-down delay, random text, QR code, friend
  approval;
- **extra restrictions**: block settings, block uninstall, block profile
  editing, block battery/power settings, block recent apps, block split screen;
- optional **password** to open the app while strict mode runs.

Lock mode is a lighter variant: a password gates the app, settings, and profile
changes, with a normal-mode path for intentional exit.

Deactivation flows have distinct states — idle, requested, cooling down, ready,
completed — and the UI must survive leaving and returning to the app. Friend
approval is asynchronous and retries while the server is unreachable.

## 8. Content filtering (observed)

Two independent mechanisms:

1. **Known surfaces.** A shipped catalog maps an app + view identity to a
   feature label (YouTube Shorts, Instagram Reels, Instagram Stories, Snapchat
   Spotlight/Stories, WhatsApp Status/Channels, Facebook Reels, business
   WhatsApp variants). Detection requires the surface to be visible.
2. **Keywords.** A user list matched case-insensitively against browser address
   bars and in-app search fields; a match sends the user home while the owning
   schedule is active.

Adult-content filtering is a separate opt-in that blocks known domains in
supported browsers. Unsupported browsers can themselves be blocked, because
their content cannot be inspected.

## 9. Guard rails as product behaviour (observed)

The reference tries to prevent impulsive bypassing by:

- making its own settings and device-admin screens hard to reach while locked;
- resisting power-menu and recents-based escapes;
- persisting across reboot;
- detecting uninstall/reinstall;
- enforcing exact-time alarms for scheduled commitments.

These are Android-specific tactics. StayProductive keeps the **intent** —
deactivation requires deliberate effort, and scheduled commitments survive
restart — and implements it with Linux-appropriate mechanisms (see
`docs/architecture/linux-design.md` §6). It does **not** attempt to become
unremovable.

## 10. Data model (observed/inferred)

Persisted state includes profiles, apps, sites, keywords, schedules, usage
events, pause/break state, strict-mode sessions, saved QR codes, alarms,
settings, and cached catalog data. The reference stores this behind a single
content provider and mirrors some of it to a backend for account sync.

StayProductive stores the same domain concepts locally in SQLite. Sync and
account features are optional and out of P0 scope.

## 11. Coverage and limitations

- Static analysis only; no runtime traces, network captures, or purchase flows.
- 92% of classes are obfuscated, so some behaviours are inferred from the UI
  surface, resources, and call paths.
- Twenty background entry points are protected by a bytecode VM; their bodies
  are not recoverable statically. Their **purpose** is known from component
  names and surrounding code; exact timing is not.
- Android-specific enforcement (accessibility, device admin, exact alarms) has
  no one-to-one Linux equivalent. The mapping is defined in
  `docs/architecture/linux-design.md` and is part of the product decision, not
  an implementation detail.

## 12. Where each behaviour is specified

| Topic | Document |
| --- | --- |
| Entities and invariants | `docs/spec/domain-model.md` |
| Decision order, counters, timers | `docs/spec/blocking-engine.md` |
| Schedule grammar, conditions, pause | `docs/spec/schedules-and-conditions.md` |
| Strict/lock commitments | `docs/spec/strict-and-lock.md` |
| Sites, keywords, surfaces | `docs/spec/content-filtering.md` |
| Screens and flows | `docs/spec/ux-flows.md` |
| Linux mechanisms | `docs/architecture/linux-design.md` |
| Pass/fail scenarios | `docs/acceptance/parity-tests.md` |
