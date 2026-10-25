//! The StayProductive local state service.
//!
//! For the first runnable UI this binary owns the core `State` and persists it
//! atomically as JSON under `$XDG_DATA_HOME/stayproductive/state.json`. It
//! speaks newline-delimited JSON on stdin/stdout; one request per line and one
//! response per line. The GTK frontend starts it and sends requests.
//!
//! This is deliberately a narrow boundary: replacing stdio with D-Bus later
//! does not move decision logic out of `stayproductive-core`.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use chrono::{NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stayproductive_core::{
    Activation, AppKey, AppRule, ClockSettings, Commitment, CommitmentKind, CountLimit, DateRange,
    DayPattern, DeactivationMethod, Event, HostPattern, HourlyPattern, Keyword, LimitPeriod,
    PauseReason, Profile, ProfileId, ReasonCode, Restriction, RuleSet, Schedule, SiteAction, State,
    Target, Timestamp, UsageLedger, Window, day_period, decide,
};

type DResult<T> = Result<T, String>;

const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Persisted {
    version: u32,
    state: State,
}

struct Daemon {
    state: State,
    path: PathBuf,
    shutdown: bool,
    /// Apps present in the last written policy, to record newly engaged
    /// blocks in the usage history.
    policy_apps: BTreeSet<AppKey>,
    /// Whether the first policy of this run has been written. Blocks that are
    /// already active at startup were recorded before and are not duplicated.
    policy_seeded: bool,
}

fn require_rule_edits_allowed(state: &State, now: Timestamp) -> DResult<()> {
    if state.edits_blocked(now) {
        Err("rules are locked by an active commitment".into())
    } else {
        Ok(())
    }
}

/// A cheap before/after comparison key for the usage ledger. Every structural
/// change also moves the heartbeat, so this catches every persist-worthy edit.
fn usage_fingerprint(
    usage: &UsageLedger,
) -> (
    usize,
    usize,
    usize,
    usize,
    Option<Timestamp>,
    Option<AppKey>,
) {
    (
        usage.intervals().len(),
        usage.launches().len(),
        usage.pickups().len(),
        usage.blocks().len(),
        usage.last_heartbeat(),
        usage.current_app().cloned(),
    )
}

fn main() {
    let path = state_path();
    let mut state = load_state(&path);
    // A crash can leave a foreground interval open; close it at the last
    // heartbeat so restored usage is never over-counted.
    state.recover();
    let mut daemon = Daemon {
        state,
        path,
        shutdown: false,
        policy_apps: BTreeSet::new(),
        policy_seeded: false,
    };
    daemon.refresh_policy(Utc::now());

    println!(
        "{}",
        json!({"event": "ready", "state_path": daemon.path.display().to_string()})
    );
    io::stdout().flush().ok();

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                println!(
                    "{}",
                    json!({"id": 0, "ok": false, "error": format!("invalid JSON: {error}")})
                );
                continue;
            }
        };
        let id = request.get("id").and_then(Value::as_u64).unwrap_or(0);
        let command = request
            .get("cmd")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();

        let response = match daemon.handle(&command, &request) {
            Ok(data) => json!({"id": id, "ok": true, "data": data}),
            Err(error) => json!({"id": id, "ok": false, "error": error}),
        };
        println!("{response}");
        io::stdout().flush().ok();
        if daemon.shutdown {
            break;
        }
    }
}

fn state_path() -> PathBuf {
    if let Ok(path) = env::var("STAYPRODUCTIVE_STATE") {
        return PathBuf::from(path);
    }
    let data_home = env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|_| env::var("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|_| PathBuf::from("."));
    data_home.join("stayproductive/state.json")
}

fn load_state(path: &Path) -> State {
    let Ok(text) = fs::read_to_string(path) else {
        return fresh_state();
    };
    match serde_json::from_str::<Persisted>(&text) {
        Ok(persisted) if persisted.version == FORMAT_VERSION => persisted.state,
        Ok(_) => {
            backup(path);
            fresh_state()
        }
        Err(error) => {
            eprintln!("stayproductive: state file is invalid: {error}");
            backup(path);
            fresh_state()
        }
    }
}

fn backup(path: &Path) {
    let backup = path.with_extension(format!("invalid-{}.json", Utc::now().timestamp()));
    let _ = fs::rename(path, backup);
}

fn write_private(path: &Path, bytes: &[u8]) -> DResult<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let existed = parent.exists();
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if !existed {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn runtime_dir() -> PathBuf {
    env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::temp_dir())
}

/// The heartbeat the extension writes while it is enabled. It is deliberately
/// short-lived so a crashed or disabled extension cannot keep claiming
/// enforcement or keep reporting focus.
struct Heartbeat {
    /// Enabled, policy loaded, and checked within the freshness window.
    fresh: bool,
    /// The block overlay is showing; no usage accrues behind it.
    blocked: bool,
    /// Normalized app ids of the focused window, empty when nothing is
    /// attributable.
    focus: Vec<AppKey>,
    /// When the current focus window became foreground, if the observer knows.
    focus_since: Option<Timestamp>,
}

fn read_heartbeat(now: Timestamp) -> Option<Heartbeat> {
    let path = runtime_dir().join("stayproductive/gnome-extension.json");
    let text = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let active = value.get("active").and_then(Value::as_bool) == Some(true);
    let policy_loaded = value.get("policy_loaded").and_then(Value::as_bool) == Some(true);
    let checked_at = value
        .get("checked_at")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Timestamp>().ok())?;
    let fresh = active
        && policy_loaded
        && now.signed_duration_since(checked_at) <= chrono::TimeDelta::seconds(5);
    let blocked = value
        .get("blocked")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let focus = value
        .get("focus")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .filter_map(|id| AppKey::new(id).ok())
                .collect()
        })
        .unwrap_or_default();
    let focus_since = value
        .get("focus_since")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Timestamp>().ok());
    Some(Heartbeat {
        fresh,
        blocked,
        focus,
        focus_since,
    })
}

