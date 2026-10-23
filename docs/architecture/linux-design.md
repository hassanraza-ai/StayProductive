# Linux architecture

How the specified behaviour is delivered on a Linux desktop. This document is
normative for the implementation; deviations need an ADR and a matrix update.

## 1. Goals and non-goals

**Goals**

- Enforce app and site rules reliably on supported desktops.
- Keep the decision engine independent of any desktop so behaviour is testable
  and identical everywhere.
- Degrade loudly and honestly when a desktop cannot be enforced.
- Default install unprivileged; privileged components opt-in, narrow, and
  removable.

**Non-goals**

- Becoming unremovable, fighting root, or modifying system binaries.
- Supporting every Wayland compositor on day one.
- Reading other applications' content or notifications.
- Cloud accounts, ads, telemetry, or DRM.

## 2. Stack decision

| Layer | Choice | Why |
| --- | --- | --- |
| Core + daemon | **Rust** (stable, edition 2024) | deterministic engine, single static binary, low idle cost, strong types for time/state |
| Async runtime | **tokio** in daemon only | timers, D-Bus, adapter sockets; core stays runtime-free |
| IPC | **D-Bus** (session bus, `org.stayproductive.Daemon1`) + JSON over a Unix socket | desktop-native; CLI/extension can use either |
| Store | **SQLite** (WAL, rusqlite) | transactional, migration-friendly, inspectable |
| UI | **GTK 4 + libadwaita** (gtk-rs) | best Linux desktop integration, accessibility, Flatpak support |
| Notifications | `org.freedesktop.Notifications` | universal |
| Privileged helper | small Rust binary + polkit action | system DNS only; no generic root API |
| Extension | WebExtension (MV3), shared TypeScript core | one codebase for Chromium and Firefox |
| Packaging | Flatpak, `.deb`, `.rpm`, Arch, systemd user unit | broad reach |

Alternatives considered: Electron (rejected: memory, no native enforcement), Qt
(viable, but GTK/libadwaita matches GNOME defaults and the first target desktops),
Kotlin/JVM (rejected: daemon footprint and packaging complexity).

### Process model

```
┌──────────────┐   D-Bus    ┌───────────────┐   unix socket   ┌──────────────┐
│  GTK UI      │◄──────────►│    daemon     │◄───────────────►│  extension   │
└──────────────┘            │  (per user)   │                 └──────────────┘
                            │  ┌─────────┐  │
┌──────────────┐   D-Bus    │  │  core   │  │   exec/spawn    ┌──────────────┐
│  CLI         │◄──────────►│  └─────────┘  │◄───────────────►│  adapters    │
└──────────────┘            └───────┬───────┘                 └──────────────┘
                                    │ SQLite
                            ┌───────▼───────┐        ┌──────────────────┐
                            │  state.db     │        │ dns-helper (opt) │
                            └───────────────┘        └──────────────────┘
```

- One daemon per user session, started by a systemd user unit. It owns the
  database; the UI never writes to it directly.
- If the daemon dies, systemd restarts it; commitment state reloads before any
  adapter is enabled, so enforcement resumes with the restrictions intact.
- The UI is optional: blocking continues without it.

### Current runnable build (transitional)

The first runnable build deviates from the target stack in four narrow ways,
without moving decisions out of `core`:

| Target | Current build | Why |
| --- | --- | --- |
| D-Bus + Unix socket | newline-delimited JSON on stdio | fastest safe UI boundary; one request/response line, started by the UI |
| SQLite | versioned atomic JSON | no queryable history is needed until trackers exist |
| gtk-rs | PyGObject | GTK 4 development headers were unavailable in the build environment; the widget layer is still GTK 4 + libadwaita |
| D-Bus-driven GNOME extension | short-lived policy file + runtime heartbeat | no daemon D-Bus name yet; the file is same-user, atomic, and expires in 30 seconds |

The daemon is still the only writer. The UI still has no decision logic. Both
are expected to be replaced at M1; the matrix and implementation notes record
the limitation rather than presenting it as the final architecture.

## 3. D-Bus surface (session bus)

```
org.stayproductive.Daemon1
  /org/stayproductive/Daemon1
    methods:
      GetStatus() -> (state, profile, until, degraded[])
      ListProfiles() -> a(s)
      ActivateProfile(s name)
      PauseProfile(s name, u seconds)      # 0 = indefinite
      ResumeProfile(s name)
      QuickBlock(u seconds)
      TakeBreak(u seconds)
      GetDecision(s app, s host) -> (verdict, reason, until)
      GetUsage(s period) -> a(...)
      RequestCommitmentEnd(s id)
      SubmitChallenge(s id, s answer)
      Doctor() -> a(s)
    signals:
      StateChanged(...)
      Blocked(...)
      Degraded(s adapter, s reason)
      CommitmentChanged(...)
```

