//! Opt-in installed-CLI contracts. `drift_canary` uses an impossible model;
//! `live_flow` spends a few tokens using the owner's Smith config and logins.
//!
//! Build the example first: `cargo build -p smith-cli --example cli_stand_in`.
//! Run either suite with `cargo test -p smith-cli --test live_cli_bridge
//! drift_canary -- --ignored --nocapture --test-threads=1` (or `live_flow`).
//! Optional overrides: LIVE_CLAUDE_BIN, LIVE_CODEX_BIN, LIVE_CLAUDE_MODEL
//! (haiku), LIVE_CODEX_MODEL (absent means Codex chooses). No SMITH_* switch
//! is introduced: Smith rejects unknown variables in that namespace.
//! LIVE_CHILD_MODEL=<Smith model id> enables a provider child; LIVE_ADVISOR=1
//! enables the owner's configured advisor. Both spend additional tokens.

#![cfg(unix)]

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

#[path = "cli_contract/bridge_cleanup.rs"]
mod bridge_cleanup;
use bridge_cleanup::BridgeCleanup;

#[path = "live_cli_bridge/canary.rs"]
mod canary;
#[path = "live_cli_bridge/live_flow.rs"]
mod live_flow;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../smith-runtime/tests/fixtures/cli_bridge"
);
const INVALID_MODEL: &str = "no-such-model-xyz";
const LOCAL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Claude,
    Codex,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    fn program(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn kind(self) -> &'static str {
        match self {
            Self::Claude => "claude-code",
            Self::Codex => "codex",
        }
    }

    fn bin_env(self) -> &'static str {
        match self {
            Self::Claude => "LIVE_CLAUDE_BIN",
            Self::Codex => "LIVE_CODEX_BIN",
        }
    }

    fn fixture_dir(self) -> PathBuf {
        Path::new(FIXTURES).join(match self {
            Self::Claude => "claude-code-2.1.296",
            Self::Codex => "codex-0.162.1",
        })
    }
}

fn clean_environment(command: &mut Command) {
    // Agent launchers reject nested sessions based on these ambient markers.
    // Preserve HOME (the real CLI login), PATH, and all other owner settings.
    for (name, _) in std::env::vars_os() {
        let text = name.to_string_lossy();
        if text.starts_with("CLAUDE") || text.starts_with("CODEX_COMPANION") {
            command.env_remove(name);
        }
    }
}

fn executable(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

fn find_binary(requested: &OsStr, context: &str) -> PathBuf {
    let requested_path = Path::new(requested);
    let candidates = if requested_path.components().count() > 1 || requested_path.is_absolute() {
        vec![requested_path.to_owned()]
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|directory| directory.join(requested))
            .collect()
    };
    let path = candidates
        .into_iter()
        .find(|path| executable(path))
        .unwrap_or_else(|| {
            panic!(
                "{context}: requested live test requires binary {requested:?}; looked at that path \
                 or searched PATH, but found no executable (this test does not silently skip)"
            )
        });
    // Do not canonicalize a CLI symlink: launchers can depend on their own path.
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|error| panic!("{context}: current directory: {error}"))
            .join(path)
    }
}

struct Installed {
    kind: Kind,
    binary: PathBuf,
    context: String,
}

impl Installed {
    fn discover(kind: Kind) -> Self {
        let initial = format!("{} version unavailable ({})", kind.name(), kind.bin_env());
        let requested =
            std::env::var_os(kind.bin_env()).unwrap_or_else(|| OsString::from(kind.program()));
        let binary = find_binary(&requested, &initial);
        let mut cli = Self {
            kind,
            context: format!("{initial}, binary {}", binary.display()),
            binary,
        };
        let output = cli.run(&["--version"], LOCAL_TIMEOUT);
        let raw = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        cli.context = format!(
            "{} version {raw:?}, binary {}",
            kind.name(),
            cli.binary.display()
        );
        require_success(&output, &cli.context, "--version");
        let parsed = raw.split_whitespace().find(|token| {
            let mut components = token.trim_start_matches('v').split('.');
            (0..3).all(|_| {
                components.next().is_some_and(|part| {
                    let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
                    !digits.is_empty() && digits.parse::<u32>().is_ok()
                })
            })
        });
        assert!(
            parsed.is_some(),
            "{}: --version has no major.minor.patch version",
            cli.context
        );
        cli
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        clean_environment(&mut command);
        command
    }

