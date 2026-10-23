# StayProductive for GNOME Shell

This extension is StayProductive's first desktop enforcement adapter. It reads a
short-lived policy file produced by the local state service, watches the focused
window, and covers a blocked application with a full-screen local overlay.

It is optional. The rest of StayProductive works without it.

## What it enforces

- Application `block` rules in an active profile.
- Application quick blocks created from the dashboard.
- The same allowlist and precedence decisions the Rust engine already makes;
  the extension only receives the result.
- Strict mode indirectly: if a strict commitment is keeping a profile active,
  that profile's app blocks are published like any other active profile.

It does **not** enforce websites, keywords, daily/hourly usage limits, launch
limits, or pickup limits yet. Those need the browser extension and the usage
tracker. The UI says so instead of pretending.

## Requirements

- GNOME Shell 45–49 on Wayland or X11.
- StayProductive is running so it can refresh the policy and state service.
- One log out / log in after the first installation. GNOME Shell does not load a
  newly copied extension until the Shell restarts.

## Install

Use **Settings → Desktop integration → Install**, then log out and back in.
Status should change to **Active** after the Shell loads the extension and
writes its heartbeat.

Manual installation:

```sh
UUID=stayproductive@hassanraza.ai
TARGET="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/$UUID"
mkdir -p "$(dirname "$TARGET")"
cp -r integrations/gnome/"$UUID" "$TARGET"
gnome-extensions enable "$UUID"
```

Then log out and back in. Check the extension in **Extensions** or with:

```sh
gnome-extensions info stayproductive@hassanraza.ai
```

## How it works

1. The daemon writes `enforcement.json` beside `state.json` after every command.
   The file contains only applications that the engine would block right now,
   the reason code, the optional end time, and a `valid_until` timestamp
   (30 seconds after generation).
2. The extension reads the policy every second. An expired policy is ignored,
   so closing StayProductive stops enforcement instead of leaving stale blocks.
3. For each focused window it collects the Wayland/XWayland app id,
   `WM_CLASS`, and GApplication id, normalizes them, and looks for a policy entry.
4. On a match it shows an overlay on every monitor with the reason and end time,
   and takes keyboard focus away from the blocked window.
5. Every two seconds it writes
   `$XDG_RUNTIME_DIR/stayproductive/gnome-extension.json`. The daemon treats that
   heartbeat as fresh for five seconds, and the heartbeat must say it loaded a
   valid policy, before the daemon reports enforcement as active. A disabled,
   crashed, or policy-less extension cannot keep claiming enforcement.

The overlay can be left with `Super`/window switching, but the blocked window is
covered again when it regains focus.

## Privacy

- No network access, account, telemetry, or remote service.
- The policy file is mode `0600`; the heartbeat is mode `0600` in the user's
  runtime directory.
- The extension reads app identity only. It does not read window contents,
  keystrokes, URLs, or clipboard data.
- The heartbeat contains no window titles or per-app history.

## Emergency unblock

StayProductive is a commitment device, not a rootkit. The local user can always
disable the adapter:

```sh
gnome-extensions disable stayproductive@hassanraza.ai
```

A strict session still constrains what the StayProductive UI and daemon will
change, but the extension is a removable user-level component. See
`docs/spec/strict-and-lock.md` for the threat model.

## Uninstall

```sh
gnome-extensions disable stayproductive@hassanraza.ai
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/stayproductive@hassanraza.ai"
```

The extension source and tests live in this directory. Run the shared matching
tests with:

```sh
gjs -m integrations/gnome/tests/matching.test.js
```