/// One app the engine would block right now.
struct PolicyEntry {
    app: AppKey,
    reason: ReasonCode,
    until: Option<Timestamp>,
    profile: Option<ProfileId>,
    profile_name: Option<String>,
}

fn policy_entries(state: &State, now: Timestamp) -> Vec<PolicyEntry> {
    let mut candidates: BTreeSet<AppKey> = BTreeSet::new();
    for profile in &state.profiles {
        for app in profile.rules.apps.keys() {
            candidates.insert(app.clone());
        }
    }
    for block in &state.quick_blocks {
        if let Target::App(app) = &block.target {
            candidates.insert(app.clone());
        }
    }

    let mut entries = Vec::new();
    for app in candidates {
        let event = Event::ForegroundChanged {
            app: app.clone(),
            window: None,
        };
        let decision = decide(state, &event, now);
        if !decision.is_block() {
            continue;
        }
        let profile = decision
            .profile
            .and_then(|id| state.profile(id))
            .map(|profile| profile.name.clone());
        entries.push(PolicyEntry {
            app,
            reason: decision.reason,
            until: decision.until,
            profile: decision.profile,
            profile_name: profile,
        });
    }
    entries
}

fn fresh_state() -> State {
    State::new(ClockSettings::new(NaiveTime::MIN, system_timezone()))
}

fn system_timezone() -> Tz {
    if let Ok(value) = env::var("TZ") {
        if let Ok(tz) = value.parse::<Tz>() {
            return tz;
        }
    }
    if let Ok(value) = iana_time_zone::get_timezone() {
        if let Ok(tz) = value.parse::<Tz>() {
            return tz;
        }
    }
    if let Ok(value) = fs::read_to_string("/etc/timezone") {
        if let Ok(tz) = value.trim().parse::<Tz>() {
            return tz;
        }
    }
    chrono_tz::UTC
}