    fn run(&self, args: &[&str], duration: Duration) -> Output {
        let mut command = self.command();
        command.args(args);
        capture(&mut command, None, Instant::now() + duration, &self.context)
    }

    fn help(&self, args: &[&str]) -> String {
        let output = self.run(args, LOCAL_TIMEOUT);
        require_success(&output, &self.context, &args.join(" "));
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }
}

// File-backed stdio cannot fill a pipe or hang while an orphan holds it open.
// The guard exists before polling, and kills the entire group on every exit,
// including assertion unwinding and timeouts. No new signal dependency needed.
struct Process {
    child: Child,
}

fn reap_until(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn signal_group(pid: u32, signal: &str) {
    // /bin/kill is itself bounded. It executes no descendants; Child::kill is
    // the fallback if that local signal utility fails to exit promptly.
    if let Ok(mut signal) = Command::new("/bin/kill")
        .args([signal, "--", &format!("-{pid}")])
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        && reap_until(&mut signal, Instant::now() + Duration::from_secs(2)).is_none()
    {
        let _ = signal.kill();
        let _ = reap_until(&mut signal, Instant::now() + Duration::from_secs(2));
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        signal_group(self.child.id(), "-KILL");
        let _ = self.child.kill();
        let _ = reap_until(&mut self.child, Instant::now() + Duration::from_secs(2));
    }
}

fn capture(command: &mut Command, input: Option<File>, deadline: Instant, context: &str) -> Output {
    assert!(
        Instant::now() < deadline,
        "{context}: process deadline expired before spawn"
    );
    let stdout = tempfile::NamedTempFile::new()
        .unwrap_or_else(|error| panic!("{context}: stdout capture: {error}"));
    let stderr = tempfile::NamedTempFile::new()
        .unwrap_or_else(|error| panic!("{context}: stderr capture: {error}"));
    let clone_file = |file: &tempfile::NamedTempFile| {
        file.as_file()
            .try_clone()
            .unwrap_or_else(|error| panic!("{context}: capture handle: {error}"))
    };
    let child = command
        .process_group(0)
        .stdin(input.map_or_else(Stdio::null, Stdio::from))
        .stdout(Stdio::from(clone_file(&stdout)))
        .stderr(Stdio::from(clone_file(&stderr)))
        .spawn()
        .unwrap_or_else(|error| panic!("{context}: could not start {command:?}: {error}"));
    let mut process = Process { child };
    let status = reap_until(&mut process.child, deadline);
    if status.is_none() {
        // Smith owns a separate group for its CLI. Give its SIGINT handler a
        // bounded cleanup grace to cancel that group and remove the endpoint
        // before the guard forcibly kills the Smith group as well.
        signal_group(process.child.id(), "-INT");
        let _ = reap_until(&mut process.child, Instant::now() + Duration::from_secs(2));
    }
    drop(process);
    let stdout =
        fs::read(stdout.path()).unwrap_or_else(|error| panic!("{context}: stdout: {error}"));
    let stderr =
        fs::read(stderr.path()).unwrap_or_else(|error| panic!("{context}: stderr: {error}"));
    let status = status.unwrap_or_else(|| {
        panic!(
            "{context}: process exceeded its deadline; killed process group; stdout={} stderr={}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        )
    });
    Output {
        status,
        stdout,
        stderr,
    }
}

fn require_success(output: &Output, context: &str, step: &str) {
    assert!(
        output.status.success(),
        "{context}: {step} exited {}; stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn json_lines(bytes: &[u8], context: &str) -> Vec<Value> {
    let text = std::str::from_utf8(bytes)
        .unwrap_or_else(|error| panic!("{context}: stdout JSON UTF-8 changed: {error}"));
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str(line).unwrap_or_else(|error| {
                panic!(
                    "{context}: stdout JSON envelope line {} changed: {error}: {line}",
                    index + 1
                )
            })
        })
        .collect()
}

fn fixture(path: &Path, context: &str) -> Vec<Value> {
    json_lines(
        &fs::read(path)
            .unwrap_or_else(|error| panic!("{context}: fixture {}: {error}", path.display())),
        &format!("{context}: fixture {}", path.display()),
    )
}

fn event<'a>(rows: &'a [Value], kind: &str, context: &str) -> &'a Value {
    rows.iter()
        .find(|row| row["type"] == kind)
        .unwrap_or_else(|| panic!("{context}: missing envelope type={kind}; rows={rows:?}"))
}

