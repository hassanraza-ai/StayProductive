# Content filtering

Three layers, independently toggleable: **host rules**, **keyword rules**, and
**in-page surface rules**. A block may come from any layer; the user sees which
one fired.

## 1. Host rules

```rust
struct SiteRule {
    pattern: HostPattern,   // example.com | *.example.com | * (all)
    action: SiteAction,     // Block | Adult | Limit(Duration)
}
```

Matching:

- Inputs are normalized with IDNA2008 to ASCII (punycode), lowercased, trailing
  dot removed; IPv4/IPv6 literals are matched exactly.
- `example.com` matches the host only, not subdomains. `*.example.com` matches
  subdomains and the bare domain.
- URLs are never matched by path, query, or fragment at this layer; those belong
  to keyword rules.
- Allowlist entries are evaluated after commitments and before block rules
  (blocking engine §3). Allowing `good.example.com` under `*.example.com` is
  valid and shows as a child exception in the UI.

## 2. Enforcement modes

| Mode | Coverage | Limits |
| --- | --- | --- |
| **DNS** (system helper) | all browsers and many apps | bypassed by DoH/DoT, VPN, hard-coded IPs, cached answers; per-host only |
| **Browser extension** | supported browsers | only browsers where installed; no coverage of native apps |
| **Both** | best coverage | two failure domains; decisions stay consistent |

The daemon is authoritative: the extension and the resolver ask it (local IPC)
or receive a rule snapshot and apply it. A block decision is never different
between the two paths for the same host at the same time.

Sinkhole behaviour:

- DNS returns `0.0.0.0`/`::` (configurable) or a local block-page address.
- The local block page explains the reason and the end time; it does not
  advertise the product.
- DNS responses set a short TTL (≤ 30 s) so a rule change applies quickly.
- On helper failure, DNS mode reports `Degraded`; the extension continues.

Supported browsers for extension features: Chromium family (Chrome, Chromium,
Brave, Edge, Vivaldi, Opera), Firefox and derivatives. If a browser cannot be
instrumented, the product offers to block it entirely (a normal app rule).

## 3. Keyword rules

- Scope: **URL** (address bar), **search queries** (in-page search boxes and
  the browser's search field), or both.
- Normalization: Unicode NFKC, case folding, collapse whitespace; matching is
  substring on the normalized URL/query.
- Minimum length: 2 characters. Empty or whitespace-only entries are rejected.
- User entries are literal text, not regex. Wildcards are not supported.
- Match precedence is unaffected by allowlists: an allowed host does not
  bypass a keyword rule for a URL on that host unless the keyword rule is
  disabled for it (explicit "allow anyway" per match, logged).
- A match blocks navigation before the request is sent where the extension can
  observe it; otherwise the page is replaced immediately after commit.

## 4. In-page surfaces

Web equivalents of the reference's native surface catalog:

| Surface | Detection |
| --- | --- |
| YouTube Shorts | URL `/shorts/…`, or the shorts player element in the DOM |
| Instagram Reels | URL `/reels/…`, or the reels viewer element |
| Instagram Stories | story viewer container |
| Facebook Reels | `/reel/…` and the reels viewer |
| Snapchat Spotlight (web) | spotlight route/element, where available |
| WhatsApp Channels/Status (web) | channels route; status is not available on web |
| TikTok-style feeds | `/foryou`, full-screen feed container (optional, off by default) |

Rules:

- Toggles are per surface, per profile, default off.
- Detection uses stable structural signals (URL patterns, ARIA roles,
  data attributes) with a versioned selector pack, not brittle pixel checks.
- When a selector pack becomes stale, the extension reports `Degraded` for that
  surface instead of silently allowing it.
- A surface block replaces the surface with the standard block card, leaving the
  rest of the page usable — matching the product's "block the feed, not the app"
  behaviour.

## 5. Adult content

- Opt-in per profile; uses a maintained domain list plus optional category
  lists, resolved and filtered locally.
- Safe-search enforcement is best-effort: DNS CNAMEs where providers support
  it, extension parameters otherwise. The limitations screen lists what is not
  covered.
- The block page for adult content is neutral and does not reveal the rule to
  bystanders.

## 6. Privacy

- Host and URL processing happens on the machine. No page content, URL, or
  title is transmitted.
- Logs store the registrable domain by default, not the full URL; full URLs are
  opt-in and expire with raw events.
- DNS query logs are not written. The helper exposes counters only.

## 7. Known gaps (must be shown in-app)

- **Native desktop clients** (WhatsApp, Telegram, Slack, Discord) are not
  inspectable; surface rules do not apply inside them. Blocking the app is the
  supported alternative.
- **In-app browsers / embedded webviews** may not expose URLs to the extension.
- **DoH/DoT/VPN** bypass DNS filtering; if detected (best effort), the UI warns
  rather than silently failing.
- **Encrypted Client Hello / IP-literal navigation** is out of scope.
- **Mobile-only surfaces** (WhatsApp Status, Snapchat Spotlight in the native
  app) cannot be reproduced.

## 8. Test cases

| Case | Expected |
| --- | --- |
| `example.com` rule, visit `www.example.com` | allowed unless `*.example.com` |
| `*.example.com`, visit `a.b.example.com` | blocked |
| IDN host typed in Unicode and punycode | same decision |
| keyword "game" in `example.com/game` | blocked when URL scope on |
| keyword match on an allowlisted host | blocked, reason `R-KEYWORD` (unless allow-anyway) |
| extension offline, DNS on | host blocked, reason `R-SITE` |
| DNS helper offline, extension on | host blocked in supported browsers, degraded elsewhere |
| selector pack stale | surface reported degraded, no silent allow |
