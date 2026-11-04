//! End-to-end checks for the local state service protocol.
//!
//! These drive the real binary over stdio, with a temporary state file. They do
//! not need a desktop or a display, so they run in CI alongside the core suite.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

fn temp_state() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("stayproductive-protocol-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).expect("temp state directory");
    let state = dir.join("state.json");
    (dir, state)
}

struct Service {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Service {
    fn start(state: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_stayproductive-daemon"))
            .env("STAYPRODUCTIVE_STATE", state)
            .env("STAYPRODUCTIVE_X11_ADAPTER", "0")
            .env("TZ", "UTC")
            .env(
                "XDG_RUNTIME_DIR",
                state.parent().unwrap_or_else(|| Path::new(".")),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start state service");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let mut service = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        let greeting: Value = serde_json::from_str(&service.read_line()).expect("greeting json");
        assert_eq!(greeting["event"], "ready");
        assert_eq!(
            PathBuf::from(greeting["state_path"].as_str().expect("state path")),
            state
        );
        service
    }

    fn read_line(&mut self) -> String {
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("read response");
        assert!(!line.is_empty(), "service closed stdout");
        line
    }

    fn request(&mut self, command: &str, payload: Value) -> Value {
        let mut body = payload;
        body["id"] = json!(self.next_id);
        body["cmd"] = json!(command);
        self.next_id += 1;
        writeln!(self.stdin, "{body}").expect("write request");
        self.stdin.flush().expect("flush request");
        serde_json::from_str(&self.read_line()).expect("response json")
    }

    fn ok(&mut self, command: &str, payload: Value) -> Value {
        let response = self.request(command, payload);
        assert_eq!(response["ok"], true, "{command} failed: {response}");
        response["data"].clone()
    }

    fn shutdown(mut self) {
        self.ok("shutdown", json!({}));
        let status = self.child.wait().expect("wait for service");
        assert!(status.success(), "service exited with {status}");
    }
}

#[test]
fn strict_commitment_locks_rules_and_survives_restart() {
    let (dir, path) = temp_state();
    let _ = std::fs::remove_file(&path);

    let mut service = Service::start(&path);
    let snapshot = service.ok(
        "create_profile",
        json!({
            "profile": {
                "name": "Focus",
                "activation": "manual",
                "apps": [{"key": "firefox", "action": "block"}]
            }
        }),
    );
    let profile_id = snapshot["profiles"][0]["id"]
        .as_str()
        .expect("profile id")
        .to_owned();
    service.ok("activate_profile", json!({"profile_id": profile_id}));
    service.ok(
        "start_strict",
        json!({"minutes": 30, "cooldown_minutes": 10}),
    );

    for (command, payload) in [
        (
            "update_profile",
            json!({
                "profile_id": profile_id,
                "profile": {"name": "Focus 2", "activation": "manual"}
            }),
        ),
        ("delete_profile", json!({"profile_id": profile_id})),
        ("deactivate_profile", json!({})),
        ("activate_profile", json!({"profile_id": profile_id})),
        (
            "pause_profile",
            json!({"profile_id": profile_id, "minutes": 5}),
        ),
    ] {
        let response = service.request(command, payload);
        assert_eq!(response["ok"], false, "{command} should be locked");
        assert!(
            response["error"]
                .as_str()
                .is_some_and(|error| error.contains("locked") || error.contains("commitment")),
            "unexpected error for {command}: {response}"
        );
    }

    let commitment_id = service.ok("snapshot", json!({}))["commitment"]["id"]
        .as_str()
        .expect("commitment id")
        .to_owned();
    service.ok(
        "request_commitment_end",
        json!({"commitment_id": commitment_id}),
    );
    let early = service.request(
        "finish_commitment_end",
        json!({"commitment_id": commitment_id}),
    );
    assert_eq!(early["ok"], false, "cool-down must not be skippable");
    service.shutdown();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("state file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "state file must be private");
    }

    let mut restarted = Service::start(&path);
    let snapshot = restarted.ok("snapshot", json!({}));
    assert_eq!(snapshot["profiles"][0]["name"], "Focus");
    assert_eq!(snapshot["commitment"]["state"], "cooling_down");
    assert_eq!(
        snapshot["commitment"]["restrictions"],
        json!(["block_edits", "block_pause"])
    );
    restarted.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

fn write_heartbeat(
    heartbeat: &Path,
    focus: Option<Vec<&str>>,
    focus_since: Option<chrono::DateTime<chrono::Utc>>,
    blocked: bool,
) {
    write_adapter_heartbeat(heartbeat, "gnome-extension", focus, focus_since, blocked);
}

fn write_adapter_heartbeat(
    heartbeat: &Path,
    adapter: &str,
    focus: Option<Vec<&str>>,
    focus_since: Option<chrono::DateTime<chrono::Utc>>,
    blocked: bool,
) {
    let now = chrono::Utc::now();
    let body = json!({
        "adapter": adapter,
        "active": true,
        "policy_loaded": true,
        "blocked": blocked,
        "focus": focus,
        "focus_since": focus_since.map(|at| at.to_rfc3339()),
        "checked_at": now.to_rfc3339(),
    });
    std::fs::write(heartbeat, body.to_string()).expect("write heartbeat");
}

fn policy_apps(policy_path: &Path) -> Vec<Value> {
    let policy: Value = serde_json::from_slice(&std::fs::read(policy_path).expect("policy file"))
        .expect("policy json");
    policy["apps"].as_array().expect("apps array").clone()
}

fn usage_seconds(snapshot: &Value, app: &str) -> u64 {
    snapshot["usage_today"]
        .as_array()
        .expect("usage_today array")
        .iter()
        .find(|entry| entry["app"] == app)
        .map(|entry| entry["seconds"].as_u64().expect("seconds"))
        .unwrap_or(0)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Read a stored zip written by the export command. This deliberately parses
/// the central directory instead of trusting the writer, so a wrong offset or
/// checksum fails the test.
fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let eocd = (0..bytes.len() - 4)
        .rev()
        .find(|&at| &bytes[at..at + 4] == b"PK\x05\x06")
        .expect("end of central directory");
    let count = u16::from_le_bytes(bytes[eocd + 10..eocd + 12].try_into().expect("count")) as usize;
    let mut at =
        u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().expect("offset")) as usize;
    let mut files = BTreeMap::new();
    for _ in 0..count {
        assert_eq!(
            &bytes[at..at + 4],
            b"PK\x01\x02",
            "central directory header"
        );
        let method = u16::from_le_bytes(bytes[at + 10..at + 12].try_into().expect("method"));
        assert_eq!(method, 0, "export entries are stored, not compressed");
        let crc = u32::from_le_bytes(bytes[at + 16..at + 20].try_into().expect("crc"));
        let size = u32::from_le_bytes(bytes[at + 20..at + 24].try_into().expect("size")) as usize;
        let name_len =
            u16::from_le_bytes(bytes[at + 28..at + 30].try_into().expect("name length")) as usize;
        let extra_len =
            u16::from_le_bytes(bytes[at + 30..at + 32].try_into().expect("extra length")) as usize;
        let comment_len =
            u16::from_le_bytes(bytes[at + 32..at + 34].try_into().expect("comment length"))
                as usize;
        let local =
            u32::from_le_bytes(bytes[at + 42..at + 46].try_into().expect("local offset")) as usize;
        let name = String::from_utf8(bytes[at + 46..at + 46 + name_len].to_vec())
            .expect("utf-8 entry name");
        assert_eq!(
            &bytes[local..local + 4],
            b"PK\x03\x04",
            "local header for {name}"
        );
        let local_name = u16::from_le_bytes(
            bytes[local + 26..local + 28]
                .try_into()
                .expect("name length"),
        ) as usize;
        let local_extra = u16::from_le_bytes(
            bytes[local + 28..local + 30]
                .try_into()
                .expect("extra length"),
        ) as usize;
        let start = local + 30 + local_name + local_extra;
        let data = bytes[start..start + size].to_vec();
        assert_eq!(crc32(&data), crc, "crc for {name}");
        files.insert(name, data);
        at += 46 + name_len + extra_len + comment_len;
    }
    files
}

#[test]
fn export_and_delete_usage_data() {
    let (dir, path) = temp_state();
    let runtime = dir.join("stayproductive");
    std::fs::create_dir_all(&runtime).expect("runtime directory");
    let heartbeat = runtime.join("gnome-extension.json");
    let export_path = dir.join("export.zip");

    let mut service = Service::start(&path);
    let created = service.ok(
        "create_profile",
        json!({
            "profile": {
                "name": "Focus",
                "activation": "manual",
                "apps": [{"key": "firefox", "action": "daily_limit", "minutes": 60}]
            }
        }),
    );
    let profile_id = created["profiles"][0]["id"]
        .as_str()
        .expect("profile id")
        .to_owned();
    service.ok("activate_profile", json!({"profile_id": profile_id}));

    write_heartbeat(
        &heartbeat,
        Some(vec!["firefox"]),
        Some(chrono::Utc::now() - chrono::Duration::seconds(90)),
        false,
    );
    let snapshot = service.ok("snapshot", json!({}));
    assert!(usage_seconds(&snapshot, "firefox") >= 90);

    let exported = service.ok("export_data", json!({"path": export_path}));
    assert_eq!(exported["version"], 1);
    assert_eq!(exported["format"], "stayproductive.export");
    assert_eq!(exported["usage_included"], true);
    let bytes = std::fs::read(&export_path).expect("export archive");
    let files = unzip(&bytes);

    let manifest: Value = serde_json::from_slice(&files["manifest.json"]).expect("manifest json");
    assert_eq!(manifest["format"], "stayproductive.export");
    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["usage_included"], true);