impl Daemon {
    fn handle(&mut self, command: &str, request: &Value) -> DResult<Value> {
        let now = Utc::now();
        let tracked = self.track_usage(now);
        let command_dirty = !matches!(command, "snapshot" | "shutdown");
        let result =
            match command {
                "snapshot" => Ok(self.snapshot()),
                "create_profile" => {
                    require_rule_edits_allowed(&self.state, now)?;
                    let spec: ProfileSpec = from_request(request, "profile")?;
                    let name = spec.name.clone();
                    let mut profile = Profile::new(name, now).map_err(|e| e.to_string())?;
                    apply_profile_spec(&mut profile, &spec, now)?;
                    self.state.add_profile(profile).map_err(|e| e.to_string())?;
                    Ok(self.snapshot())
                }
                "update_profile" => {
                    require_rule_edits_allowed(&self.state, now)?;
                    let id = parse_ulid(request, "profile_id")?;
                    let spec: ProfileSpec = from_request(request, "profile")?;
                    let new_name = spec.name.trim().to_owned();
                    if self.state.profiles.iter().any(|profile| {
                        profile.id != id && profile.name.eq_ignore_ascii_case(&new_name)
                    }) {
                        return Err("profile name must be unique".into());
                    }
                    {
                        let profile = self
                            .state
                            .profile_mut(id)
                            .ok_or_else(|| "profile not found".to_owned())?;
                        let mut updated = profile.clone();
                        apply_profile_spec(&mut updated, &spec, now)?;
                        *profile = updated;
                    }
                    Ok(self.snapshot())
                }
                "delete_profile" => {
                    require_rule_edits_allowed(&self.state, now)?;
                    let id = parse_ulid(request, "profile_id")?;
                    self.state.remove_profile(id).map_err(|e| e.to_string())?;
                    Ok(self.snapshot())
                }
                "activate_profile" => {
                    require_rule_edits_allowed(&self.state, now)?;
                    let id = parse_ulid(request, "profile_id")?;
                    let profile = self
                        .state
                        .profile(id)
                        .ok_or_else(|| "profile not found".to_owned())?;
                    if profile.activation != Activation::Manual {
                        return Err("only manual profiles can be activated directly".into());
                    }
                    self.state
                        .activate_manual(id, now)
                        .map_err(|e| e.to_string())?;
                    Ok(self.snapshot())
                }
                "deactivate_profile" => {
                    require_rule_edits_allowed(&self.state, now)?;
                    self.state.deactivate_manual(now);
                    Ok(self.snapshot())
                }
                "pause_profile" => {
                    let id = parse_ulid(request, "profile_id")?;
                    let minutes = request.get("minutes").and_then(Value::as_u64).unwrap_or(0);
                    let until = if minutes == 0 {
                        None
                    } else {
                        Some(now + chrono::TimeDelta::minutes(minutes as i64))
                    };
                    self.state
                        .pause_profile(id, until, PauseReason::User, now)
                        .map_err(|e| e.to_string())?;
                    Ok(self.snapshot())
                }
                "resume_profile" => {
                    let id = parse_ulid(request, "profile_id")?;
                    self.state
                        .resume_profile(id, now)
                        .map_err(|e| e.to_string())?;
                    Ok(self.snapshot())
                }
                "quick_block" => {
                    let kind = request.get("kind").and_then(Value::as_str).unwrap_or("app");
                    let value = request
                        .get("target")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing target".to_owned())?;
                    let minutes = request
                        .get("minutes")
                        .and_then(Value::as_u64)
                        .unwrap_or(10)
                        .max(1);
                    let target = match kind {
                        "site" => Target::Site(
                            HostPattern::new(value)
                                .map_err(|e| e.to_string())?
                                .host()
                                .clone(),
                        ),
                        _ => Target::App(AppKey::new(value).map_err(|e| e.to_string())?),
                    };
                    self.state
                        .quick_block(
                            target,
                            now + chrono::TimeDelta::minutes(minutes as i64),
                            now,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(self.snapshot())
                }
                "clear_quick_block" => {
                    let kind = request.get("kind").and_then(Value::as_str).unwrap_or("app");
                    let value = request
                        .get("target")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing target".to_owned())?;
                    let target = match kind {
                        "site" => Target::Site(
                            HostPattern::new(value)
                                .map_err(|e| e.to_string())?
                                .host()
                                .clone(),
                        ),
                        _ => Target::App(AppKey::new(value).map_err(|e| e.to_string())?),
                    };
                    self.state.clear_quick_block(&target);
                    Ok(self.snapshot())
                }
                "start_strict" => {
                    let minutes = request
                        .get("minutes")
                        .and_then(Value::as_u64)
                        .unwrap_or(30)
                        .max(1);
                    let cooldown = request
                        .get("cooldown_minutes")
                        .and_then(Value::as_u64)
                        .unwrap_or(10)
                        .max(1);
                    let commitment = Commitment::new(
                        CommitmentKind::Strict,
                        now,
                        Some(now + chrono::TimeDelta::minutes(minutes as i64)),
                        DeactivationMethod::Cooldown(Duration::from_secs(cooldown * 60)),
                    )
                    .with_restrictions([Restriction::BlockEdits, Restriction::BlockPause]);
                    self.state
                        .add_commitment(commitment)
                        .map_err(|e| e.to_string())?;
                    Ok(self.snapshot())
                }
                "request_commitment_end" => {
                    let id = parse_ulid(request, "commitment_id")?;
                    let end = self
                        .state
                        .request_commitment_end(id, now)
                        .map_err(|e| e.to_string())?;
                    Ok(json!({"cooldown_ends_at": end, "snapshot": self.snapshot()}))
                }
                "finish_commitment_end" => {
                    let id = parse_ulid(request, "commitment_id")?;
                    self.state
                        .finish_commitment_end(id, now)
                        .map_err(|e| e.to_string())?;
                    Ok(self.snapshot())
                }
                "set_settings" => {
                    let start = request
                        .get("start_of_day")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "missing start_of_day".to_owned())?;
                    let start_of_day = parse_time(start)?;
                    let timezone = request
                        .get("timezone")
                        .and_then(Value::as_str)
                        .unwrap_or("UTC");
                    let timezone = Tz::from_str(timezone)
                        .map_err(|_| format!("unknown time zone: {timezone}"))?;
                    self.state.settings.start_of_day = start_of_day;
                    self.state.settings.timezone = timezone;
                    Ok(self.snapshot())
                }
                "shutdown" => {
                    self.shutdown = true;
                    Ok(json!({"bye": true}))
                }
                other => return Err(format!("unknown command: {other}")),
            };

        if result.is_ok() {
            let recorded = self.refresh_policy(now);
            if command_dirty || tracked || recorded {
                self.persist()?;
            }
        }
        result
    }