- Versioned interface (`Daemon1`); breaking changes bump the suffix and the
  daemon serves both during a transition.
- All methods are idempotent where semantics allow; every state-changing call
  returns the new state.
- Authorization: same-UID peers only (D-Bus policy). No method bypasses a
  commitment.

## 4. Desktop adapters

Each adapter implements:

```rust
trait DesktopAdapter {
    fn capabilities(&self) -> Capabilities;     // foreground, hide, overlay, workspaces
    fn foreground(&self) -> Result<Foreground>; // app, window, pid
    fn hide(&self, window: WindowKey) -> Result<()>;
    fn show_overlay(&self, monitors: &[Monitor], reason: &BlockInfo) -> Result<()>;
    fn events(&self) -> EventStream;            // focus, workspace, monitor changes
}
```

| Desktop | Foreground | Hide/minimise | Overlay | Notes |
| --- | --- | --- | --- | --- |
| **X11 (generic)** | `_NET_ACTIVE_WINDOW` + `_NET_WM_PID` | `_NET_WM_STATE_HIDDEN` / minimise | override-redirect fullscreen on each monitor | tier 1; works on GNOME X11, KDE X11, Xfce, Cinnamon, MATE |
| **sway / i3** | sway IPC `GET_TREE` | `move to scratchpad` | layer-shell surface (sway), or fullscreen view | tier 1 for sway; i3 overlay via fullscreen window |
| **Hyprland** | `hyprctl activewindow -j` / event socket | `dispatch movetoworkspacesilent` | layer-shell | tier 1 |
| **GNOME (Wayland)** | Shell extension (focus + app id) | extension `minimize` | extension fullscreen actor | tier 2 |
| **KDE Plasma (Wayland)** | KWin script + D-Bus | script `minimize` | script fullscreen window | tier 2 |
| **Other wlroots** | per-compositor IPC | best effort | layer-shell where available | tier 3, report degraded |

Adapter detection is automatic (`XDG_CURRENT_DESKTOP`, `WAYLAND_DISPLAY`,
`SWAYSOCK`, `HYPRLAND_INSTANCE_SIGNATURE`), with a manual override in settings.
Capabilities are reported, not assumed: the UI shows exactly what is enforced.

## 5. Enforcement strategy

### App blocking

1. Foreground event arrives with `AppKey` and `WindowKey`.
2. Engine blocks → adapter shows the overlay on all monitors and hides the
   blocked window (no process kill).
3. "Back" focuses the previously allowed window/workspace; if none, the overlay
   remains with a workspace switcher.
4. Repeated attempts are counted and rate-limited; the overlay stays.

Why not kill processes: data loss, launcher breakage, and it is not what the
reference does (it sends the user home). Hiding an app is the faithful mapping.

### Site blocking

- **DNS helper (optional, system-wide)**: a local resolver that answers blocked
  hosts with a sinkhole address and forwards everything else. Configured through
  `systemd-resolved` or the desktop's resolver; changes are reversible and
  recorded.
- **Extension (default, no privilege)**: sees navigations, applies host,
  keyword, and surface rules, and renders the block card.
- Both ask the daemon for decisions so the reason and end time match.

### Settings blocking (strict mode)

- Attempt, in order: desktop portal settings (where available), a KWin/GNOME
  policy hook, then an overlay that covers the settings window.
- Never edit system files or disable the settings app.
- If none is available, the capability is reported unsupported **before** the
  commitment is accepted.

## 6. Overlay

- One overlay process (part of the UI crate) with a fullscreen, always-on-top,
  keyboard-grabbing window per monitor.
- X11: override-redirect window + `_NET_WM_STATE_ABOVE`, `_NET_WM_STATE_FULLSCREEN`,
  input focus; re-assert above state on focus changes.
- wlroots: `wlr-layer-shell` on the overlay layer with exclusive keyboard
  interactivity.
- GNOME/KDE: extension/script creates the actor/window; the daemon drives it.
- The overlay must never cover the user's security prompts (polkit, screen
  unlock) and must release grabs on session lock.

## 7. Privilege model and DNS helper

