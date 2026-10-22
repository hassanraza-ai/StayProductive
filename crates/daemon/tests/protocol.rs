//! End-to-end checks for the local state service protocol.
//!
//! These drive the real binary over stdio, with a temporary state file. They do
//! not need a desktop or a display, so they run in CI alongside the core suite.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

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
    let path = std::env::temp_dir().join(format!(
        "stayproductive-protocol-{}.json",
        ulid::Ulid::new()
    ));
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
    let _ = std::fs::remove_file(&path);
}
