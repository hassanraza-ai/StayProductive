//! The StayProductive local state service.
//!
//! The daemon owns the core `State` and persists it atomically as JSON under
//! `$XDG_DATA_HOME/stayproductive/state.json`. It speaks newline-delimited JSON:
//! one request per line and one response per line.
//!
//! Two transports are supported:
//!
//! - `--socket [PATH]` — the normal desktop transport. The daemon binds a
//!   same-user Unix socket in `$XDG_RUNTIME_DIR/stayproductive/`, refreshes the
//!   enforcement policy on its own, and keeps running after the GTK window
//!   closes. systemd starts it at login and restarts it if it is killed.
//! - stdio (default) — kept for the protocol tests and one-shot tooling. The
//!   process exits with its parent.
//!
//! This is deliberately a narrow boundary: replacing it with D-Bus later does
//! not move decision logic out of `stayproductive-core`.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::{NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stayproductive_core::{
    Activation, AppKey, AppRule, ClockSettings, Commitment, CommitmentKind, CountLimit, DateRange,
    DayPattern, DeactivationMethod, Event, HostPattern, HourlyPattern, Keyword, LimitPeriod,
    PauseReason, Profile, Restriction, RuleSet, Schedule, SiteAction, State, Target, Timestamp,
    Window, decide,
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
}

fn require_rule_edits_allowed(state: &State, now: Timestamp) -> DResult<()> {
    if state.edits_blocked(now) {
        Err("rules are locked by an active commitment".into())
    } else {
        Ok(())
    }
}

fn main() {
    let mode = match parse_args(env::args().skip(1).collect()) {
        Ok(mode) => mode,
        Err(error) => {
            eprintln!("stayproductive-daemon: {error}");
            eprintln!("usage: stayproductive-daemon [--stdio | --socket [PATH]]");
            std::process::exit(2);
        }
    };
    match mode {
        Mode::Stdio => run_stdio(),
        Mode::Socket(path) => run_socket(path),
    }
}

enum Mode {
    Stdio,
    Socket(PathBuf),
}

fn parse_args(args: Vec<String>) -> Result<Mode, String> {
    let mut mode = Mode::Stdio;
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].as_str();
        match argument {
            "--stdio" => {
                mode = Mode::Stdio;
                index += 1;
            }
            "--socket" => match args.get(index + 1).filter(|value| !value.starts_with('-')) {
                Some(path) => {
                    mode = Mode::Socket(PathBuf::from(path));
                    index += 2;
                }
                None => {
                    mode = Mode::Socket(default_socket_path());
                    index += 1;
                }
            },
            "--help" | "-h" => {
                println!("usage: stayproductive-daemon [--stdio | --socket [PATH]]");
                std::process::exit(0);
            }
            other if other.starts_with("--socket=") => {
                mode = Mode::Socket(PathBuf::from(&other["--socket=".len()..]));
                index += 1;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(mode)
}

/// Same-user control socket. A custom state file gets its own socket so a test
/// or portable instance never talks to the login service's daemon.
fn default_socket_path() -> PathBuf {
    if let Ok(path) = env::var("STAYPRODUCTIVE_SOCKET") {
        return PathBuf::from(path);
    }
    let mut name = String::from("daemon");
    if let Ok(state) = env::var("STAYPRODUCTIVE_STATE") {
        if !state.is_empty() {
            name.push('-');
            name.push_str(&fnv1a_hex(state.as_bytes()));
        }
    }
    runtime_dir()
        .join("stayproductive")
        .join(format!("{name}.sock"))
}

/// Stable 64-bit FNV-1a, mirrored by `engine.py` for the same socket name.
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn run_stdio() {
    let mut daemon = Daemon::new(state_path());
    daemon.write_policy(Utc::now());

    println!("{}", daemon.greeting());
    io::stdout().flush().ok();

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = daemon.respond(&line);
        println!("{response}");
        io::stdout().flush().ok();
        if daemon.shutdown {
            break;
        }
    }
}

