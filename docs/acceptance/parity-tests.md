# Acceptance tests

Executable scenarios for the parity matrix. Each `A-*` ID maps to a matrix row.
Tests run against the daemon with a **fake clock**, a **fake compositor**, and a
**fixture database**; adapter-specific runs substitute the recorded trace for
that desktop.

Conventions:

- `t("09:30")` is a local-time instant on a fixed test date; the clock is
  advanced explicitly.
- "blocked" means the engine returned `Block` **and** the adapter received the
  enforcement call with the same reason code.
- Every scenario is also run after a daemon restart unless stated otherwise.
- Failures must name the reason code and the rule that produced it.

## Profiles and rules

| ID | Scenario |
| --- | --- |
| A-PROF-01 | Given two profiles, when one is renamed and duplicated, then names are unique, the copy has the same rules, and history is not duplicated. |
| A-PROF-02 | Given a manual profile, when activated from UI, tray, and CLI in turn, then all three report the same active state. |
| A-PROF-03 | Given profile A active manually, when B is activated manually, then A is deactivated and exactly one manual profile remains active. |
| A-PROF-04 | Given each bundled template, when created, then the resulting profile is editable and its rules match the template table. |
| A-PROF-05 | Given profiles A and B, when "copy from A" is used in B, then items are added without removing B's existing items. |
| A-PROF-06 | Given a strict-eligible profile with an active schedule, when the daemon starts, then its rules produce `R-STRICT` even if another profile is paused. |
| A-RULE-01 | Given Firefox blocked, when it becomes foreground, then the overlay is shown within 250 ms and the reason is `R-PROFILE`. |
| A-RULE-02 | Given `example.com` blocked, when the extension navigates there, then the block card appears and the reason is `R-SITE`. |
| A-RULE-03 | Given keyword "game", when a URL contains it, then navigation is blocked before paint where observable. |
| A-RULE-04 | Given an app both blocked and allowlisted in a non-strict profile, when foreground, then it is allowed with `R-ALLOW`. |
| A-RULE-05 | Given `*.example.com` blocked and `good.example.com` allowed, then the subdomain loads and others do not. |
| A-RULE-06 | Given a goal rule requiring 30 min in Study, when 30 min elapse then 10 min of Social are granted, and credit is consumed while used. |
| A-RULE-07 | Given a wait timer of 5 min after 25 min use, when the threshold is reached, then access is denied until the wait ends. |
| A-RULE-08 | Given a 10-minute quick block, when it expires, then the app is allowed without editing the profile. |

## Limits and counters

| ID | Scenario |
| --- | --- |
| A-LIM-01 | Given a 30-minute daily limit, when exactly 30:00 foreground is recorded, then the app blocks until the next start-of-day. |
| A-LIM-02 | Given a 5-minute hourly limit, when 5:00 is recorded at 10:59, then it blocks until 11:00 and usage resets. |
| A-LIM-03 | Given a 2-launch/hour limit, when the app is focused twice inside 5 s, then it counts once; a third launch after 5 s is blocked. |
| A-LIM-04 | Given a 20-pickup/day limit, when the session unlocks 20 times, then the next unlock triggers the block reason `R-PICKUP`. |
| A-LIM-05 | Given 29:59 of usage, when the daemon is killed and restarted, then the counter resumes at 29:59 and the next second blocks. |
| A-LIM-06 | Given start-of-day 04:00, when usage occurs at 02:00 Tuesday, then it is counted against Monday. |
| A-LIM-07 | Given a 30-minute limit, when 20 minutes remain, then exactly one low-time notification fires per threshold. |

## Schedules and conditions

| ID | Scenario |
| --- | --- |
| A-SCH-01 | Given 09:00–17:00, then 08:59:59 allows, 09:00:00 blocks, 16:59:59 blocks, 17:00:00 allows. |
| A-SCH-02 | Given Mon–Fri 08:00–17:00, then Saturday is inactive and Monday is active. |
| A-SCH-03 | Given 09:00–12:00 and 14:00–18:00, then 12:30 allows and 13:59 allows, 14:00 blocks. |
| A-SCH-04 | Given a Sunday full-day schedule and a one-time window, then each is active exactly within its range. |
| A-SCH-05 | Given 50-on/10-off hourly, then the block returns at the top of each hour with the specified duty cycle. |
| A-SCH-06 | Given start-of-day 04:00, the daily reset occurs at 04:00 and not at midnight. |
| A-SCH-07 | Given an active profile, when paused for 15 minutes, then rules are suppressed and resume automatically at the deadline. |
| A-SCH-08 | Given indefinite pause, then rules stay suppressed across restart until resume. |
| A-SCH-09 | Given an active block ending at 17:00, when extended by 30 min, then it ends at 17:30 and cannot be shortened. |
| A-SCH-10 | Given an edited schedule, when saved, then the next decision uses it without a daemon restart. |
| A-SCH-11 | Given location activation with fail-closed, when GeoClue is unavailable, then the profile stays active and the reason is surfaced. |

## Strict and lock mode