fn claude_init<'a>(rows: &'a [Value], context: &str) -> &'a Value {
    // Real output can contain system/hook_started before system/init; those
    // hook events were deliberately removed from the redacted recordings.
    rows.iter()
        .find(|row| row["type"] == "system" && row["subtype"] == "init")
        .unwrap_or_else(|| panic!("{context}: missing system/init envelope: {rows:?}"))
}

fn same_type(actual: &Value, recorded: &Value) -> bool {
    matches!(
        (actual, recorded),
        (Value::Null, Value::Null)
            | (Value::Bool(_), Value::Bool(_))
            | (Value::Number(_), Value::Number(_))
            | (Value::String(_), Value::String(_))
            | (Value::Array(_), Value::Array(_))
            | (Value::Object(_), Value::Object(_))
    )
}

fn fields_match(actual: &Value, recorded: &Value, fields: &[&str], context: &str) {
    for field in fields {
        let expected = recorded
            .pointer(field)
            .unwrap_or_else(|| panic!("{context}: recorded field {field} missing"));
        let observed = actual
            .pointer(field)
            .unwrap_or_else(|| panic!("{context}: output field {field} missing: {actual}"));
        assert!(
            same_type(observed, expected),
            "{context}: output field {field} changed type: recorded={expected}, observed={observed}"
        );
        // Identity varies per run. Envelope discriminators and error booleans
        // must retain the values actually recorded, including subtype=success
        // on Claude's error result (subtype alone cannot mean success).
        if matches!(*field, "/type" | "/subtype" | "/is_error") {
            assert_eq!(
                observed, expected,
                "{context}: output field {field} changed"
            );
        }
    }
}

const CLAUDE_INIT_FIELDS: &[&str] = &["/type", "/subtype", "/session_id", "/mcp_servers", "/tools"];
const CLAUDE_RESULT_FIELDS: &[&str] = &["/type", "/subtype", "/session_id", "/is_error", "/result"];
const CODEX_THREAD_FIELDS: &[&str] = &["/type", "/thread_id"];
const CODEX_ERROR_FIELDS: &[&str] = &["/type", "/message"];
const CODEX_FAILED_FIELDS: &[&str] = &["/type", "/error/message"];

fn recorded_failed(kind: Kind, context: &str) -> Vec<Value> {
    fixture(
        &kind.fixture_dir().join("stream/failed-turn.jsonl"),
        context,
    )
}

fn stand_in(context: &str) -> PathBuf {
    // Required location is relative to this test executable, including a
    // non-default target directory or Cargo profile.
    let executable = std::env::current_exe()
        .unwrap_or_else(|error| panic!("{context}: test executable: {error}"));
    let path = executable
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("{context}: test executable has no profile directory"))
        .join("examples/cli_stand_in");
    assert!(
        path.is_file(),
        "{context}: counting server missing at {}; run cargo build -p smith-cli --example cli_stand_in first",
        path.display()
    );
    path
}

fn counting_args(directory: &Path) -> Value {
    json!([
        "--counting-mcp-server",
        "--log",
        directory.join("calls.jsonl"),
        "--handshake-marker",
        directory.join("initialize.json")
    ])
}

