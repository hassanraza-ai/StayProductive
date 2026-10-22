# Core engine implementation notes

Status: **M0 in progress.** The pure engine, scheduler, period calculator,
usage ledger, and domain model live in `crates/core`. There is no daemon,
SQLite store, UI, or adapter yet; those are tracked in the parity matrix.

These notes record decisions the existing specs left implicit. Anything marked
**TODO(spec)** is deliberately unfinished and must not be read as implemented.

## Implemented and covered by tests

`crates/core/tests/acceptance.rs` mirrors the acceptance IDs:

| Area | Acceptance IDs |
| --- | --- |
| Profiles | `A-PROF-01`, `A-PROF-02`, `A-PROF-03`, `A-PROF-05`, `A-PROF-06` |
| Rules | `A-RULE-01` … `A-RULE-05`, `A-RULE-07`, `A-RULE-08` |
| Limits | `A-LIM-01` … `A-LIM-06` |
| Schedules | `A-SCH-01` … `A-SCH-10` |
| Content | `A-CON-06` |
| Data | `A-DATA-02` (backwards clock jump, commitment end preserved) |

Unit tests additionally cover DST day lengths, the local-date period
calculator, schedule gap/merge behaviour, keyword folding, and ledger
attribution.

## Decisions made in code

### Rule identity

Rules carry a ULID (`AppRuleEntry`, `SiteRule`, `KeywordRule`) even though the
domain-model snippets show bare rule values. `Decision.rule` points at it, so
the UI can explain a block by rule. This is additive and does not change the
behavioural model.

### Launch limits

A limit of *N* allows *N* launches in the period and blocks the *(N+1)*-th.
The anti-thrash window is measured as time **away** from the app: leaving and
returning within 5 s does not count as a new launch. This matches `A-LIM-03`
(“focused twice inside 5 s … counts once; a third launch after 5 s is blocked”).

Usage limits are different: they block when `usage >= limit`, exactly as
`A-LIM-01` requires.

### Wait timers

A wait timer is periodic inside the logical day. Each time cumulative usage
reaches another multiple of `use_for`, access is denied for `wait` from the
crossing instant. With `adaptive`, once half of the wait has elapsed the
remaining wait is halved once. This is the normative definition in
`docs/spec/blocking-engine.md` §4; the adaptive sentence remains **inferred**
from the reference.

### Commitments and strict profiles

An active commitment locks the rules that are active at that instant; it does
not invent new rules. An explicit commitment allowlist excludes the target from
the strict block. Otherwise `R-STRICT` wins over allowlists, pauses, and
expired limits. A strict-eligible profile that is inside its window produces
`R-STRICT` for any target it has a rule for, including when another profile is
paused.

### Start-of-day and DST gaps

`logical_date(now) = date(local_now - start_of_day)`, then period boundaries are
resolved from local dates. A local time inside a DST gap is shifted forward by
the gap length; a window that both starts and ends inside the gap collapses to
zero length and does not run. Ambiguous fall-back times resolve to the earliest
occurrence, so the repeated hour is evaluated once.

### Pickup limits

`RuleSet` carries an optional `pickup_limit` (`CountLimit`). It is not in the
domain-model snippet but is required by `A-LIM-04`; it applies globally to the
profile's targets. The next decision after the limit is reached reports
`R-PICKUP`.

## Not implemented yet

- **`A-RULE-06` / `AppRule::Goal`** — credit earning and `AllowWithNotice` need a
  mutable credit ledger owned by the daemon. TODO(spec): define rounding, credit
  expiry and what is shown while credit remains.
- **`A-PROF-04` templates** — bundled presets are not defined yet.
- **Site time limits** — the extension must first record per-site intervals;
  `SiteAction::Limit` is currently inert rather than guessed at. TODO(spec).
- **Content surfaces** (`A-CON-01`…`A-CON-05`) — the engine has no surface rule
  set or surface matching yet.
- **SQLite persistence and migrations** (`F-DATA-01`) — state is serde-only and
  used for deterministic tests.
- **Daemon, adapters, UI, D-Bus** — everything after the core decision.
- **Low-time alerts** (`A-LIM-07` / `A-PLAT-11`).

## Test commands

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