| ID | Scenario |
| --- | --- |
| A-STR-01 | Given strict mode until 18:00, when the clock reaches 18:00, then it ends automatically and restrictions lift. |
| A-STR-02 | Given strict mode bound to a schedule, then it re-arms on the next window and cannot be paused between windows. |
| A-STR-03 | Given a 10-minute cool-down, when disable is pressed, then the countdown starts, suspend pauses it, and a second press after it ends deactivates. |
| A-STR-04 | Given a random-text challenge, then exact text succeeds, wrong text fails three times and triggers a 30 s lockout. |
| A-STR-05 | Given a saved QR, then scanning it ends the request; an invalid code does not. |
| A-STR-06 | Given friend approval with the service unreachable, then the state stays "waiting", retries, and never self-approves. |
| A-STR-07 | Given a password gate, then opening the app without the password is denied; wrong attempts back off; the correct one succeeds. |
| A-STR-08 | Given "block settings" supported by the desktop, then the settings window is covered; if unsupported, the commitment cannot be started with that option. |
| A-STR-09 | Given an active commitment, then profile edits, deletion, allowlist additions, and rule loosening are rejected by daemon and UI. |
| A-STR-10 | Given lock mode, then "Normal mode" requires the password and restores full access. |
| A-STR-11 | Given an active commitment, when the daemon and UI are killed and the machine reboots, then restrictions are active before adapters start. |
| A-STR-13 | Given a strict alarm, when it fires, then sound plays and dismissing requires the configured challenge. |

## Content filtering

| ID | Scenario |
| --- | --- |
| A-CON-01 | Given Shorts blocking on, when `/shorts/abc` is opened, then the surface is replaced and the URL remains usable. |
| A-CON-02 | Given Reels/Stories toggles, then only enabled surfaces are replaced. |
| A-CON-03 | Given all toggles off, then no surface is altered. |
| A-CON-04 | Given adult filtering, when a listed domain resolves, then it is sinkholed and the neutral block page appears. |
| A-CON-05 | Given an unsupported browser is blocked, then launching it is treated as an app block. |
| A-CON-06 | Given keyword "GaMe", then `game`, `GAME`, and full-width variants match. |
| A-CON-08 | Given an embedded webview that cannot be inspected, then the app reports the limitation instead of silently claiming coverage. |

## Statistics

| ID | Scenario |
| --- | --- |
| A-STA-01 | Given a focus interval, then exactly its duration is attributed to that app and no other. |
| A-STA-02 | Given events across a week boundary, then totals for today/week/month match the fixtures. |
| A-STA-03 | Given three apps, then the top-distractions order is by time and ties are stable. |
| A-STA-04 | Given a day with gaps, then the timeline shows idle gaps, not zero usage. |
| A-STA-05 | Given screen lock/unlock events, then pickups increment once per unlock and launches once per foreground entry. |
| A-STA-06 | Given seven days of data, then the weekly report compares equal-length previous periods. |
| A-STA-07 | Given a blocked day, then the history lists reason codes and end times without window titles. |
| A-STA-08 | Given "export", then a schema-versioned archive is produced; "delete" removes raw and aggregate data. |

## UX and settings

| ID | Scenario |
| --- | --- |
| A-UX-01 | Given a clean profile, onboarding creates exactly one profile and no optional capability is silently enabled. |
| A-UX-02 | Given an active profile, the dashboard shows profile name, state, and next boundary, updating on change. |
| A-UX-03 | Given a block, the overlay shows reason, rule name, and end time, and "Back" focuses the previous window. |
| A-UX-04 | Given a break, the countdown matches the timer and the allowlist behaves as chosen. |
| A-UX-05 | Given unsaved editor changes, navigating away prompts; discarding restores the saved state. |
| A-UX-06 | Given each schedule type, the editor validates and round-trips without loss. |
| A-UX-07 | Given strict setup, the summary names the exact method and the earliest end before acceptance. |
| A-UX-08 | Given settings changes, the daemon applies them without restart and the UI reflects them. |
| A-UX-09 | Given the tray, pause/resume/quick block act on the daemon and the icon reflects state. |
| A-UX-10 | Given keyboard-only navigation, every action is reachable and every icon has a label. |
| A-UX-11 | Given a locale change, no string falls back to keys and plural forms are correct. |
| A-UX-12 | Given an export, importing it on a clean install restores profiles, settings, and current commitment (never weakening it). |

## Platform

| ID | Scenario |
| --- | --- |
| A-PLAT-01 | Given a session start, the daemon starts once and restarts after a kill. |
| A-PLAT-02 | Given X11, focus changes produce foreground events and hide/overlay calls in order. |
| A-PLAT-03 | Given sway, the IPC adapter reports focus and scratchpad-hides blocked apps. |
| A-PLAT-04 | Given Hyprland, the event socket drives the same decisions. |
| A-PLAT-05 | Given GNOME Wayland with the extension, app id and overlay work; without it, degraded is reported. |
| A-PLAT-06 | Given KWin with the script, the same conformance trace passes. |
| A-PLAT-07 | Given the DNS helper, enabling writes resolver config reversibly; stopping the daemon reverts it. |
| A-PLAT-08 | Given the extension, a host rule blocks navigation and the reason matches the daemon. |
| A-PLAT-09 | Given screen lock, tracking stops and overlays release; unlock resumes without false pickups. |
| A-PLAT-10 | Given two monitors, the overlay covers both and the card appears on the focused one. |
| A-PLAT-11 | Given a low-time threshold, exactly one notification is delivered and it is clickable. |

## Data and lifecycle

| ID | Scenario |
| --- | --- |
| A-DATA-01 | Given a database from the previous schema, migration preserves every profile, rule, and event count. |
| A-DATA-02 | Given a DST transition and a backwards clock jump, totals are not double counted and commitments are not shortened. |
| A-DATA-03 | Given an older UI and newer daemon, the UI shows a version error and blocking continues. |
| A-DATA-04 | Given "reset all data", commitments block the action; after confirmation, rules/history are cleared and the daemon stays consistent. |

## Gap verification

| ID | Scenario |
| --- | --- |
| A-GAP-01 | The app's diagnostics screen lists every `gap` row from the matrix with a one-line reason. |
| A-GAP-02 | No screen claims enforcement where the adapter reports a missing capability. |
