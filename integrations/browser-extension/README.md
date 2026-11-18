# Browser extension

A Manifest V3 WebExtension that asks the StayProductive daemon before a page is
shown and applies the host rules it can before the request leaves the browser.
It is a companion to the desktop app, not a second policy engine: the Rust
daemon makes every decision, and the extension is useful only while that daemon
is running for the same user.

Implemented against `docs/spec/content-filtering.md` (host and keyword layers)
and `F-PLAT-08` in the parity matrix. Clean-room: no code, strings, icons, or
assets from the reference application are used.

## What it covers

| Layer | Mechanism | Confidence |
| --- | --- | --- |
| Host block and allow rules | `declarativeNetRequest` rules built from `browser_policy` | best effort, refreshed every minute and after navigations |
| Quick site blocks | Same dynamic rules, higher priority than normal allows | best effort |
| Keyword rules | `content.js` asks `browser_decision` on each navigation and same-document URL change | after the page starts loading |
| Site time limits | Not implemented; the daemon reports `site_limits` as a limitation | none |
| In-page surfaces (Shorts, Reels, Stories, adult filtering in-page) | Not implemented; the daemon reports `surface_rules` | none |
| Native applications | Out of scope for a browser extension | none |

The extension never claims more than this: `browser_policy` returns a
`limitations` array and the toolbar badge turns into `!` when the daemon cannot
be reached.

## Requirements

- A built and running daemon for your user (`./stayproductive`, or
  `systemctl --user start stayproductive.service`).
- Chrome/Chromium 120 or newer, or Firefox 128 or newer.
- Python 3.9 or newer for the native host. The host is per-user; no root.

## Install

### Chromium, Chrome, Brave, Edge

1. Open `chrome://extensions`, turn on **Developer mode**, choose **Load
   unpacked**, and select `integrations/browser-extension/extension`.
2. Copy the extension ID shown on the card (32 letters, `a`–`p`).
3. Install the native host with that exact origin:

   ```sh
   python3 integrations/browser-extension/install.py \
     --browser chrome \
     --extension-id <the-id-from-step-2>
   ```

   Use `--browser chromium`, `--browser brave`, or `--browser edge` instead when
   that is the browser you loaded it into. Repeat `--browser` for more than one.
4. Restart the browser, reopen `chrome://extensions`, and press **Reload** on
   the extension. The toolbar badge should be blank. `!` means the host or the
   daemon could not be reached.

Chrome identifies an unpacked extension by its path unless the manifest carries
a stable `key`. If you move the repository, reload the extension and rerun the
installer with the new ID.

### Firefox

1. Open `about:debugging#/runtime/this-firefox`, choose **Load Temporary
   Add-on**, and select `extension/manifest.firefox.json`.
2. Install the host once (Firefox uses the fixed ID `stayproductive@local`, so
   no extension ID is needed):

   ```sh
   python3 integrations/browser-extension/install.py --browser firefox
   ```

3. Restart Firefox. Temporary add-ons disappear when Firefox closes; load it
   again after a restart. A signed build for permanent installation is not
   packaged yet.

### Verify the connection

With the daemon running:

```sh
PYTHONPATH=crates/ui python3 - <<'PY'
from stayproductive.engine import Engine
print(Engine().browser_policy())
PY
```

If that succeeds but the badge still shows `!`, check the host manifest path
printed by the installer, confirm the browser was fully restarted, and confirm
the Chrome `allowed_origins` ID matches the extension card. Browser-native
messaging deliberately fails closed: a wrong ID produces "Specified native
messaging host not found" and no rules are applied.

## Privacy

- Page URLs and titles are sent only to the local Unix socket of the daemon for
  the duration of one decision. They are not written to disk by the extension
  or by the native host, and the daemon does not store them for
  `browser_decision`.
- There is no network request, analytics, account, or remote endpoint. The
  extension's permissions are `nativeMessaging`, `declarativeNetRequest`,
  `storage`, `alarms`, and `<all_urls>` (the last is required to see which page
  is about to load).
- `storage.local` keeps only counts (blocks, allows, keywords), the check time,
  and a degraded message; not URLs.
- The native host is a short Python proxy with a fixed command allowlist
  (`browser_policy`, `browser_decision`). It cannot read files for the
  extension, run commands, or open network connections. Its replies are framed
  as native messages; malformed or larger-than-1-MiB frames end the port.

## Known limitations

These are product limitations, not installation mistakes:

- Keyword matching happens after navigation starts, so a blocked page can be
  visible for a moment. `declarativeNetRequest` has no keyword filter.
- Site time limits and in-page surface rules are not enforced by this build.
- Dynamic host rules refresh once a minute, and opportunistically at most every
  30 seconds after a navigation. A profile change can therefore take up to a
  minute to reach the pre-request layer; the next navigation always asks the
  daemon directly.
- An iframe is blocked outright without an explanation page, because browsers
  do not allow a useful redirect UI there.
- Incognito/private windows are covered only if the user explicitly allows the
  extension there.
- The extension does not stop a user who disables it, edits the extension
  directory, or uses a browser where it is not installed. Strict-mode tamper
  resistance remains the daemon's responsibility; see the implementation notes.
- Overlay removal by a hostile page is possible in principle. The daemon still
  blocks the navigation if it is a host rule, and the address bar is never
  rewritten.

## Tests

```sh
node --test integrations/browser-extension/tests/*.test.mjs
python3 -m unittest discover -s integrations/browser-extension/tests -v
```

The Node tests cover exact/suffix host matching, rule priority, redirect
parameters, subframe blocking, and block-page wording. The Python tests cover
native-messaging framing, command and argument filtering, daemon failures,
socket-name parity with the UI, and the installer in a temporary `HOME`.

The extension has also been loaded in Chrome with the native host connected and
`browser_decision` answered by the real daemon; that manual check is recorded in
`docs/parity/feature-matrix.md`. Automated browser automation is not part of the
suite because a headless browser cannot exercise a real toolbar badge or a
native-messaging registration reliably.

## Uninstall

```sh
python3 integrations/browser-extension/install.py --uninstall
```

Then remove the extension from the browser's extensions page. `--uninstall` only
removes the host copy and the manifests this installer wrote; it does not touch
the daemon, profiles, or usage data.