    let profiles: Value = serde_json::from_slice(&files["profiles.json"]).expect("profiles json");
    assert_eq!(profiles["profiles"][0]["name"], "Focus");

    let settings: Value = serde_json::from_slice(&files["settings.json"]).expect("settings json");
    assert_eq!(settings["start_of_day"], "00:00");

    let usage_name = files
        .keys()
        .find(|name| name.starts_with("usage/"))
        .expect("monthly usage file");
    let usage: Value = serde_json::from_slice(&files[usage_name]).expect("usage json");
    assert_eq!(usage["intervals"][0]["app"], "firefox");

    // Deleting history needs an explicit decision.
    let refused = service.request("delete_history", json!({}));
    assert_eq!(refused["ok"], false, "unconfirmed delete: {refused}");
    assert!(
        refused["error"]
            .as_str()
            .expect("error text")
            .contains("confirm")
    );

    std::fs::remove_file(&heartbeat).ok();
    let deleted = service.ok("delete_history", json!({"confirm": true}));
    assert!(
        deleted["removed"]["intervals"].as_u64().expect("count") >= 1,
        "delete reports what it removed: {deleted}"
    );
    assert_eq!(deleted["snapshot"]["usage_today"], json!([]));
    assert_eq!(deleted["snapshot"]["history"], json!([]));
    assert_eq!(
        deleted["snapshot"]["profiles"]
            .as_array()
            .expect("profiles")
            .len(),
        1,
        "profiles are configuration, not history"
    );

