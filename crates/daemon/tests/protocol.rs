//! End-to-end checks for the local state service protocol.
//!
//! These drive the real binary over stdio, with a temporary state file. They do
//! not need a desktop or a display, so they run in CI alongside the core suite.

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

#[test]
fn enforcement_snapshot_requires_fresh_extension_heartbeat() {
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
            "active": false,
            "policy_loaded": true,
            "checked_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string(),
    )
    .expect("write inactive heartbeat");
    let snapshot = service.ok("snapshot", json!({}));
    assert_eq!(snapshot["enforcement"]["state"], "unavailable");

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