/// Background transport. One daemon per state file; the socket is mode `0600`
/// inside a `0700` directory, so only the same user can talk to it.
fn run_socket(socket_path: PathBuf) {
    if let Some(parent) = socket_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let existed = parent.exists();
        if let Err(error) = fs::create_dir_all(parent) {
            eprintln!(
                "stayproductive-daemon: could not create {}: {error}",
                parent.display()
            );
            std::process::exit(1);
        }
        #[cfg(unix)]
        if !existed {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }

    if UnixStream::connect(&socket_path).is_ok() {
        eprintln!(
            "stayproductive-daemon: already running at {}",
            socket_path.display()
        );
        return;
    }
    let _ = fs::remove_file(&socket_path);

    let listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            eprintln!(
                "stayproductive-daemon: already running at {}",
                socket_path.display()
            );
            return;
        }
        Err(error) => {
            eprintln!(
                "stayproductive-daemon: could not bind {}: {error}",
                socket_path.display()
            );
            std::process::exit(1);
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600));
    }

    let daemon = Arc::new(Mutex::new(Daemon::new(state_path())));
    if let Ok(daemon) = daemon.lock() {
        daemon.write_policy(Utc::now());
    }
    eprintln!(
        "stayproductive-daemon: listening on {}",
        socket_path.display()
    );

    // The window may be closed for hours. Refreshing the policy from the daemon
    // itself is what lets enforcement (and schedule starts) continue without a
    // client; the policy is deliberately short-lived.
    {
        let shared = Arc::clone(&daemon);
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(5));
                let Ok(daemon) = shared.lock() else {
                    return;
                };
                daemon.write_policy(Utc::now());
            }
        });
    }

    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let shared = Arc::clone(&daemon);
                let path = socket_path.clone();
                thread::spawn(move || serve_client(stream, shared, path));
            }
            Err(error) => eprintln!("stayproductive-daemon: accept failed: {error}"),
        }
    }
}