    // The deletion is persisted, not only in memory.
    service.shutdown();
    let mut restarted = Service::start(&path);
    let snapshot = restarted.ok("snapshot", json!({}));
    assert_eq!(snapshot["usage_today"], json!([]));
    assert_eq!(snapshot["profiles"].as_array().expect("profiles").len(), 1);
    restarted.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn usage_tracking_enforces_daily_limit_and_records_history() {
    let (dir, path) = temp_state();
    let runtime = dir.join("stayproductive");
    std::fs::create_dir_all(&runtime).expect("runtime directory");
    let heartbeat = runtime.join("gnome-extension.json");
    let policy_path = dir.join("enforcement.json");

    let mut service = Service::start(&path);
    let created = service.ok(
        "create_profile",
        json!({
            "profile": {
                "name": "Focus",
                "activation": "manual",
                "apps": [{"key": "firefox", "action": "daily_limit", "minutes": 1}]
            }
        }),
    );
    let profile_id = created["profiles"][0]["id"]
        .as_str()
        .expect("profile id")
        .to_owned();
    service.ok("activate_profile", json!({"profile_id": profile_id}));
    assert!(
        policy_apps(&policy_path).is_empty(),
        "under the limit the app is not blocked"
    );

    // 90 seconds of reported focus, over the 1-minute daily limit.
    write_heartbeat(
        &heartbeat,
        Some(vec!["org.mozilla.firefox"]),
        Some(chrono::Utc::now() - chrono::Duration::seconds(90)),
        false,
    );
    let snapshot = service.ok("snapshot", json!({}));
    let apps = policy_apps(&policy_path);
    assert_eq!(apps.len(), 1, "the daily limit must now block: {apps:?}");
    assert_eq!(apps[0]["app"], "firefox");
    assert_eq!(apps[0]["reason"], "R-LIMIT-DAILY");
    assert!(
        apps[0]["until"].as_str().is_some(),
        "limit blocks say when the day ends"
    );
    assert_eq!(apps[0]["profile"], "Focus");
    assert!(
        usage_seconds(&snapshot, "firefox") >= 90,
        "focus time is attributed to the app: {}",
        snapshot["usage_today"]
    );
    assert_eq!(
        snapshot["usage_today"]
            .as_array()
            .expect("usage_today")
            .len(),
        1,
        "no other app is attributed"
    );
    // The block is recorded when the policy is written, after the response
    // above was rendered; the next poll sees it in history.
    let snapshot = service.ok("snapshot", json!({}));
    assert_eq!(
        snapshot["history"][0]["target"], "firefox",
        "the enforced block is recorded in history"
    );
    assert_eq!(snapshot["history"][0]["reason"], "R-LIMIT-DAILY");

    service.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn blocked_overlay_suspends_usage_accrual() {
    let (dir, path) = temp_state();
    let runtime = dir.join("stayproductive");
    std::fs::create_dir_all(&runtime).expect("runtime directory");
    let heartbeat = runtime.join("gnome-extension.json");

    let mut service = Service::start(&path);
    service.ok(
        "create_profile",
        json!({
            "profile": {
                "name": "Focus",
                "activation": "manual",
                "apps": [{"key": "firefox", "action": "daily_limit", "minutes": 60}]
            }
        }),
    );
    let profile_id = service.ok("snapshot", json!({}))["profiles"][0]["id"]
        .as_str()
        .expect("profile id")
        .to_owned();
    service.ok("activate_profile", json!({"profile_id": profile_id}));

    let since = chrono::Utc::now() - chrono::Duration::seconds(90);
    write_heartbeat(&heartbeat, Some(vec!["firefox"]), Some(since), false);
    let before = usage_seconds(&service.ok("snapshot", json!({})), "firefox");
    assert!(before >= 90, "usage is attributed while focused: {before}");

    // The overlay is up; time behind the block screen must not count.
    write_heartbeat(&heartbeat, Some(vec!["firefox"]), Some(since), true);
    service.ok("snapshot", json!({}));

    // Same focus window resumes after the overlay; the blocked gap must not
    // be re-attributed even though focus_since still predates it.
    write_heartbeat(&heartbeat, Some(vec!["firefox"]), Some(since), false);
    let after = usage_seconds(&service.ok("snapshot", json!({})), "firefox");
    assert!(
        after - before < 30,
        "blocked time must not accrue: before {before}, after {after}"
    );

    // A stale heartbeat means no observer: attribution stops again.
    std::fs::write(
        &heartbeat,
        json!({
            "active": true,
            "policy_loaded": true,
            "blocked": false,
            "focus": ["firefox"],
            "focus_since": since.to_rfc3339(),
            "checked_at": (chrono::Utc::now() - chrono::Duration::seconds(30)).to_rfc3339(),
        })
        .to_string(),
    )
    .expect("write stale heartbeat");
    let stopped = usage_seconds(&service.ok("snapshot", json!({})), "firefox");
    assert!(
        stopped - after < 30,
        "attribution must stop without a fresh observer: after {after}, stopped {stopped}"
    );

    service.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn usage_counters_survive_restart() {
    let (dir, path) = temp_state();
    let runtime = dir.join("stayproductive");
    std::fs::create_dir_all(&runtime).expect("runtime directory");
    let heartbeat = runtime.join("gnome-extension.json");
    let policy_path = dir.join("enforcement.json");

    let mut service = Service::start(&path);
    let created = service.ok(
        "create_profile",
        json!({
            "profile": {
                "name": "Focus",
                "activation": "manual",
                "apps": [{"key": "steam", "action": "daily_limit", "minutes": 1}]
            }
        }),
    );
    let profile_id = created["profiles"][0]["id"]
        .as_str()
        .expect("profile id")
        .to_owned();
    service.ok("activate_profile", json!({"profile_id": profile_id}));

    write_heartbeat(
        &heartbeat,
        Some(vec!["steam"]),
        Some(chrono::Utc::now() - chrono::Duration::seconds(90)),
        false,
    );
    let snapshot = service.ok("snapshot", json!({}));
    assert!(
        usage_seconds(&snapshot, "steam") >= 90,
        "usage recorded before restart"
    );
    assert!(
        policy_apps(&policy_path)
            .iter()
            .any(|app| app["app"] == "steam"),
        "blocked before restart"
    );
    service.shutdown();

    // Restart with no focus reported: the recorded intervals must keep the
    // limit exhausted (A-LIM-05).
    write_heartbeat(&heartbeat, None, None, false);
    let mut restarted = Service::start(&path);
    let snapshot = restarted.ok("snapshot", json!({}));
    assert!(
        usage_seconds(&snapshot, "steam") >= 90,
        "usage survives the restart"
    );
    assert!(
        policy_apps(&policy_path)
            .iter()
            .any(|app| app["app"] == "steam" && app["reason"] == "R-LIMIT-DAILY"),
        "the exhausted limit still blocks after restart"
    );
    restarted.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn x11_focus_reports_feed_usage_limits() {
    let (dir, path) = temp_state();
    let runtime = dir.join("stayproductive");
    std::fs::create_dir_all(&runtime).expect("runtime directory");
    // No GNOME extension here: the X11 adapter is the only observer.
    let heartbeat = runtime.join("x11-adapter.json");
    let policy_path = dir.join("enforcement.json");

    let mut service = Service::start(&path);
    let created = service.ok(
        "create_profile",
        json!({
            "profile": {
                "name": "Focus",
                "activation": "manual",
                "apps": [{"key": "firefox", "action": "daily_limit", "minutes": 1}]
            }
        }),
    );
    let profile_id = created["profiles"][0]["id"]
        .as_str()
        .expect("profile id")
        .to_owned();
    service.ok("activate_profile", json!({"profile_id": profile_id}));

    // The adapter reports WM_CLASS and desktop-id aliases; the daemon maps
    // them to the profile's own key.
    write_adapter_heartbeat(
        &heartbeat,
        "x11",
        Some(vec!["navigator", "firefox", "org.mozilla.firefox"]),
        Some(chrono::Utc::now() - chrono::Duration::seconds(90)),
        false,
    );
    let snapshot = service.ok("snapshot", json!({}));
    assert!(
        usage_seconds(&snapshot, "firefox") >= 90,
        "X11 focus time is attributed: {}",
        usage_seconds(&snapshot, "firefox")
    );
    let apps = policy_apps(&policy_path);
    assert_eq!(apps.len(), 1, "the daily limit blocks on X11 too: {apps:?}");
    assert_eq!(apps[0]["app"], "firefox");
    assert_eq!(apps[0]["reason"], "R-LIMIT-DAILY");

    // Behind the X11 overlay the adapter reports no focus, so the interval
    // closes instead of accruing behind the explanation.
    write_adapter_heartbeat(&heartbeat, "x11", None, None, true);
    let before = usage_seconds(&service.ok("snapshot", json!({})), "firefox");
    std::thread::sleep(std::time::Duration::from_millis(1200));
    let after = usage_seconds(&service.ok("snapshot", json!({})), "firefox");
    assert_eq!(after, before, "no usage accrues behind the block overlay");

    service.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn enforcement_snapshot_requires_a_fresh_adapter_heartbeat() {
    let (dir, path) = temp_state();
    let mut service = Service::start(&path);
    let runtime = dir.join("stayproductive");
    std::fs::create_dir_all(&runtime).expect("runtime directory");
    let heartbeat = runtime.join("gnome-extension.json");

    let snapshot = service.ok("snapshot", json!({}));
    assert_eq!(snapshot["enforcement"]["state"], "unavailable");
    assert_eq!(snapshot["enforcement"]["adapter"], "gnome-extension");
    assert_eq!(
        snapshot["enforcement"]["policy_path"],
        dir.join("enforcement.json").display().to_string()
    );

    std::fs::write(
        &heartbeat,
        json!({
            "adapter": "gnome-extension",
            "active": true,
            "policy_loaded": true,
            "checked_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string(),
    )
    .expect("write heartbeat");
    let snapshot = service.ok("snapshot", json!({}));
    assert_eq!(snapshot["enforcement"]["state"], "active");

    std::fs::write(
        &heartbeat,
        json!({
            "adapter": "gnome-extension",
            "active": true,
            "policy_loaded": true,
            "checked_at": (chrono::Utc::now() - chrono::Duration::seconds(10)).to_rfc3339(),
        })
        .to_string(),
    )
    .expect("write stale heartbeat");
    let snapshot = service.ok("snapshot", json!({}));
    assert_eq!(snapshot["enforcement"]["state"], "unavailable");

    std::fs::write(
        &heartbeat,
        json!({
            "adapter": "gnome-extension",
            "active": true,
            "policy_loaded": false,
            "checked_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string(),
    )
    .expect("write heartbeat without policy");
    let snapshot = service.ok("snapshot", json!({}));
    assert_eq!(snapshot["enforcement"]["state"], "unavailable");

    std::fs::write(
        &heartbeat,
        json!({
            "adapter": "gnome-extension",
            "active": false,
            "policy_loaded": true,
            "checked_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string(),
    )
    .expect("write inactive heartbeat");
    let snapshot = service.ok("snapshot", json!({}));
    assert_eq!(snapshot["enforcement"]["state"], "unavailable");

    // An X11 heartbeat with the expected adapter id wins over GNOME, and a
    // file that declares a different adapter is ignored.
    std::fs::write(
        runtime.join("x11-adapter.json"),
        json!({
            "adapter": "not-x11",
            "active": true,
            "policy_loaded": true,
            "checked_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string(),
    )
    .expect("write foreign x11 heartbeat");
    let snapshot = service.ok("snapshot", json!({}));
    assert_eq!(snapshot["enforcement"]["state"], "unavailable");

    std::fs::write(
        runtime.join("x11-adapter.json"),
        json!({
            "adapter": "x11",
            "active": true,
            "policy_loaded": true,
            "checked_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string(),
    )
    .expect("write x11 heartbeat");
    let snapshot = service.ok("snapshot", json!({}));
    assert_eq!(snapshot["enforcement"]["state"], "active");
    assert_eq!(snapshot["enforcement"]["adapter"], "x11");
    let _ = std::fs::remove_file(runtime.join("x11-adapter.json"));

    service.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn enforcement_policy_tracks_active_app_rules() {
    let (dir, path) = temp_state();
    let policy_path = dir.join("enforcement.json");
    let mut service = Service::start(&path);

    let created = service.ok(
        "create_profile",
        json!({
            "profile": {
                "name": "Focus",
                "activation": "manual",
                "apps": [
                    {"key": "firefox", "action": "block"},
                    {"key": "steam", "action": "daily_limit", "minutes": 30}
                ]
            }
        }),
    );
    let profile_id = created["profiles"][0]["id"]
        .as_str()
        .expect("profile id")
        .to_owned();

    let policy: Value = serde_json::from_slice(&std::fs::read(&policy_path).expect("policy file"))
        .expect("policy json");
    assert_eq!(policy["version"], 1);
    assert_eq!(policy["apps"], json!([]), "inactive profile must not block");

    service.ok("activate_profile", json!({"profile_id": profile_id}));
    let snapshot = service.ok("snapshot", json!({}));
    let policy: Value = serde_json::from_slice(&std::fs::read(&policy_path).expect("policy file"))
        .expect("policy json");
    assert_eq!(policy["apps"].as_array().expect("apps").len(), 1);
    assert_eq!(policy["apps"][0]["app"], "firefox");
    assert_eq!(policy["apps"][0]["reason"], "R-PROFILE");
    assert_eq!(policy["apps"][0]["profile"], "Focus");
    assert!(policy["valid_until"].as_str().is_some());
    assert_eq!(snapshot["enforcement"]["adapter"], "gnome-extension");
    assert_eq!(
        snapshot["enforcement"]["policy_path"],
        policy_path.display().to_string()
    );

    service.ok("deactivate_profile", json!({}));
    let policy: Value = serde_json::from_slice(&std::fs::read(&policy_path).expect("policy file"))
        .expect("policy json");
    assert_eq!(policy["apps"], json!([]));

    service.ok(
        "quick_block",
        json!({"kind": "app", "target": "steam", "minutes": 15}),
    );
    let policy: Value = serde_json::from_slice(&std::fs::read(&policy_path).expect("policy file"))
        .expect("policy json");
    assert_eq!(policy["apps"].as_array().expect("apps").len(), 1);
    assert_eq!(policy["apps"][0]["app"], "steam");
    assert_eq!(policy["apps"][0]["reason"], "R-QUICK");
    assert!(policy["apps"][0]["until"].as_str().is_some());
    assert!(policy["apps"][0]["profile"].is_null());

    service.ok(
        "clear_quick_block",
        json!({"kind": "app", "target": "steam"}),
    );
    let policy: Value = serde_json::from_slice(&std::fs::read(&policy_path).expect("policy file"))
        .expect("policy json");
    assert_eq!(policy["apps"], json!([]));

    service.shutdown();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&policy_path)
            .expect("policy file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "policy file must be private");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- socket mode

use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

struct SocketClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl SocketClient {
    fn connect(path: &Path) -> Self {
        let stream = UnixStream::connect(path).expect("connect to socket");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("read timeout");
        let reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut client = Self {
            stream,
            reader,
            next_id: 1,
        };
        let greeting: Value = serde_json::from_str(&client.read_line()).expect("greeting json");
        assert_eq!(greeting["event"], "ready");
        client
    }

    fn read_line(&mut self) -> String {
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("read response");
        assert!(!line.is_empty(), "service closed the socket");
        line
    }

    fn request(&mut self, command: &str, payload: Value) -> Value {
        let mut body = payload;
        body["id"] = json!(self.next_id);
        body["cmd"] = json!(command);
        self.next_id += 1;
        writeln!(self.stream, "{body}").expect("write request");
        self.stream.flush().expect("flush request");
        serde_json::from_str(&self.read_line()).expect("response json")
    }

    fn ok(&mut self, command: &str, payload: Value) -> Value {
        let response = self.request(command, payload);
        assert_eq!(response["ok"], true, "{command} failed: {response}");
        response["data"].clone()
    }
}

struct SocketDaemon {
    child: Option<Child>,
    socket: PathBuf,
}

impl SocketDaemon {
    fn start(state: &Path, socket: &Path) -> Self {
        Self::start_with_adapter(state, socket, None)
    }

    fn start_with_adapter(state: &Path, socket: &Path, adapter: Option<&Path>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_stayproductive-daemon"));
        command
            .arg("--socket")
            .arg(socket)
            .env("STAYPRODUCTIVE_STATE", state)
            .env(
                "STAYPRODUCTIVE_X11_ADAPTER",
                adapter
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "0".to_string()),
            )
            .env(
                "XDG_RUNTIME_DIR",
                state.parent().unwrap_or_else(|| Path::new(".")),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn().expect("start socket service");
        // Own the child immediately so a failed readiness check or a panic
        // cannot leave a zombie behind.
        let service = Self {
            child: Some(child),
            socket: socket.to_path_buf(),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if UnixStream::connect(socket).is_ok() {
                return service;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "socket service did not become ready at {}",
            socket.display()
        );
    }

    fn shutdown(mut self) {
        let mut client = SocketClient::connect(&self.socket);
        client.ok("shutdown", json!({}));
        let mut child = self.child.take().expect("service child");
        let status = child.wait().expect("wait for service");
        assert!(status.success(), "service exited with {status}");
        assert!(!self.socket.exists(), "socket file must be removed on exit");
    }
}

impl Drop for SocketDaemon {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn socket_service_is_single_instance_and_private() {
    let (dir, state) = temp_state();
    let socket = dir.join("stayproductive").join("daemon.sock");
    let daemon = SocketDaemon::start(&state, &socket);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&socket)
            .expect("socket file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "socket must be same-user only");
        let dir_mode = std::fs::metadata(socket.parent().expect("socket parent"))
            .expect("socket directory")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, "socket directory must be private");
    }

    let mut second = Command::new(env!("CARGO_BIN_EXE_stayproductive-daemon"))
        .arg("--socket")
        .arg(&socket)
        .env("STAYPRODUCTIVE_STATE", &state)
        .env("STAYPRODUCTIVE_X11_ADAPTER", "0")
        .env(
            "XDG_RUNTIME_DIR",
            state.parent().unwrap_or_else(|| Path::new(".")),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start second instance");
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = second.try_wait().expect("poll second instance") {
            break Some(status);
        }
        if Instant::now() > deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.is_some_and(|status| status.success()),
        "a second instance must exit successfully without taking over"
    );

    let mut client = SocketClient::connect(&socket);
    let snapshot = client.ok("snapshot", json!({}));
    assert_eq!(
        PathBuf::from(snapshot["state_path"].as_str().expect("state path")),
        state
    );
    daemon.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn socket_service_refreshes_the_policy_without_a_client() {
    let (dir, state) = temp_state();
    let socket = dir.join("stayproductive").join("daemon.sock");
    let policy_path = dir.join("enforcement.json");
    let daemon = SocketDaemon::start(&state, &socket);

    let mut client = SocketClient::connect(&socket);
    let created = client.ok(
        "create_profile",
        json!({
            "profile": {
                "name": "Focus",
                "activation": "manual",
                "apps": [{"key": "firefox", "action": "block"}]
            }
        }),
    );
    let profile_id = created["profiles"][0]["id"]
        .as_str()
        .expect("profile id")
        .to_owned();
    client.ok("activate_profile", json!({"profile_id": profile_id}));
    let before = read_policy(&policy_path)["generated_at"]
        .as_str()
        .expect("generated_at")
        .to_owned();
    drop(client);

    // No client for six seconds; the daemon itself must keep the short-lived
    // policy fresh, otherwise enforcement silently lapses when the window closes.
    std::thread::sleep(Duration::from_secs(6));
    let after = read_policy(&policy_path)["generated_at"]
        .as_str()
        .expect("generated_at")
        .to_owned();
    assert!(
        after > before,
        "policy timestamp did not advance without a client: {before} -> {after}"
    );

    let mut client = SocketClient::connect(&socket);
    let snapshot = client.ok("snapshot", json!({}));
    assert_eq!(snapshot["profiles"][0]["name"], "Focus");
    drop(client);
    daemon.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

fn read_policy(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("policy file")).expect("policy json")
}

#[test]
fn socket_service_tracks_usage_without_a_client() {
    let (dir, state) = temp_state();
    let runtime = dir.join("stayproductive");
    std::fs::create_dir_all(&runtime).expect("runtime directory");
    let socket = runtime.join("daemon.sock");
    let heartbeat = runtime.join("x11-adapter.json");
    let daemon = SocketDaemon::start(&state, &socket);

    let mut client = SocketClient::connect(&socket);
    let created = client.ok(
        "create_profile",
        json!({
            "profile": {
                "name": "Focus",
                "activation": "manual",
                "apps": [{"key": "firefox", "action": "daily_limit", "minutes": 60}]
            }
        }),
    );
    let profile_id = created["profiles"][0]["id"]
        .as_str()
        .expect("profile id")
        .to_owned();
    client.ok("activate_profile", json!({"profile_id": profile_id}));
    drop(client);

    write_adapter_heartbeat(
        &heartbeat,
        "x11",
        Some(vec!["firefox"]),
        Some(chrono::Utc::now() - chrono::Duration::seconds(90)),
        false,
    );

    // No client for six seconds: the daemon itself must attribute the reported
    // focus, not only refresh the policy, so limits keep working when the GTK
    // window is closed.
    std::thread::sleep(Duration::from_secs(6));

    let mut client = SocketClient::connect(&socket);
    let snapshot = client.ok("snapshot", json!({}));
    assert!(
        usage_seconds(&snapshot, "firefox") >= 90,
        "usage is attributed without a client: {}",
        usage_seconds(&snapshot, "firefox")
    );
    drop(client);
    daemon.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn socket_service_supervises_the_x11_adapter() {
    let (dir, state) = temp_state();
    let runtime = dir.join("stayproductive");
    std::fs::create_dir_all(&runtime).expect("runtime directory");
    let socket = runtime.join("daemon.sock");
    let heartbeat = runtime.join("x11-adapter.json");
    let pid_file = dir.join("adapter.pid");
    let adapter = dir.join("fake-adapter.sh");
    let script = format!(
        "#!/bin/sh\n\
         heartbeat=\"\"\n\
         while [ $# -gt 0 ]; do\n\
         \x20 case \"$1\" in\n\
         \x20   --heartbeat) heartbeat=\"$2\"; shift 2 ;;\n\
         \x20   *) shift ;;\n\
         \x20 esac\n\
         done\n\
         echo $$ > \"{}\"\n\
         while :; do\n\
         \x20 mkdir -p \"$(dirname \"$heartbeat\")\"\n\
         \x20 printf '{{\"adapter\":\"x11\",\"active\":true,\"policy_loaded\":true,\"checked_at\":\"%s\"}}\\n' \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\" > \"$heartbeat\"\n\
         \x20 sleep 0.2\n\
         done\n",
        pid_file.display()
    );
    std::fs::write(&adapter, script).expect("write fake adapter");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o700))
            .expect("make adapter executable");
    }

    let daemon = SocketDaemon::start_with_adapter(&state, &socket, Some(&adapter));
    let deadline = Instant::now() + Duration::from_secs(10);
    let first_pid = loop {
        if let Ok(text) = std::fs::read_to_string(&pid_file)
            && let Ok(pid) = text.trim().parse::<u32>()
            && heartbeat.exists()
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "adapter did not start");
        std::thread::sleep(Duration::from_millis(50));
    };

    let mut client = SocketClient::connect(&socket);
    let snapshot = client.ok("snapshot", json!({}));
    assert_eq!(snapshot["enforcement"]["state"], "active");
    assert_eq!(snapshot["enforcement"]["adapter"], "x11");

    // Kill the adapter: the daemon must notice, drop the stale heartbeat, and
    // start a replacement without any UI or client involvement.
    let status = Command::new("kill")
        .args(["-9", &first_pid.to_string()])
        .status()
        .expect("kill adapter");
    assert!(status.success(), "kill failed");
    let deadline = Instant::now() + Duration::from_secs(10);
    let second_pid = loop {
        if let Ok(text) = std::fs::read_to_string(&pid_file)
            && let Ok(pid) = text.trim().parse::<u32>()
            && pid != first_pid
            && heartbeat.exists()
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "adapter was not restarted");
        std::thread::sleep(Duration::from_millis(50));
    };
    let snapshot = client.ok("snapshot", json!({}));
    assert_eq!(snapshot["enforcement"]["state"], "active");
    assert_eq!(snapshot["enforcement"]["adapter"], "x11");
    drop(client);
    daemon.shutdown();

    // Shutdown stops the adapter; no orphan keeps enforcing.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Path::new(&format!("/proc/{second_pid}")).exists() {
        assert!(
            Instant::now() < deadline,
            "adapter {second_pid} survived shutdown"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