    /// Feed the newest extension heartbeat into the usage ledger.
    ///
    /// Usage accrues only while a fresh observer reports an attributable,
    /// unblocked focus window. Without an observer, or behind the block
    /// overlay, the open interval is closed at `now`. Returns whether the
    /// ledger changed, so the caller knows to persist.
    fn track_usage(&mut self, now: Timestamp) -> bool {
        let heartbeat = read_heartbeat(now);
        let focus = match &heartbeat {
            Some(hb) if hb.fresh && !hb.blocked && !hb.focus.is_empty() => {
                // Attribution must use the profile's own app key, or the
                // engine's usage queries would look at a different app.
                self.resolve_focus(&hb.focus)
                    .map(|app| (app, hb.focus_since))
            }
            _ => None,
        };
        let usage = &mut self.state.usage;
        let before = usage_fingerprint(usage);
        match focus {
            Some((app, since)) => {
                // An interval can only start at or after the last
                // observation; an older `focus_since` (for example after a
                // block ended) would re-attribute time we already closed.
                // With no prior observation the observer's word is all we
                // have, so its `focus_since` is trusted.
                let mut at = since.unwrap_or(now);
                if let Some(floor) = usage.last_heartbeat() {
                    at = at.max(floor);
                }
                if at > now {
                    at = now;
                }
                if usage.current_app() == Some(&app) {
                    match usage.current_since() {
                        Some(start) if at <= start => usage.heartbeat(now),
                        // The focus left and returned within one sampling
                        // gap; close and reopen at the reported instant.
                        Some(_) => {
                            usage.close_foreground(at);
                            usage.foreground(app, at);
                        }
                        None => usage.foreground(app, at),
                    }
                } else {
                    usage.foreground(app, at);
                }
            }
            None => {
                if usage.current_app().is_some() {
                    usage.close_foreground(now);
                }
            }
        }
        let after = usage_fingerprint(&self.state.usage);
        before != after
    }

    /// Map the ids a window reported to the app key a profile rules over.
    ///
    /// The extension already expands trailing-component aliases, but a
    /// profile may use a desktop id while the window reports only the
    /// WM_CLASS or the other way round, so both tails are matched. Apps no
    /// profile rules over are not tracked.
    fn resolve_focus(&self, ids: &[AppKey]) -> Option<AppKey> {
        for id in ids {
            let id_tail = id.as_str().rsplit('.').next().unwrap_or_default();
            for profile in &self.state.profiles {
                for key in profile.rules.apps.keys() {
                    let key_tail = key.as_str().rsplit('.').next().unwrap_or_default();
                    if key == id
                        || (!key_tail.is_empty() && key_tail == id.as_str())
                        || (!id_tail.is_empty() && id_tail == key.as_str())
                    {
                        return Some(key.clone());
                    }
                }
            }
        }
        None
    }

    /// Write the enforcement policy and record newly engaged blocks.
    ///
    /// Returns whether any block record was added, so the caller knows to
    /// persist. The first policy of a run only seeds the comparison set: a
    /// block that is already active at startup was recorded by the run that
    /// engaged it.
    fn refresh_policy(&mut self, now: Timestamp) -> bool {
        let entries = policy_entries(&self.state, now);
        let mut recorded = false;
        let mut current = BTreeSet::new();
        for entry in &entries {
            current.insert(entry.app.clone());
            if self.policy_seeded && !self.policy_apps.contains(&entry.app) {
                self.state.usage.blocked(
                    Target::App(entry.app.clone()),
                    entry.reason,
                    now,
                    entry.until,
                    entry.profile,
                );
                recorded = true;
            }
        }
        self.policy_apps = current;
        self.policy_seeded = true;

        let apps: Vec<Value> = entries
            .iter()
            .map(|entry| {
                json!({
                    "app": entry.app.as_str(),
                    "reason": entry.reason.as_str(),
                    "until": entry.until,
                    "profile": entry.profile_name,
                })
            })
            .collect();
        let policy = json!({
            "version": 1,
            "generated_at": now,
            "valid_until": now + chrono::TimeDelta::seconds(30),
            "apps": apps,
        });
        let Ok(encoded) = serde_json::to_vec_pretty(&policy) else {
            eprintln!("stayproductive: could not encode enforcement policy");
            return recorded;
        };
        if let Err(error) = write_private(&self.policy_path(), &encoded) {
            eprintln!("stayproductive: could not write enforcement policy: {error}");
        }
        recorded
    }

    fn persist(&self) -> DResult<()> {
        let payload = Persisted {
            version: FORMAT_VERSION,
            state: self.state.clone(),
        };
        let encoded = serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?;
        write_private(&self.path, &encoded)
    }

    fn policy_path(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("enforcement.json")
    }

    fn usage_today(&self, now: Timestamp) -> Value {
        let day = day_period(now, &self.state.settings);
        let mut rows: Vec<(String, u64)> = self
            .state
            .usage
            .usage_per_app(&day, now)
            .into_iter()
            .map(|(app, duration)| (app.as_str().to_owned(), duration.as_secs()))
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Value::Array(
            rows.into_iter()
                .map(|(app, seconds)| json!({"app": app, "seconds": seconds}))
                .collect(),
        )
    }