fn serve_client(stream: UnixStream, daemon: Arc<Mutex<Daemon>>, socket_path: PathBuf) {
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut writer = stream;
    {
        let Ok(daemon) = daemon.lock() else {
            return;
        };
        if writeln!(writer, "{}", daemon.greeting()).is_err() {
            return;
        }
        if writer.flush().is_err() {
            return;
        }
    }

    let reader = BufReader::new(reader_stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let (response, shutdown) = {
            let Ok(mut daemon) = daemon.lock() else {
                return;
            };
            let response = daemon.respond(&line);
            (response, daemon.shutdown)
        };
        if writeln!(writer, "{response}")
            .and_then(|_| writer.flush())
            .is_err()
        {
            return;
        }
        if shutdown {
            let _ = fs::remove_file(&socket_path);
            std::process::exit(0);
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

/// The extension writes this heartbeat while it is enabled. It is deliberately
/// short-lived so a crashed or disabled extension cannot keep claiming
/// enforcement.
fn extension_heartbeat(now: Timestamp) -> bool {
    let path = runtime_dir().join("stayproductive/gnome-extension.json");
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    if value.get("active").and_then(Value::as_bool) != Some(true)
        || value.get("policy_loaded").and_then(Value::as_bool) != Some(true)
    {
        return false;
    }
    let Some(checked_at) = value
        .get("checked_at")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Timestamp>().ok())
    else {
        return false;
    };
    let age = now.signed_duration_since(checked_at);
    age <= chrono::TimeDelta::seconds(5)
}

fn enforcement_policy(state: &State, now: Timestamp) -> Value {
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

    let mut apps = Vec::new();
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
        apps.push(json!({
            "app": app.as_str(),
            "reason": decision.reason.as_str(),
            "until": decision.until,
            "profile": profile,
        }));
    }

    json!({
        "version": 1,
        "generated_at": now,
        "valid_until": now + chrono::TimeDelta::seconds(30),
        "apps": apps,
    })
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
    fn new(path: PathBuf) -> Self {
        let state = load_state(&path);
        Self {
            state,
            path,
            shutdown: false,
        }
    }

    fn greeting(&self) -> String {
        json!({
            "event": "ready",
            "state_path": self.path.display().to_string(),
        })
        .to_string()
    }

    /// One newline-delimited request in, one out. Invalid JSON never kills the
    /// service; it returns an error envelope with a zero id.
    fn respond(&mut self, line: &str) -> String {
        let request: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => {
                return json!({
                    "id": 0,
                    "ok": false,
                    "error": format!("invalid JSON: {error}"),
                })
                .to_string();
            }
        };
        let id = request.get("id").and_then(Value::as_u64).unwrap_or(0);
        let command = request
            .get("cmd")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match self.handle(&command, &request) {
            Ok(data) => json!({"id": id, "ok": true, "data": data}),
            Err(error) => json!({"id": id, "ok": false, "error": error}),
        }
        .to_string()
    }

    fn handle(&mut self, command: &str, request: &Value) -> DResult<Value> {
        let now = Utc::now();
        let mut dirty = true;
        let result =
            match command {
                "snapshot" => {
                    dirty = false;
                    Ok(self.snapshot())
                }
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
                    dirty = false;
                    self.shutdown = true;
                    Ok(json!({"bye": true}))
                }
                other => return Err(format!("unknown command: {other}")),
            };

        if dirty && result.is_ok() {
            self.persist()?;
        }
        if result.is_ok() {
            self.write_policy(now);
        }
        result
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

    fn write_policy(&self, now: Timestamp) {
        let policy = enforcement_policy(&self.state, now);
        let Ok(encoded) = serde_json::to_vec_pretty(&policy) else {
            eprintln!("stayproductive: could not encode enforcement policy");
            return;
        };
        if let Err(error) = write_private(&self.policy_path(), &encoded) {
            eprintln!("stayproductive: could not write enforcement policy: {error}");
        }
    }

    fn snapshot(&self) -> Value {
        let now = Utc::now();
        let heartbeat = extension_heartbeat(now);
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
                "kind": match block.target { Target::App(_) => "app", Target::Site(_) => "site" },
                "until": block.until,
            })).collect::<Vec<_>>(),
            "history": history,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_the_python_client() {
        // `engine.py` derives the same socket suffix for a custom state file.
        assert_eq!(fnv1a_hex(b""), "cbf29ce484222325");
        assert_eq!(fnv1a_hex(b"abc"), "e71fa2190541574b");
    }

    #[test]
    fn socket_argument_takes_an_optional_explicit_path() {
        match parse_args(vec!["--socket".into(), "/tmp/chosen.sock".into()]).unwrap() {
            Mode::Socket(path) => assert_eq!(path, PathBuf::from("/tmp/chosen.sock")),
            Mode::Stdio => panic!("expected socket mode"),
        }
        match parse_args(vec!["--socket=/tmp/inline.sock".into()]).unwrap() {
            Mode::Socket(path) => assert_eq!(path, PathBuf::from("/tmp/inline.sock")),
            Mode::Stdio => panic!("expected socket mode"),
        }
        assert!(matches!(
            parse_args(vec!["--stdio".into()]).unwrap(),
            Mode::Stdio
        ));
        assert!(parse_args(vec!["--wat".into()]).is_err());
    }

    #[test]
    fn invalid_json_returns_an_error_envelope() {
        let path = env::temp_dir().join(format!("stayproductive-unit-{}.json", ulid::Ulid::new()));
        let mut daemon = Daemon::new(path);
        let response: Value = serde_json::from_str(&daemon.respond("{not json")).unwrap();
        assert_eq!(response["id"], 0);
        assert_eq!(response["ok"], false);
    }
}
