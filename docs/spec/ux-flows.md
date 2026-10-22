# UX flows

The app has two surfaces: a **main window** (GTK/libadwaita) and an **enforcement
overlay**. Everything below is the user-visible contract; visual design is free
within the libadwaita guidelines.

## 1. Principles

1. **Explain every block.** Reason, rule name, and end time in one sentence.
2. **One primary action per screen.** Secondary actions are text buttons.
3. **No dark patterns.** No pre-checked consent, no fake countdowns, no hidden
   exit. If an exit requires waiting, say so before it starts.
4. **Calm, not punitive.** Neutral language; no shaming, no streak loss for
   taking a break.
5. **Reversible by default.** Destructive actions confirm; commitments are the
   only irreversible states and are clearly labelled as such.

## 2. Onboarding (first run)

| Step | Content | Actions |
| --- | --- | --- |
| 1. Welcome | one sentence of purpose; "not a parental-control tool" note | Continue |
| 2. Goal | "Where do you want to focus first?" — Social, Work, Study, Sleep, Custom | pick one |
| 3. Permissions | list of required capabilities with why, per desktop: foreground app visibility, notifications, autostart; optional DNS helper | Grant / Skip optional |
| 4. First profile | pre-filled from the chosen goal; apps pre-selected but editable | Create |
| 5. Done | "You're set" + what happens next; no account required | Open dashboard |

Rules:

- Skipping optional capabilities is allowed; the dashboard shows a persistent
  but dismissible banner for missing enforcement.
- The onboarding never asks for an email, purchase, or analytics consent.
- Re-running onboarding is available from Settings → Help.

## 3. Dashboard

Top to bottom:

1. **Status card** — Active / Paused / Strict / Degraded, current profile name,
   next boundary ("ends in 1 h 20 m" or "starts at 18:00").
2. **Quick actions** — Pause (duration picker), Take a break, Quick block,
   Activate profile.
3. **Today** — screen time, top three distractions, blocks count; link to Stats.
4. **Profiles** — cards with name, schedule summary, enable switch; "New
   profile" and "From template".
5. **Footer** — daemon status and enforcement coverage ("X11: enforced",
   "Browser extension: connected").

Empty state: "No profile yet" with a create button and a template shortcut.

## 4. Profiles

List → detail → editor.

**Create**: name → template (optional) → apps → sites → keywords → limits →
schedule → advanced → save.

**Editor sections** (each independently editable, with a sticky Save):

| Section | Controls |
| --- | --- |
| Name/icon | text, colour, icon picker |
| Apps | searchable list with installed apps, toggles, "block all" / "allow only" |
| Sites | host entry with validation and suggestions; allowlist tab |
| Keywords | chips, comma-separated paste, scope selector |
| Limits | per-app daily/hourly, launches, wait timer |
| Schedule | schedule-type picker, day picker, time windows, start of day |
| Advanced | strict eligibility, surface filters, adult filter, notification blocking |

Interaction rules:

- Unsaved changes prompt on navigation.
- Deleting a profile confirms and explains that history is kept.
- Duplicating copies rules but not history or pause state.
- "Copy from profile" adds items without removing existing ones.
- A rule for an uninstalled app shows as "not installed" and can be removed.

## 5. Block and break screens

**Block screen** (full-screen overlay on every monitor):

```
        [app icon or site favicon]
        YouTube is blocked
        Daily limit reached (30 min)
        Resumes tomorrow at 08:00
        [Take a break]  [Back]
        quota: motivational quote (optional)
```

- Primary action leaves the app (`Back` → focus previous window/workspace).
- If a break is available, it is offered; if a commitment is active, the button
  is replaced by "Strict mode — ends at …" and no break is offered.
- The overlay is keyboard-navigable, screen-reader labelled, and dismissible
  only through its actions.
- Multi-monitor: the overlay covers all monitors; the focused monitor shows the
  card.
- A short grace (default 300 ms, configurable) hides the overlay if the user
  immediately switches away.