    fn snapshot(&self) -> Value {
        let now = Utc::now();
        let heartbeat = read_heartbeat(now).is_some_and(|heartbeat| heartbeat.fresh);
        let enforcement = if heartbeat {
            json!({
                "state": "active",
                "adapter": "gnome-extension",
                "message": "The GNOME Shell extension is enforcing app blocks for native Wayland and XWayland windows.",
                "policy_path": self.policy_path().display().to_string(),
            })
        } else {
            json!({
                "state": "unavailable",
                "adapter": "gnome-extension",
                "message": "No enforcement adapter is reporting. The policy file is ready; install and enable the GNOME extension to apply it.",
                "policy_path": self.policy_path().display().to_string(),
            })
        };
        let profiles: Vec<Value> = self
            .state
            .profiles
            .iter()
            .map(|profile| profile_view(&self.state, profile, now))
            .collect();
        let active: Vec<Value> = self
            .state
            .profiles
            .iter()
            .filter(|profile| self.state.profile_is_active(profile, now))
            .map(|profile| {
                json!({
                    "id": profile.id.to_string(),
                    "name": profile.name,
                    "ends_at": self.state.profile_end(profile, now),
                    "paused": profile.pause.is_paused(now),
                })
            })
            .collect();
        let next_boundary = self.next_boundary(now);
        let commitment = self.state.active_commitment(now).map(commitment_view);
        let history: Vec<Value> = self
            .state
            .usage
            .blocks()
            .iter()
            .rev()
            .take(30)
            .map(|block| {
                json!({
                    "at": block.at,
                    "target": block.target.key(),
                    "reason": block.reason.as_str(),
                    "until": block.until,
                })
            })
            .collect();

        json!({
            "now": now,
            "settings": {
                "start_of_day": self.state.settings.start_of_day.format("%H:%M").to_string(),
                "timezone": self.state.settings.timezone.name(),
            },
            "manual_active": self.state.manual_active.map(|id| id.to_string()),
            "profiles": profiles,
            "active_profiles": active,
            "next_boundary": next_boundary,
            "commitment": commitment,
            "quick_blocks": self.state.quick_blocks.iter().map(|block| json!({
                "target": block.target.key(),
                "kind": match &block.target { Target::App(_) => "app", Target::Site(_) => "site" },
                "until": block.until,
            })).collect::<Vec<_>>(),
            "history": history,
            "usage_today": self.usage_today(now),
            "enforcement": enforcement,
            "state_path": self.path.display().to_string(),
        })
    }

    fn next_boundary(&self, now: Timestamp) -> Option<Timestamp> {
        let mut candidates: Vec<Timestamp> = Vec::new();
        for profile in &self.state.profiles {
            if !self.state.profile_is_active(profile, now) {
                continue;
            }
            if let Some(end) = self.state.profile_end(profile, now) {
                candidates.push(end);
            }
        }
        for commitment in &self.state.commitments {
            if commitment.is_enforcing(now) {
                if let Some(end) = commitment.ends_at {
                    candidates.push(end);
                }
            }
        }
        for block in &self.state.quick_blocks {
            if block.until > now {
                candidates.push(block.until);
            }
        }
        candidates.into_iter().min()
    }
}

fn from_request<T: for<'de> Deserialize<'de>>(request: &Value, key: &str) -> DResult<T> {
    let value = request
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing {key}"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid {key}: {e}"))
}

fn parse_ulid(request: &Value, key: &str) -> DResult<ulid::Ulid> {
    let value = request
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))?;
    ulid::Ulid::from_str(value).map_err(|e| format!("invalid {key}: {e}"))
}

fn parse_time(value: &str) -> DResult<NaiveTime> {
    NaiveTime::parse_from_str(value, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(value, "%H:%M:%S"))
        .map_err(|_| format!("invalid time: {value}"))
}

fn parse_date(value: &str) -> DResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| format!("invalid date: {value}"))
}

fn parse_period(value: Option<&str>) -> LimitPeriod {
    match value.unwrap_or("daily") {
        "hourly" => LimitPeriod::Hourly,
        _ => LimitPeriod::Daily,
    }
}

#[derive(Default, Deserialize)]
struct ProfileSpec {
    #[serde(default)]
    name: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_priority")]
    priority: u8,
    #[serde(default)]
    activation: String,
    #[serde(default)]
    strict_eligible: bool,
    #[serde(default)]
    schedule: Option<ScheduleSpec>,
    #[serde(default)]
    apps: Vec<AppSpec>,
    #[serde(default)]
    sites: Vec<SiteSpec>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    allow_apps: Vec<String>,
    #[serde(default)]
    allow_sites: Vec<String>,
    #[serde(default)]
    pickup_limit: Option<PickupSpec>,
}

fn default_true() -> bool {
    true
}

