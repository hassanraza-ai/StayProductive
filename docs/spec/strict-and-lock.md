# Strict mode and lock mode

Strict mode is a **commitment device**: the user deliberately makes deactivation
hard *before* motivation fades. Lock mode is the lighter, password-gated variant.
Neither is DRM; a user with root can always remove the software. The design goal
is to make impulsive bypassing require deliberate, visible effort.

## 1. Kinds

| | Strict mode | Lock mode |
| --- | --- | --- |
| Purpose | enforce a promise for a period | protect settings/app with a password |
| Entry | explicit setup with an end condition | password set once |
| Restrictions | configurable set, frozen once active | fixed: settings, app access, profile edits |
| Exit | configured method only | password |
| Duration | expiry, schedule, or manual | until disabled |
| Pause allowed | no | no |

## 2. Setup

The setup wizard asks, in order:

1. **End condition** — a date/time, a schedule, or "until I turn it off".
2. **Deactivation method** — cool-down, random text, QR, or friend approval.
3. **Restrictions** — what to block while active.
4. **Optional password** — protects opening StayProductive and switching modes.
5. **Confirmation** — a screen that states exactly what will be locked and how
   it can be ended, requiring an explicit accept.

The confirmation must not use dark patterns: no pre-checked consent, no
countdown that auto-accepts, and the exit method is always named.

## 3. Restrictions

```rust
struct RestrictionSet {
    block_settings: bool,      // deny the system settings app while active
    block_edits: bool,         // rules/commitment cannot be edited or loosened
    block_uninstall: bool,     // documented gap on Linux
    password_gate: bool,       // opening the app requires the password

    // Optional, best-effort via compositor adapters:
    block_recent_windows: bool,
    block_split_view: bool,
    block_power_settings: bool,
}
```

Rules:

- Restrictions are chosen before `Armed → Active` and are immutable afterwards.
- Adding a restriction while active is allowed only through the same
  confirmation flow; removing one is impossible until the commitment ends.
- `block_edits` covers profile deletion, rule loosening, schedule shortening,
  allowlist additions, and disabling the daemon from the UI.
- The CLI exposes no way to remove a restriction; it may only report state.

**Gaps, stated honestly:**

- `block_uninstall` cannot be guaranteed on Linux. The package can be removed
  with the package manager. The UI says so when the option is selected.
- `block_power_settings` cannot be enforced against a user with root. It is
  best-effort via the settings portal where available, otherwise marked
  unsupported.
- `block_settings` uses a policy overlay/portal attempt, never system file
  modification. If it cannot be enforced, the UI shows "not enforceable on this
  desktop" *before* the commitment is accepted.

## 4. Lifecycle

```
        setup accepted
              │
              ▼
           Armed ──(start time)──► Active
                                      │
                     user requests end│
                                      ▼
                             DeactivationRequested
                                      │
              ┌───────────────────────┼────────────────────────┐
              ▼                       ▼                        ▼
        CoolingDown             AwaitingChallenge        AwaitingApproval
              │                       │                        │
              └───────────┬───────────┴────────────┬───────────┘
                          ▼                        │
                     ReadyToEnd ◄─────────────────┘
                          │ user confirms
                          ▼
                        Ended
```

- `Armed` exists only when the commitment starts in the future (schedule or
  picked start). It can be cancelled before it becomes `Active`.
- Once `Active`, only the configured method moves the state forward.
- `DeactivationRequested` is recorded immediately, so closing the app does not
  reset progress.
- Any request can be cancelled before `Ended`, returning to `Active`.
- `Ended` is terminal; the restrictions are lifted atomically and the event is
  audited.

## 5. Deactivation methods

| Method | Flow | Notes |
| --- | --- | --- |
| **Expiry** | automatic at `ends_at` | wall clock; survives shutdown |
| **Schedule** | automatic when the schedule leaves its window | strict mode re-arms next window |
| **Cool-down** | press disable → wait `d` → press again | timer pauses during suspend; UI shows countdown |
| **Random text** | type a generated phrase exactly | case-sensitive; generated locally, shown on the request screen |
| **QR** | scan a saved code with the camera | code is stored at setup; losing it is a recovery case (§9) |
| **Friend approval** | request sent to a trusted contact; wait for approval | optional self-hosted service; retries while unreachable |

