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
            .env("STAYPRODUCTIVE_X11_ADAPTER", "0")
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