fn default_priority() -> u8 {
    5
}

#[derive(Default, Deserialize)]
struct ScheduleSpec {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    windows: Vec<WindowSpec>,
    #[serde(default)]
    days: Vec<String>,
    #[serde(default)]
    on_minutes: Option<u64>,
    #[serde(default)]
    off_minutes: Option<u64>,
    #[serde(default)]
    start_date: Option<String>,
    #[serde(default)]
    end_date: Option<String>,
}

#[derive(Deserialize)]
struct WindowSpec {
    start: String,
    end: String,
}

#[derive(Default, Deserialize)]
struct AppSpec {
    key: String,
    action: String,
    #[serde(default)]
    minutes: Option<u64>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    per: Option<String>,
    #[serde(default)]
    use_for_minutes: Option<u64>,
    #[serde(default)]
    wait_minutes: Option<u64>,
    #[serde(default)]
    adaptive: Option<bool>,
    #[serde(default)]
    earn_for: Option<String>,
}

#[derive(Deserialize)]
struct SiteSpec {
    pattern: String,
    action: String,
    #[serde(default)]
    minutes: Option<u64>,
}

#[derive(Deserialize)]
struct PickupSpec {
    count: u32,
    #[serde(default)]
    per: Option<String>,
}

fn apply_profile_spec(profile: &mut Profile, spec: &ProfileSpec, now: Timestamp) -> DResult<()> {
    profile.name = spec.name.trim().to_owned();
    profile.enabled = spec.enabled;
    profile.priority = spec.priority.min(9);
    profile.activation = match spec.activation.as_str() {
        "scheduled" => Activation::Scheduled,
        "always" => Activation::Always,
        _ => Activation::Manual,
    };
    profile.strict_eligible = spec.strict_eligible;
    profile.schedule = spec.schedule.as_ref().map(schedule_from_spec).transpose()?;
    profile.rules = rules_from_spec(spec)?;
    profile.updated_at = now;
    profile.validate().map_err(|e| e.to_string())
}

