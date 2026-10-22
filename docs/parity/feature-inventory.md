# Feature inventory

Subsystem-level inventory of the reference product, with Linux relevance and
confidence. Behaviour details live in `docs/spec/`; priorities and test IDs live
in `docs/parity/feature-matrix.md`.

Confidence: **O** observed in the shipped artifact, **D** derived from observed
facts, **I** inferred. The artifact was analysed statically and never executed.

## 1. Core blocking

| Feature | Reference mechanism | Conf. | Linux plan |
| --- | --- | --- | --- |
| Hard app block | per-profile app rules enforced on foreground change | O | compositor adapter + lock overlay |
| Per-app daily limit | foreground time accumulated per app/day | O | usage tracker + counters |
| Per-app hourly limit | foreground time per clock hour | O | same |
| Daily/hourly launch limits | foreground launch events | O | launch counter |
| Pickup (unlock) limits | device unlock events | O | logind session unlock events |
| Hard site block | browser address-bar inspection | O | DNS sinkhole + extension |
| Keyword block | match URL/search text against user list | O | extension + URL feed |
| Allowlist (apps/sites) | explicit allow overrides profile | O | engine precedence |
| Goal-based access | spend time in chosen app to unlock another | O | engine rule type |
| Wait timer | cooldown after N minutes of use | O | engine timer |
| Adaptive timer | cooldown shrinks when app unused | O | engine timer |
| Quick block | ad-hoc block for a duration | O | UI + engine |
| Break/pause/extend | time-boxed suspension or extension | O | engine timers |

## 2. Schedules and profiles

| Feature | Reference mechanism | Conf. | Linux plan |
| --- | --- | --- | --- |
| Multiple profiles | named rule bundles | O | domain model |
| Profile templates | shipped presets (focus, sleep, detox, exam, …) | O | bundled presets |
| Daily schedule | time window per day | O | schedule engine |
| Day-of-week schedule | per-day windows | O | schedule engine |
| Hourly pattern | within-hour windows | O | schedule engine |
| Interval schedule | multiple windows per day | O | schedule engine |
| Full-day schedule | 00:00–24:00 | O | schedule engine |
| Custom/one-time schedule | dated window | O | schedule engine |
| Start-of-day offset | shifts daily counter reset | O | counter period |
| Manual activation | dashboard/widget toggle | O | D-Bus method |
| Location activation | geofence | O | GeoClue2 (P2) |
| Pause with/without duration | indefinite pause option | O | engine state |

## 3. Strict mode and lock mode

| Feature | Reference mechanism | Conf. | Linux plan |
| --- | --- | --- | --- |
| Expiry-based strict mode | timer, auto-deactivate | O | commitment state machine |
| Schedule-based strict mode | active while schedule runs | O | commitment state machine |
| Cool-down deactivation | wait before disable | O | state machine + timer |
| Random-text challenge | type generated text | O | challenge type |
| QR challenge | scan pre-saved code | O | challenge type (camera/portal) |
| Friend approval | server-mediated async approval | O | optional self-hosted approval |
| Password gate | protects app/settings/edits | O | local secret (argon2id) |
| Block settings | prevents changing restrictions | O | policy + overlay/portal |
| Block uninstall | device-admin style resistance | O | **no safe equivalent**; documented gap |
| Block recent apps / split screen | accessibility heuristics | O | compositor policy (P1) |
| Block power/battery settings | accessibility heuristics | O | documented gap |
| Strict alarms | exact alarm + challenge to dismiss | O | timer + challenge (P2) |
| Lock mode | password-gated normal mode | O | policy state |

## 4. Content filtering

| Feature | Reference mechanism | Conf. | Linux plan |
| --- | --- | --- | --- |
| YouTube Shorts | view identity + visibility | O | browser extension DOM rules |
| Instagram Reels | view identity + visibility | O | extension |
| Instagram/Facebook/Snapchat Stories | view identity | O | extension |
| Snapchat Spotlight | view identity | O | extension |
| WhatsApp Status/Channels | view identity | O | documented limitation (native app) |
| Facebook Reels | localized text match | O | extension |
| Adult domains | domain list | O | DNS + extension |
| Unsupported browsers | block the browser itself | O | engine rule |
| Keyword blocking | URL/search text | O | extension + URL pipe |
| In-app browser gap | web views skipped | O | **explicit limitation** |

## 5. Awareness and statistics

| Feature | Reference mechanism | Conf. | Linux plan |
| --- | --- | --- | --- |
| Usage timeline | foreground events | O | SQLite event log |
| Top distractions | aggregate per app/site | O | queries |
| Screen-time totals | daily/weekly | O | queries |
| Pickup/unlock count | device unlock events | O | logind |
| Launch count | process/window events | O | tracker |
| Weekly report | derived comparison | O | report view |
| Low-time alerts | threshold notifications | O | daemon + notifications |
| Blocked-notification log | notification listener | O | **not reproduced** (privacy) |

## 6. Focus tools

| Feature | Reference mechanism | Conf. | Linux plan |
| --- | --- | --- | --- |
| Pomodoro | focus/break/session settings | O | timer engine |
| Break timer | countdown + widget | O | timer + tray |
| Ambient sounds | streamed catalog + mixes | O | bundled/optional local audio |
| Motivational quote | block-screen text | O | setting |
| Themes/dark mode | theme list | O | libadwaita theme |
| Launcher mode | app as home screen | O | documented gap (Android-only) |
| Widgets | home-screen widgets | O | tray/desktop widget (P2) |
| Quick-settings tile | Android tile | O | tray quick action |
| Backup/restore | file export | O | JSON export/import |
| Storage manager | cache/usage cleanup | O | settings page |

## 7. Account and cloud (optional, not required)

| Feature | Reference mechanism | Conf. | Linux plan |
| --- | --- | --- | --- |
| Email account | backend auth | O | out of scope (local-first) |
| Device registration | backend | O | optional self-hosted |
| Usage/purchase sync | backend | O | optional self-hosted |
| Friend approval | backend | O | optional self-hosted |
| Feedback | backend | O | issue tracker / mailto |

## 8. Platform integration

| Feature | Reference mechanism | Conf. | Linux plan |
| --- | --- | --- | --- |
| Boot persistence | boot receivers | O | systemd user unit |
| Background reliability | foreground service, battery exemptions | O | daemon |
| Screen lock awareness | user-present receiver | O | logind `LockedHint` |
| Install/uninstall detection | package broadcasts | O | OS package events (not needed) |
| Exact alarms | AlarmManager exact | O | timer wheel + systemd timers |
| Notifications | notification listener + FCM | O | D-Bus notifications |
| Accessibility enforcement | accessibility service | O | compositor adapters |

## 9. Known gaps and deliberate non-goals

- **Native-app surface filtering is not reproducible on Linux.** WhatsApp
  Status/Channels, Instagram, Snapchat and YouTube are native/mobile surfaces;
  the equivalent on Linux is a browser extension (web clients) plus a
  documented gap for native clients.
- **Uninstall prevention has no safe Linux equivalent.** A user with root can
  always remove software. Strict mode is a commitment device, not DRM.
- **No notification-content reading.** The reference logs blocked notifications;
  StayProductive will not read other apps' notification contents.
- **No ads, billing, telemetry, or mandatory accounts.**
- **No device-admin equivalent.** Linux polkit/systemd integration is explicit
  and uninstallable.

## 10. Coverage note

Presence is established from code paths, resources, and the manifest. Runtime
timing, server responses, and the exact wording of every dialog were not
exercised. Where the spec needs a number the reference does not state (e.g.
default grace), the value is marked as a StayProductive decision, not parity.