fn assert_handshake(directory: &Path, context: &str) {
    let path = directory.join("initialize.json");
    let bytes = fs::read(&path).unwrap_or_else(|error| {
        panic!("{context}: mcp_servers.smith did not handshake (missing initialize marker {}): {error}", path.display())
    });
    let request: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("{context}: initialize marker JSON: {error}"));
    assert_eq!(
        request["method"], "initialize",
        "{context}: handshake method changed"
    );
    assert!(
        request["params"]["protocolVersion"].is_string(),
        "{context}: initialize.params.protocolVersion missing: {request}"
    );
    let log = fs::read(directory.join("calls.jsonl"))
        .unwrap_or_else(|error| panic!("{context}: counting server startup log missing: {error}"));
    assert!(
        log.is_empty(),
        "{context}: impossible-model canary unexpectedly called a model tool"
    );
}

#[test]
#[ignore = "run cargo test -p smith-cli --test live_cli_bridge claude_drift_canary -- --ignored --nocapture; requires installed claude, no model completion"]
fn claude_drift_canary() {
    canary::canary(Kind::Claude);
}

#[test]
#[ignore = "run cargo test -p smith-cli --test live_cli_bridge codex_drift_canary -- --ignored --nocapture; requires installed codex, no model completion"]
fn codex_drift_canary() {
    canary::canary(Kind::Codex);
}

#[test]
fn recorded_streams_retain_the_fields_compared_by_the_canary() {
    for kind in [Kind::Claude, Kind::Codex] {
        let reference = recorded_failed(kind, kind.name());
        let directory = kind.fixture_dir().join("stream");
        let mut count = 0;
        for entry in fs::read_dir(&directory).expect("recorded stream directory") {
            let path = entry.expect("recorded stream entry").path();
            if path.extension() != Some(OsStr::new("jsonl")) {
                continue;
            }
            count += 1;
            let context = format!("recorded {} stream {}", kind.name(), path.display());
            let rows = fixture(&path, &context);
            assert!(!rows.is_empty(), "{context}: empty stream");
            assert!(
                rows.iter().all(|row| row["type"].is_string()),
                "{context}: envelope type missing"
            );
            match kind {
                Kind::Claude => {
                    let init = claude_init(&rows, &context);
                    fields_match(
                        init,
                        claude_init(&reference, &context),
                        CLAUDE_INIT_FIELDS,
                        &context,
                    );
                    assert!(
                        init["session_id"].is_string(),
                        "{context}: init.session_id must be a string"
                    );
                    assert!(
                        init["tools"]
                            .as_array()
                            .expect("tools array")
                            .iter()
                            .all(Value::is_string),
                        "{context}: tools[] must be strings"
                    );
                    for server in init["mcp_servers"].as_array().expect("servers array") {
                        fields_match(
                            server,
                            &claude_init(&reference, &context)["mcp_servers"][0],
                            &["/name", "/status"],
                            &context,
                        );
                    }
                    let result = event(&rows, "result", &context);
                    assert!(
                        result["result"].is_string() && result["session_id"].is_string(),
                        "{context}: result.result and result.session_id must be strings"
                    );
                    // Successful streams have is_error=false; the canary's
                    // failed-turn baseline must specifically keep true.
                    assert!(
                        result["is_error"].is_boolean(),
                        "{context}: result.is_error missing"
                    );
                    let mut shape = event(&reference, "result", &context).clone();
                    shape["is_error"] = result["is_error"].clone();
                    fields_match(result, &shape, CLAUDE_RESULT_FIELDS, &context);
                }
                Kind::Codex => {
                    assert!(
                        event(&rows, "thread.started", &context)["thread_id"].is_string(),
                        "{context}: thread.started.thread_id must be a string"
                    );
                    fields_match(
                        event(&rows, "thread.started", &context),
                        event(&reference, "thread.started", &context),
                        CODEX_THREAD_FIELDS,
                        &context,
                    );
                    for (name, fields) in [
                        ("error", CODEX_ERROR_FIELDS),
                        ("turn.failed", CODEX_FAILED_FIELDS),
                    ] {
                        for row in rows.iter().filter(|row| row["type"] == name) {
                            let field = if name == "error" {
                                "/message"
                            } else {
                                "/error/message"
                            };
                            assert!(
                                row.pointer(field).is_some_and(Value::is_string),
                                "{context}: {name}{field} must be a string"
                            );
                            fields_match(row, event(&reference, name, &context), fields, &context);
                        }
                    }
                    assert!(
                        rows.iter().any(|row| matches!(
                            row["type"].as_str(),
                            Some("turn.completed" | "turn.failed")
                        )),
                        "{context}: terminal envelope missing"
                    );
                }
            }
        }
        assert!(count > 0, "{}: no stream/*.jsonl fixtures", kind.name());
        match kind {
            Kind::Claude => {
                assert_eq!(
                    event(&reference, "result", "Claude fixture")["is_error"],
                    true
                );
                let init = claude_init(&reference, "Claude fixture");
                assert_eq!(init["mcp_servers"][0]["status"], "connected");
                assert!(init["tools"].as_array().expect("tools").iter().any(|tool| {
                    tool.as_str()
                        .is_some_and(|name| name.starts_with("mcp__spike__"))
                }));
            }
            Kind::Codex => {
                for name in ["error", "turn.failed"] {
                    event(&reference, name, "Codex failed-turn fixture");
                }
            }
        }
    }
}

