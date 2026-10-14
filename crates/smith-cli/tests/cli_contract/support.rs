//! Process helpers shared by relay and scripted CLI bridge contracts.

use std::io::{self, BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

#[path = "bridge_cleanup.rs"]
mod bridge_cleanup;
#[allow(unused_imports)]
pub(crate) use bridge_cleanup::BridgeCleanup;

// Once the process exits and its group is killed, allow loaded runners time
// to schedule the EOF readers independently of the process's original deadline.
pub(crate) const POST_EXIT_DRAIN_GRACE: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
pub(crate) struct Deadline(Instant);

impl Deadline {
    pub(crate) fn after(duration: Duration) -> Self {
        Self(Instant::now() + duration)
    }

    pub(crate) fn remaining(self, step: &str) -> Duration {
        self.0
            .checked_duration_since(Instant::now())
            .unwrap_or_else(|| panic!("deadline exceeded during {step}"))
    }

    pub(crate) fn receive<T>(self, receiver: &Receiver<T>, step: &str) -> T {
        receiver
            .recv_timeout(self.remaining(step))
            .unwrap_or_else(|error| panic!("{step} did not finish before the deadline: {error}"))
    }

    pub(crate) fn work<T: Send + 'static>(
        self,
        step: &str,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(work());
        });
        self.receive(&receiver, step)
    }
}

pub(crate) fn stand_in_path() -> PathBuf {
    // Cargo builds package examples for integration tests, but does not install
    // them with smith; the fixture therefore lives beside the test binary build.
    let path = Path::new(env!("CARGO_BIN_EXE_smith"))
        .parent()
        .expect("smith binary directory")
        .join("examples")
        .join("cli_stand_in");
    assert!(
        path.is_file(),
        "CLI stand-in example missing at {}; build it with cargo build -p smith-cli --example cli_stand_in",
        path.display()
    );
    path
}

pub(crate) fn socket_directory() -> tempfile::TempDir {
    // macOS limits Unix socket paths to 104 bytes; its default temp base can
    // consume that budget before the socket's own name is appended.
    tempfile::Builder::new()
        .prefix("smb-")
        .tempdir_in("/tmp")
        .expect("a short socket directory")
}

pub(crate) struct Process {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<io::Result<Vec<u8>>>,
    stderr: Receiver<io::Result<Vec<u8>>>,
    stderr_so_far: Arc<Mutex<Vec<u8>>>,
    bytes: Vec<u8>,
    deadline: Deadline,
}