- Default package: no root, no setuid, no capabilities.
- Optional `stayproductive-dns-helper`:
  - D-Bus system service with three methods: `Enable(hosts)`, `Disable()`,
    `Status()`; no arbitrary file writes, no shell, no path parameters.
  - polkit action `org.stayproductive.helper.configure` requiring
    `auth_admin_keep`; the prompt names the exact change.
  - Reverts automatically if the daemon disappears (watchdog) or on uninstall.
  - Refuses to run if another resolver manager is active and unmanaged; reports
    what it would change first.
- `/etc/hosts` editing is **not** used (conflicts, no TTL, hard to revert).
- Flatpak build: DNS helper unavailable; extension-only mode, stated in the
  store listing and in-app.

## 8. Data, IPC, and compatibility

- SQLite schema version is in `PRAGMA user_version`; migrations are forward-only
  and run under a transaction after a file copy backup.
- IPC payloads carry `schema_version`; the daemon rejects unknown newer versions
  with a clear error and the UI offers to update.
- The GNOME extension currently reads a short-lived unsigned policy snapshot
  written by the daemon and writes a runtime heartbeat. It is same-user only and
  expires after 30 seconds. Signed snapshots and blocking across daemon restarts
  remain target behaviour, not current behaviour.

## 9. Performance budgets

| Metric | Budget | Measurement |
| --- | --- | --- |
| Idle CPU (no window changes) | < 0.3% of one core | 10-minute `pidstat` |
| Decision latency p99 | < 5 ms | criterion bench + trace |
| Daemon RSS (10k rules) | < 80 MB | `/proc/self/status` |
| Event queue | bounded at 4,096, drop-oldest with counter | metrics |
| Storage growth | < 20 MB/month typical | fixture replay |
| UI cold start | < 800 ms | `hyperfine` |

Measure before/after for changes in the decision path; report numbers in the PR,
not adjectives.

## 10. Threat model

| Threat | Mitigation | Residual risk |
| --- | --- | --- |
| User kills daemon | systemd restart, state reload | root can stop it |
| User edits DB | commitment audit + integrity check; UI refuses inconsistent state | root can edit |
| Clock tampering | backwards never shortens a commitment; monotonic timers | forward jump ends expiry (accepted) |
| Uninstall | stated up front | cannot be prevented |
| DNS bypass (DoH/VPN) | extension covers supported browsers; UI warns | determined user bypasses |
| Extension tampering | snapshot signed; daemon re-asserts | user controls their browser |
| Malicious app impersonating overlay | overlay is our process; window identity checks | compositor trust |

The product promise is **friction, not imprisonment**. Documentation says this
plainly.

## 11. Packaging

| Format | Includes | Limitations |
| --- | --- | --- |
| Flatpak | daemon, UI, extension, user service | no DNS helper, no system autostart beyond portal, adapter access varies |
| `.deb` / `.rpm` | daemon, UI, extension, optional helper package | helper requires polkit |
| Arch (AUR) | split packages `stayproductive`, `-dns-helper` | same |
| AppImage | UI + daemon | no system integration by default |

Autostart: systemd user unit `stayproductive.service` enabled per user; the UI
offers a toggle. No system-wide daemon.

## 12. Testing per adapter

- Each adapter ships recorded event traces (focus sequences, workspace changes,
  monitor hotplug) as fixtures.
- A conformance suite runs the same scenario against every adapter with a fake
  compositor, asserting the same decisions and enforcement calls.
- One manual smoke test per tier-1 desktop is required before release; CI cannot
  prove compositor behaviour.
- Unsupported desktops must produce a `Degraded` capability report, tested.

## 13. Decisions recorded

| ADR | Decision |
| --- | --- |
| ADR-001 | Rust + GTK4/libadwaita, daemon/UI split |
| ADR-002 | D-Bus session API + Unix socket for the extension |
| ADR-003 | Hide, do not kill, blocked applications |
| ADR-004 | DNS helper is opt-in and separate; Flatpak is extension-only |
| ADR-005 | Commitments use wall-clock expiry, monotonic cool-downs |
| ADR-006 | Transitional GNOME enforcement uses an expiring policy file and heartbeat; D-Bus replaces it at M1 |

Open decisions: see `docs/parity/feature-matrix.md` §Open decisions (D-1…D-4).

## 14. Risks

1. **Wayland fragmentation** — mitigated by tiering and explicit degraded states.
2. **DNS helper security** — narrow API, polkit, watchdog, no file paths.
3. **Overlay focus fights** — bounded retries, compositor-native layers first,
   security prompts excluded.
4. **Scope creep toward Android hacks** — AGENTS.md forbids reproducing them.
5. **User expectation of "unbreakable"** — onboarding and docs state the real
   threat model; strict mode is a commitment device.