**Break screen**: countdown, end time, optional allowlist ("During this break:
everything else is blocked" / list), "End break early" only if the profile
allows it. During a commitment, ending early is impossible.

## 6. Strict mode and lock mode

- Setup wizard: end condition → method → restrictions → optional password →
  summary → explicit accept (§ `strict-and-lock.md`).
- Active state: dashboard card shows method and earliest end; "Disable" starts
  the configured flow, never an immediate disable.
- Cool-down: full-screen or modal with a live timer; can be left and resumed;
  "Cancel" keeps strict mode on.
- Challenge: text input with paste allowed; QR with camera/upload; approval with
  a status timeline and retry state.
- Lock mode: "Normal mode" button prompts for the password; wrong attempts back
  off visibly.

## 7. Stats

- Range selector: Today / Week / Month (default Today).
- Summary: total screen time, blocks, pickups, launches.
- Top distractions: app/site, time, share, trend vs previous period.
- Timeline: 24-hour or 7-day bars; per-app drill-down.
- Every chart has a table alternative and a text summary for screen readers.
- Export CSV/JSON and "delete history" live here, with confirmation.

## 8. Settings

Groups: General (start of day, language, theme, start at login), Enforcement
(adapters, DNS helper, browser extension), Notifications (low-time alerts,
block alerts), Privacy (history retention, export, delete), Advanced (CLI, daemon
socket, logs), About (version, licenses, credits).

- The "Enforcement" page is the single place that reports what is actually
  enforced and what is degraded, per desktop.
- No telemetry toggle exists because there is no telemetry.

## 9. Tray and notifications

- Tray icon reflects state: active, paused, strict, degraded.
- Menu: Open, Pause 15/30/60 min, Resume, Quick block 10/30 min, Take a break,
  Quit (quit is blocked during a commitment, with an explanation).
- Notifications: limit about to be reached, limit exhausted, pause ending,
  strict mode ending soon. Every notification has a "don't show again" path.

## 10. CLI

```
stayproductive status
stayproductive profile list|activate|pause|resume <name>
stayproductive block --for 25m
stayproductive break --for 10m
stayproductive strict status|request-end
stayproductive doctor            # adapter/enforcement diagnostics
```

The CLI is a client of the daemon; it never edits the database directly. It has
no command that bypasses a commitment.

## 11. States

| State | UI |
| --- | --- |
| No profiles | onboarding CTA + template shortcuts |
| Paused | status card + resume; rules visibly inactive |
| Strict | distinct accent, method and end time always visible |
| Degraded | warning banner naming the unsupported adapter; blocking that can work still works |
| Adapter disconnected | banner + tray badge; retry loop with backoff |
| Database error | blocking uses the last snapshot; a repair screen offers export and reset |
| Unsupported desktop | installation explains which adapters exist; the app still runs but says it cannot enforce app blocking |

## 12. Accessibility and localisation

- All controls reachable by keyboard; focus order follows visual order.
- Every icon-only button has an accessible label; charts have text summaries.
- Respect high-contrast and reduced-motion settings.
- Strings live in gettext catalogs; plural forms used for counts; no sentence
  concatenation.
- Minimum contrast 4.5:1 for text, 3:1 for large text and focus rings.
- The block overlay must not trap focus indefinitely without a visible exit.

## 13. Screen inventory

| ID | Screen | Primary action |
| --- | --- | --- |
| S-01 | Onboarding wizard | Continue |
| S-02 | Dashboard | Activate / Pause |
| S-03 | Profile list | New profile |
| S-04 | Profile editor | Save |
| S-05 | Schedule editor | Save |
| S-06 | App/site picker | Done |
| S-07 | Block overlay | Back |
| S-08 | Break overlay | End break (if allowed) |
| S-09 | Strict setup | Accept |
| S-10 | Strict deactivation | Continue method |
| S-11 | Stats | Change range |
| S-12 | Settings | — |
| S-13 | Diagnostics (`doctor`) | Copy report |
| S-14 | Backup/restore | Export / Import |
| S-15 | Tray menu | Resume / Pause |