impl Process {
    pub(crate) fn spawn(command: &mut Command, deadline: Deadline) -> Self {
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .env(
                "CLI_STAND_IN_WAIT_TIMEOUT_MS",
                deadline.remaining("fixture spawn").as_millis().to_string(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("fixture process started");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout");
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut bytes = Vec::new();
                let result = reader.read_until(b'\n', &mut bytes).map(|_| bytes);
                let done = result.as_ref().map_or(true, Vec::is_empty);
                if sender.send(result).is_err() || done {
                    break;
                }
            }
        });
        let stderr = child.stderr.take().expect("stderr");
        let (error_sender, error_receiver) = mpsc::channel();
        let stderr_so_far = Arc::new(Mutex::new(Vec::new()));
        let captured = stderr_so_far.clone();
        thread::spawn(move || {
            let mut reader = stderr;
            let result = (|| {
                let mut chunk = [0; 4096];
                loop {
                    let read = reader.read(&mut chunk)?;
                    if read == 0 {
                        break;
                    }
                    captured
                        .lock()
                        .expect("stderr capture")
                        .extend_from_slice(&chunk[..read]);
                }
                Ok(captured.lock().expect("stderr capture").clone())
            })();
            let _ = error_sender.send(result);
        });
        Self {
            child,
            stdin,
            stdout: receiver,
            stderr: error_receiver,
            stderr_so_far,
            bytes: Vec::new(),
            deadline,
        }
    }

    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn send(&mut self, frame: &Value, step: &str) {
        let mut bytes = serde_json::to_vec(frame).expect("JSON request");
        bytes.push(b'\n');
        self.write_stdin(&bytes, step)
            .expect("fixture stdin written");
    }

    pub(crate) fn write_stdin(&mut self, bytes: &[u8], step: &str) -> io::Result<()> {
        let mut stdin = self.stdin.take().expect("open stdin");
        let bytes = bytes.to_vec();
        let (stdin, result) = self.deadline.work(step, move || {
            let result = stdin.write_all(&bytes).and_then(|()| stdin.flush());
            (stdin, result)
        });
        self.stdin = Some(stdin);
        result
    }

    pub(crate) fn next(&mut self, step: &str) -> Value {
        let bytes = self
            .deadline
            .receive(&self.stdout, step)
            .expect("stdout line");
        assert!(!bytes.is_empty(), "stdout ended during {step}");
        self.bytes.extend_from_slice(&bytes);
        serde_json::from_slice(&bytes).expect("JSON stdout line")
    }

    pub(crate) fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }

    pub(crate) fn kill_group(&self) {
        kill_group(self.child.id());
    }

    pub(crate) fn finish(mut self, record: Option<&Path>) -> Output {
        let status = loop {
            let step = record
                .and_then(|path| std::fs::read(path).ok())
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .map_or_else(
                    || "fixture process exit".to_owned(),
                    |value| {
                        format!(
                            "stand-in script step {}: {}",
                            value["current_step"],
                            value["steps"]
                                .as_array()
                                .and_then(|steps| steps.last())
                                .unwrap_or(&Value::Null)
                        )
                    },
                );
            self.deadline.remaining(&step);
            if let Some(status) = self.child.try_wait().expect("process status") {
                break status;
            }
            thread::sleep(Duration::from_millis(5));
        };
        // Smith's fixture-owned group may still contain a counting server
        // after an uncatchable signal. Its independently grouped harness is
        // checked by the lifetime tests before this teardown runs.
        self.kill_group();
        // EOF can be held open by an orphan, even after Smith has exited.
        // Bound the drain separately and retain partial diagnostics on failure.
        let drain_end = Instant::now() + POST_EXIT_DRAIN_GRACE;
        let captured_error = self.stderr_so_far.clone();
        let drain = |receiver: &Receiver<io::Result<Vec<u8>>>, stream: &str, stdout: &[u8]| {
            receiver.recv_timeout(drain_end.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    let reason = match error {
                        mpsc::RecvTimeoutError::Timeout => "10 s post-exit drain grace expired",
                        mpsc::RecvTimeoutError::Disconnected => "reader disconnected before EOF",
                    };
                    panic!("draining fixture {stream} failed: {reason}; stdout read so far: {}; stderr read so far: {}",
                    String::from_utf8_lossy(stdout),
                    String::from_utf8_lossy(&captured_error.lock().expect("stderr capture")))
                })
                .expect("fixture stream bytes")
        };
        loop {
            let bytes = drain(&self.stdout, "stdout", &self.bytes);
            if bytes.is_empty() {
                break;
            }
            self.bytes.extend(bytes);
        }
        let stderr = drain(&self.stderr, "stderr", &self.bytes);
        Output {
            status,
            stdout: std::mem::take(&mut self.bytes),
            stderr,
        }
    }
}

/// Captures a one-shot command with the same group and drain guarantees as
/// bridge fixtures. Close stdin so commands cannot wait for test input.
pub(crate) fn run_command(command: &mut Command, deadline: Deadline) -> Output {
    let mut process = Process::spawn(command, deadline);
    process.close_stdin();
    process.finish(None)
}