fn rules_from_spec(spec: &ProfileSpec) -> DResult<RuleSet> {
    let mut rules = RuleSet::default();
    for entry in &spec.apps {
        let key = AppKey::new(&entry.key).map_err(|e| e.to_string())?;
        let rule = match entry.action.as_str() {
            "block" => AppRule::Block,
            "daily_limit" => AppRule::DailyLimit {
                limit: minutes(entry.minutes.unwrap_or(30)),
            },
            "hourly_limit" => AppRule::HourlyLimit {
                limit: minutes(entry.minutes.unwrap_or(15)),
            },
            "launch_limit" => AppRule::LaunchLimit {
                count: entry.count.unwrap_or(1).max(1),
                per: parse_period(entry.per.as_deref()),
            },
            "wait_timer" => AppRule::WaitTimer {
                use_for: minutes(entry.use_for_minutes.unwrap_or(25)),
                wait: minutes(entry.wait_minutes.unwrap_or(5)),
                adaptive: entry.adaptive.unwrap_or(false),
            },
            "goal" => AppRule::Goal {
                earn_for: AppKey::new(
                    entry
                        .earn_for
                        .as_deref()
                        .ok_or_else(|| "goal rule needs earn_for".to_owned())?,
                )
                .map_err(|e| e.to_string())?,
                required: minutes(entry.minutes.unwrap_or(30)),
                grant: minutes(entry.count.unwrap_or(10) as u64),
                earned: Duration::ZERO,
            },
            other => return Err(format!("unknown app rule: {other}")),
        };
        rules.set_app_rule(key, rule);
    }
    for entry in &spec.sites {
        let pattern = HostPattern::new(&entry.pattern).map_err(|e| e.to_string())?;
        let action = match entry.action.as_str() {
            "block" => SiteAction::Block,
            "adult" => SiteAction::Adult,
            "limit" | "daily_limit" => SiteAction::Limit(minutes(entry.minutes.unwrap_or(30))),
            other => return Err(format!("unknown site rule: {other}")),
        };
        rules.set_site_rule(pattern, action);
    }
    for text in &spec.keywords {
        rules.add_keyword(
            Keyword::new(text).map_err(|e| e.to_string())?,
            stayproductive_core::KeywordScope::Both,
        );
    }
    for app in &spec.allow_apps {
        rules.allow_app(AppKey::new(app).map_err(|e| e.to_string())?);
    }
    for site in &spec.allow_sites {
        rules.allow_site(HostPattern::new(site).map_err(|e| e.to_string())?);
    }
    if let Some(pickup) = &spec.pickup_limit {
        rules.pickup_limit = Some(
            CountLimit::new(pickup.count, parse_period(pickup.per.as_deref()))
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(rules)
}

fn schedule_from_spec(spec: &ScheduleSpec) -> DResult<Schedule> {
    let windows: Vec<Window> = spec
        .windows
        .iter()
        .map(|window| {
            Ok(Window::new(
                parse_time(&window.start)?,
                parse_time(&window.end)?,
            ))
        })
        .collect::<DResult<_>>()?;
    let days = days_from_spec(&spec.days)?;
    match spec.kind.as_str() {
        "daily" => Schedule::daily(windows),
        "intervals" => Schedule::intervals(windows),
        "days" | "weekdays" => Schedule::days(days, windows),
        "full_day" => Schedule::full_day(days),
        "hourly" => {
            let on = spec.on_minutes.unwrap_or(50).clamp(1, 60);
            let off = spec.off_minutes.unwrap_or(60 - on);
            Schedule::hourly(
                days,
                HourlyPattern::new(Duration::from_secs(on * 60), Duration::from_secs(off * 60))
                    .map_err(|e| e.to_string())?,
            )
        }
        "one_time" => {
            let start = parse_date(
                spec.start_date
                    .as_deref()
                    .ok_or_else(|| "one-time schedule needs start_date".to_owned())?,
            )?;
            let end = parse_date(
                spec.end_date
                    .as_deref()
                    .ok_or_else(|| "one-time schedule needs end_date".to_owned())?,
            )?;
            Schedule::one_time(
                DateRange::new(start, end).map_err(|e| e.to_string())?,
                windows,
            )
        }
        other => return Err(format!("unknown schedule kind: {other}")),
    }
    .map_err(|e| e.to_string())
}

fn days_from_spec(days: &[String]) -> DResult<DayPattern> {
    if days.is_empty() {
        return Ok(DayPattern::EveryDay);
    }
    let weekdays = days
        .iter()
        .map(|day| match day.to_ascii_lowercase().as_str() {
            "mon" | "monday" => Ok(chrono::Weekday::Mon),
            "tue" | "tuesday" => Ok(chrono::Weekday::Tue),
            "wed" | "wednesday" => Ok(chrono::Weekday::Wed),
            "thu" | "thursday" => Ok(chrono::Weekday::Thu),
            "fri" | "friday" => Ok(chrono::Weekday::Fri),
            "sat" | "saturday" => Ok(chrono::Weekday::Sat),
            "sun" | "sunday" => Ok(chrono::Weekday::Sun),
            other => Err(format!("unknown weekday: {other}")),
        })
        .collect::<DResult<Vec<_>>>()?;
    Ok(DayPattern::Weekdays(weekdays))
}

fn minutes(value: u64) -> Duration {
    Duration::from_secs(value * 60)
}

fn profile_view(state: &State, profile: &Profile, now: Timestamp) -> Value {
    let apps: Vec<Value> = profile
        .rules
        .apps
        .iter()
        .map(|(key, entry)| app_rule_view(key.as_str(), &entry.rule))
        .collect();
    let sites: Vec<Value> = profile
        .rules
        .sites
        .iter()
        .map(|rule| {
            json!({
                "pattern": rule.pattern.to_string(),
                "action": match rule.action {
                    SiteAction::Block => "block",
                    SiteAction::Adult => "adult",
                    SiteAction::Limit(_) => "daily_limit",
                },
                "minutes": match rule.action {
                    SiteAction::Limit(limit) => Some(limit.as_secs() / 60),
                    _ => None,
                },
            })
        })
        .collect();
    let keywords: Vec<&str> = profile
        .rules
        .keywords
        .iter()
        .map(|keyword| keyword.text.as_str())
        .collect();
    let schedule = profile.schedule.as_ref().map(schedule_view);
    let status = schedule_status(profile, state, now);

    json!({
        "id": profile.id.to_string(),
        "name": profile.name,
        "enabled": profile.enabled,
        "priority": profile.priority,
        "activation": match profile.activation {
            Activation::Manual => "manual",
            Activation::Scheduled => "scheduled",
            Activation::Always => "always",
        },
        "strict_eligible": profile.strict_eligible,
        "paused": profile.pause.is_paused(now),
        "paused_until": profile.pause.paused_until,
        "schedule": schedule,
        "apps": apps,
        "sites": sites,
        "keywords": keywords,
        "allow_apps": profile.rules.allow_apps.iter().map(|app| app.as_str()).collect::<Vec<_>>(),
        "allow_sites": profile.rules.allow_sites.iter().map(|site| site.to_string()).collect::<Vec<_>>(),
        "pickup_limit": profile.rules.pickup_limit.map(|limit| json!({
            "count": limit.count,
            "per": match limit.per { LimitPeriod::Daily => "daily", LimitPeriod::Hourly => "hourly" },
        })),
        "active": state.profile_is_active(profile, now),
        "ends_at": state.profile_end(profile, now),
        "status": status,
    })
}

fn app_rule_view(key: &str, rule: &AppRule) -> Value {
    match rule {
        AppRule::Block => json!({"key": key, "action": "block"}),
        AppRule::DailyLimit { limit } => {
            json!({"key": key, "action": "daily_limit", "minutes": limit.as_secs() / 60})
        }
        AppRule::HourlyLimit { limit } => {
            json!({"key": key, "action": "hourly_limit", "minutes": limit.as_secs() / 60})
        }
        AppRule::LaunchLimit { count, per } => json!({
            "key": key,
            "action": "launch_limit",
            "count": count,
            "per": match per { LimitPeriod::Daily => "daily", LimitPeriod::Hourly => "hourly" },
        }),
        AppRule::WaitTimer {
            use_for,
            wait,
            adaptive,
        } => json!({
            "key": key,
            "action": "wait_timer",
            "use_for_minutes": use_for.as_secs() / 60,
            "wait_minutes": wait.as_secs() / 60,
            "adaptive": adaptive,
        }),
        AppRule::Goal {
            earn_for,
            required,
            grant,
            ..
        } => json!({
            "key": key,
            "action": "goal",
            "earn_for": earn_for.as_str(),
            "minutes": required.as_secs() / 60,
            "count": grant.as_secs() / 60,
        }),
    }
}

fn schedule_view(schedule: &Schedule) -> Value {
    let kind = match schedule.kind {
        stayproductive_core::ScheduleKind::Daily => "daily",
        stayproductive_core::ScheduleKind::Days => "days",
        stayproductive_core::ScheduleKind::Intervals => "intervals",
        stayproductive_core::ScheduleKind::FullDay => "full_day",
        stayproductive_core::ScheduleKind::Hourly => "hourly",
        stayproductive_core::ScheduleKind::OneTime => "one_time",
    };
    let days: Vec<String> = match &schedule.days {
        DayPattern::EveryDay => Vec::new(),
        DayPattern::Weekdays(days) => days
            .iter()
            .map(|day| weekday_name(*day).to_owned())
            .collect(),
        DayPattern::Custom(days) => days
            .iter()
            .map(|(day, _)| weekday_name(*day).to_owned())
            .collect(),
    };
    let windows: Vec<Value> = schedule
        .windows
        .iter()
        .map(|window| {
            json!({
                "start": window.start.format("%H:%M").to_string(),
                "end": window.end.format("%H:%M").to_string(),
            })
        })
        .collect();
    json!({
        "kind": kind,
        "days": days,
        "windows": windows,
        "on_minutes": schedule.hourly.map(|pattern| pattern.on.as_secs() / 60),
        "off_minutes": schedule.hourly.map(|pattern| pattern.off.as_secs() / 60),
        "start_date": schedule.date_range.map(|range| range.start.to_string()),
        "end_date": schedule.date_range.map(|range| range.end.to_string()),
    })
}

fn weekday_name(day: chrono::Weekday) -> &'static str {
    match day {
        chrono::Weekday::Mon => "Mon",
        chrono::Weekday::Tue => "Tue",
        chrono::Weekday::Wed => "Wed",
        chrono::Weekday::Thu => "Thu",
        chrono::Weekday::Fri => "Fri",
        chrono::Weekday::Sat => "Sat",
        chrono::Weekday::Sun => "Sun",
    }
}

fn schedule_status(profile: &Profile, state: &State, now: Timestamp) -> Value {
    let Some(schedule) = &profile.schedule else {
        return json!({"active": state.profile_is_active(profile, now), "ends_at": null, "next_start": null});
    };
    let status = schedule.status(now, state.settings.timezone);
    json!({
        "active": status.active,
        "ends_at": status.ends_at,
        "next_start": status.next_start,
    })
}

fn commitment_view(commitment: &Commitment) -> Value {
    json!({
        "id": commitment.id.to_string(),
        "kind": match commitment.kind { CommitmentKind::Strict => "strict", CommitmentKind::Lock => "lock" },
        "state": match commitment.state {
            stayproductive_core::CommitmentState::Armed => "armed",
            stayproductive_core::CommitmentState::Active => "active",
            stayproductive_core::CommitmentState::CoolingDown => "cooling_down",
            stayproductive_core::CommitmentState::ReadyToEnd => "ready_to_end",
            stayproductive_core::CommitmentState::Ended => "ended",
        },
        "starts_at": commitment.starts_at,
        "ends_at": commitment.ends_at,
        "cooldown_ends_at": commitment.cooldown_ends_at,
        "method": match &commitment.method {
            DeactivationMethod::Cooldown(duration) => json!({"kind": "cooldown", "minutes": duration.as_secs() / 60}),
            DeactivationMethod::Text(text) => json!({"kind": "text", "text": text}),
            DeactivationMethod::Qr(id) => json!({"kind": "qr", "id": id}),
            DeactivationMethod::Approval(id) => json!({"kind": "approval", "id": id}),
        },
        "restrictions": commitment.restrictions.iter().map(|restriction| match restriction {
            Restriction::BlockSettings => "block_settings",
            Restriction::BlockEdits => "block_edits",
            Restriction::PasswordGate => "password_gate",
            Restriction::BlockPause => "block_pause",
        }).collect::<Vec<_>>(),
    })
}