Method details:

- Cool-down presets: 1, 5, 10, 15, 30 min, 1 h, 6 h, 24 h, 3 days. Default 10 min.
- Random text: 40–80 characters, mixed case, no ambiguous glyphs; the input is
  compared without trimming internal whitespace; three failures trigger a
  30-second lockout before retry.
- QR: generated locally as a random 32-byte payload with a checksum; scanning
  uses the portal camera or an uploaded image; a photographed code is as valid
  as the original by design.
- Approval: request includes commitment ID, requested end, and method; approval
  is idempotent and expires after 24 h.

## 6. Password gate and lock mode

- Secrets are stored as Argon2id hashes (m=64 MiB, t=3, p=1) with a per-install
  salt. Minimum 6 characters, no maximum below 128.
- The password gates: opening the app, changing modes, editing restrictions, and
  disabling the daemon. It does **not** encrypt usage data (documented).
- Lock mode: set a password, choose which of {settings, app access, profile
  edits} it protects, and exit through "Normal Mode" with the password.
- Wrong password: exponential backoff (1 s, 2 s, 4 s, … capped at 5 min) after
  five attempts; no lockout that could permanently brick the app.
- Changing the password requires the old one; while a commitment is active,
  changing it is forbidden.

## 7. Durability

- Commitment state is written to SQLite before any restriction is applied, and
  a `state.json` snapshot is written after each transition.
- The daemon re-arms timers on start; if the daemon is killed, systemd restarts
  it and the commitment continues.
- Killing the UI has no effect. Uninstalling the package ends the commitment;
  this is stated up front and cannot be prevented.
- Commitments are excluded from "reset all data" and from restore operations
  that would weaken them.

## 8. Clock and suspend semantics

| Event | Behaviour |
| --- | --- |
| Wall clock moves forward past expiry | commitment ends; no penalty |
| Wall clock moves backward | expiry recomputed from the original wall deadline; cool-down progress preserved |
| Timezone change | wall deadline follows absolute time; UI explains |
| Suspend | expiry continues in wall time; cool-down pauses |
| Reboot | state reloaded; expired commitments end; cool-down resumes from stored progress |

## 9. Recovery and safety

A self-control tool must not create an unrecoverable device:

- **Forgotten password**: recovery by deleting the local database from a
  terminal is documented; it ends commitments and loses history. The UI states
  this path before the password is set.
- **Lost QR**: the code can be regenerated only after the commitment ends;
  during the commitment, the user can choose "I lost my QR" which starts a
  24-hour cool-down and requires typing a long confirmation phrase. This is a
  safety valve, not a bypass.
- **Approval server unreachable**: the commitment stays active; the UI shows
  "waiting" and retries. After 7 days of unreachable approval, a local recovery
  phrase is offered (documented).
- **Clock tampering**: moving the clock backwards never shortens a commitment;
  moving it forward ends it, which is accepted (the user could wait anyway).

## 10. Strict alarms (P2)

A strict alarm is a scheduled commitment with a wake-up challenge: maths,
random text, shake (mobile concept), or QR. On Linux:

- the alarm is a systemd timer + daemon timer, with a full-screen ring window;
- sound plays through PipeWire, with a preview in settings;
- the challenge is the configured one; disabling the alarm requires it;
- alarms are limited to a configurable count (reference limits free users to
  one; StayProductive has no such limit).

## 11. Audit and UX requirements

- Every transition is appended to `commitment_history`: `(at, from, to, actor,
  method, reason)`.
- The overlay states: what is blocked, why, the method to end it, and the
  earliest possible end.
- The app's own status always shows "Strict mode active until …" and the
  deactivation method.
- No screen may imply an exit that does not exist. "Disable" always either
  starts the configured method or explains why it is unavailable.