pub(crate) fn kill_group(pid: u32) {
    #[cfg(not(unix))]
    let _ = pid;
    #[cfg(unix)]
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

impl Drop for Process {
    fn drop(&mut self) {
        // Timeout and assertion failures must stop every child holding a pipe.
        self.kill_group();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) struct StandInRun {
    pub(crate) output: Output,
    pub(crate) lines: Vec<Value>,
    pub(crate) record: Value,
}

impl StandInRun {
    pub(crate) fn invocations(&self) -> &[Value] {
        self.record["invocations"]
            .as_array()
            .map_or_else(|| std::slice::from_ref(&self.record), Vec::as_slice)
    }
}

pub(crate) fn script_command(
    directory: &Path,
    args: &[String],
    script: &Value,
) -> (Command, PathBuf) {
    let (script_path, record_path) = script_files(directory, script);
    let mut command = Command::new(stand_in_path());
    command
        .args(args)
        .current_dir(directory)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("MCP_TOOL_TIMEOUT", "604800000")
        .env("MCP_TIMEOUT", "60000")
        .env("CLI_STAND_IN_SCRIPT", script_path)
        .env("CLI_STAND_IN_RECORD", &record_path);
    (command, record_path)
}

pub(crate) fn run_stand_in(
    directory: &Path,
    args: &[String],
    script: &Value,
    deadline: Deadline,
) -> StandInRun {
    let (mut command, record_path) = script_command(directory, args, script);
    let output = Process::spawn(&mut command, deadline).finish(Some(&record_path));
    recorded_run(output, &record_path)
}

pub(crate) fn script_files(directory: &Path, script: &Value) -> (PathBuf, PathBuf) {
    for entry in std::fs::read_dir(directory).expect("script directory") {
        let path = entry.expect("script entry").path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("record.") && name.ends_with(".invocation.json"))
        {
            std::fs::remove_file(path).expect("previous invocation removed");
        }
    }
    let script_path = directory.join("script.json");
    let record_path = directory.join("record.json");
    std::fs::write(
        &script_path,
        serde_json::to_vec(script).expect("script JSON"),
    )
    .expect("script file");
    // A second run must prove it wrote a fresh record even if startup fails.
    if record_path.exists() {
        std::fs::remove_file(&record_path).expect("previous stand-in record removed");
    }
    (script_path, record_path)
}

/// Runs Smith with a script inherited by its harness, using the same deadline
/// and recording format as the independent stand-in contracts.
pub(crate) fn run_scripted_smith(
    command: &mut Command,
    directory: &Path,
    script: &Value,
    deadline: Deadline,
) -> StandInRun {
    let (script_path, record_path) = script_files(directory, script);
    command
        .env("CLI_STAND_IN_SCRIPT", script_path)
        .env("CLI_STAND_IN_RECORD", &record_path);
    let output = Process::spawn(command, deadline).finish(Some(&record_path));
    recorded_run(output, &record_path)
}

pub(crate) fn recorded_run(output: Output, record_path: &Path) -> StandInRun {
    let lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON stdout event"))
        .collect();
    let mut record: Value =
        serde_json::from_slice(&std::fs::read(record_path).unwrap_or_else(|error| {
            panic!("stand-in record missing: {error}; smith output: {output:?}")
        }))
        .expect("record JSON");
    let mut invocations = std::fs::read_dir(record_path.parent().expect("record directory"))
        .expect("invocation directory")
        .map(|entry| entry.expect("invocation entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("record.") && name.ends_with(".invocation.json")
                })
        })
        .map(|path| {
            serde_json::from_slice::<Value>(&std::fs::read(path).expect("invocation bytes"))
                .expect("invocation JSON")
        })
        .collect::<Vec<_>>();
    invocations.sort_by_key(|record| {
        record["started"]
            .as_str()
            .expect("start time")
            .parse::<u128>()
            .expect("start nanoseconds")
    });
    record["invocations"] = serde_json::to_value(invocations).expect("invocations");
    StandInRun {
        output,
        lines,
        record,
    }
}

/// Resolves one saved session file without duplicating Smith's project-id rule.
pub(crate) fn session_file(home: &Path, session: &str, suffix: &str) -> PathBuf {
    let name = format!("{session}{suffix}");
    let directory = home.join(".smith/sessions");
    let mut files = Vec::new();
    let mut pending = vec![directory.clone()];
    while let Some(path) = pending.pop() {
        match std::fs::read_dir(&path) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.expect("session directory entry");
                    let kind = entry.file_type().expect("session entry type");
                    if kind.is_dir() {
                        pending.push(entry.path());
                    } else {
                        files.push(entry.path());
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("reading sessions at {}: {error}", path.display()),
        }
    }
    files.sort();
    // Root and durable child snapshots live directly in the project directory.
    // Inventory subdirectories too so a changed layout is evident in failures.
    let matches = files
        .iter()
        .filter(|path| {
            path.file_name().is_some_and(|file| file == name.as_str())
                && path.parent().and_then(Path::parent) == Some(directory.as_path())
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "one saved {name} under {}: {matches:?}; existing files: {files:?}",
        directory.display()
    );
    matches.into_iter().next().expect("session file")
}