#[test]
fn counting_server_marks_initialize_without_counting_a_tool_call() {
    let context = "counting MCP handshake marker contract (no installed CLI)";
    let directory = tempfile::tempdir().expect("counting contract directory");
    let log = directory.path().join("calls.jsonl");
    let marker = directory.path().join("initialize.json");
    let request = json!({"jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"protocolVersion":"2024-11-05","capabilities":{},
            "clientInfo":{"name":"live-cli-bridge-contract","version":"1"}}});
    // EOF after initialize makes the direct server run terminate locally;
    // no CLI installation, account, socket, or model is involved.
    let mut input = tempfile::tempfile().expect("protocol input");
    writeln!(input, "{request}").expect("initialize request");
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .expect("initialized notification");
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})
    )
    .expect("list request");
    input
        .seek(SeekFrom::Start(0))
        .expect("rewound protocol input");
    let mut command = Command::new(stand_in(context));
    command
        .arg("--counting-mcp-server")
        .arg("--log")
        .arg(&log)
        .arg("--handshake-marker")
        .arg(&marker);
    let startup = capture(&mut command, None, Instant::now() + LOCAL_TIMEOUT, context);
    require_success(&startup, context, "startup without initialize");
    assert!(
        log.is_file(),
        "{context}: startup must create the invocation log"
    );
    assert!(
        !marker.exists(),
        "{context}: process startup alone must not create the handshake marker"
    );
    let output = capture(
        &mut command,
        Some(input),
        Instant::now() + LOCAL_TIMEOUT,
        context,
    );
    require_success(&output, context, "initialize + tools/list");
    assert_handshake(directory.path(), context);
    let recorded: Value =
        serde_json::from_slice(&fs::read(marker).expect("marker")).expect("marker JSON");
    assert_eq!(
        recorded, request,
        "marker must be written by initialize, not process startup"
    );
    let rows = json_lines(&output.stdout, context);
    assert_eq!(rows.len(), 2, "notifications must not get a response");
    assert_eq!(rows[0]["id"], 1);
    assert_eq!(rows[0]["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(rows[1]["id"], 2);
    assert!(
        rows[1]["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .any(|tool| tool["name"] == "count")
    );
}

#[test]
#[ignore = "run cargo test -p smith-cli --test live_cli_bridge claude_live_flow -- --ignored --nocapture; uses real HOME, Smith config, claude login and spends tokens"]
fn claude_live_flow() {
    live_flow::live_flow(Kind::Claude);
}

#[test]
#[ignore = "run cargo test -p smith-cli --test live_cli_bridge codex_live_flow -- --ignored --nocapture; uses real HOME, Smith config, codex login and spends tokens"]
fn codex_live_flow() {
    live_flow::live_flow(Kind::Codex);
}
