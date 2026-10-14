//! Black-box lifetime contracts for the private supervisor, without setup.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill, killpg};
use nix::unistd::Pid;

#[derive(Clone, Copy)]
struct Deadline(Instant);

impl Deadline {
    fn new() -> Self {
        Self(Instant::now() + Duration::from_secs(10))
    }

    fn remaining(self) -> Duration {
        self.0
            .checked_duration_since(Instant::now())
            .expect("supervisor deadline exceeded")
    }

    fn until(self, mut ready: impl FnMut() -> bool) {
        while !ready() {
            self.remaining();
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn receive<T>(self, receiver: &Receiver<T>) -> T {
        receiver
            .recv_timeout(self.remaining())
            .expect("pipe drained before deadline")
    }
}

struct Group(i32);

impl Drop for Group {
    fn drop(&mut self) {
        let _ = killpg(Pid::from_raw(self.0), Signal::SIGKILL);
    }
}

// Reuse the pipe capture that kills Smith's group after exit and bounds EOF.
// A busy stdio MCP server belongs to Smith's group, not the harness group.
struct Process {
    inner: Option<super::support::Process>,
}

impl Process {
    fn spawn(command: &mut Command, deadline: Deadline) -> Self {
        Self {
            inner: Some(super::support::Process::spawn(
                command,
                super::support::Deadline::after(deadline.remaining()),
            )),
        }
    }

    fn finish(&mut self) -> Output {
        self.inner.take().expect("running fixture").finish(None)
    }

    fn close_stdin(&mut self) {
        self.inner.as_mut().expect("running fixture").close_stdin();
    }

    fn signal(&self, signal: Signal) {
        kill(
            Pid::from_raw(
                i32::try_from(self.inner.as_ref().expect("running fixture").id()).unwrap(),
            ),
            signal,
        )
        .unwrap();
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    deadline: Deadline,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join(".smith")).unwrap();
        fs::write(
            directory.path().join(".smith/config.toml"),
            "invalid TOML [",
        )
        .unwrap();
        Self {
            directory,
            deadline: Deadline::new(),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_smith"));
        command
            .current_dir(self.directory.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin");
        command
    }

    fn supervise(&self, program: &str, args: &[&str]) -> Process {
        Process::spawn(
            self.command()
                .args(["harness-supervise", "--", program])
                .args(args),
            self.deadline,
        )
    }

    fn pid(&self, name: &str) -> i32 {
        let path = self.directory.path().join(name);
        let mut pid = None;
        self.deadline.until(|| {
            pid = fs::read_to_string(&path)
                .ok()
                .and_then(|s| s.trim().parse().ok());
            pid.is_some()
        });
        pid.unwrap()
    }

    fn stopped(&self, pid: i32) {
        // Orphans may remain as zombies until init reaps them; a zombie is
        // stopped and cannot hold pipes or perform another tool call.
        self.deadline.until(|| !live(pid, self.deadline));
    }
}

fn live(pid: i32, deadline: Deadline) -> bool {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let output = super::support::run_command(
            Command::new("/bin/ps").args(["-o", "stat=", "-p", &pid.to_string()]),
            super::support::Deadline::after(deadline.remaining()),
        );
        let _ = sender.send(output);
    });
    let output = deadline.receive(&receiver);
    let status = String::from_utf8(output.stdout).unwrap();
    !status.trim().is_empty() && !status.trim().starts_with('Z')
}

const TREE: &str = "echo $$ > program.pid; sleep 300 & echo $! > grandchild.pid; wait";

#[test]
fn stdout_stderr_and_exit_codes_pass_through_without_setup() {
    for code in [0, 3] {
        let fixture = Fixture::new();
        let mut process = fixture.supervise(
            "/bin/sh",
            &[
                "-c",
                "printf 'out\\000\\377\\n'; printf 'err\\000\\376\\n' >&2; exit \"$1\"",
                "child",
                &code.to_string(),
            ],
        );
        let output = process.finish();
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        assert_eq!(output.stdout, b"out\0\xff\n");
        assert_eq!(output.stderr, b"err\0\xfe\n");
    }
    let fixture = Fixture::new();
    let output = fixture
        .supervise("/bin/sh", &["-c", "kill -TERM $$"])
        .finish();
    assert_eq!(output.status.code(), Some(143), "{output:?}");
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}

#[test]
fn stdin_eof_kills_supervisor_program_and_grandchild() {
    let fixture = Fixture::new();
    let mut process = fixture.supervise("/bin/sh", &["-c", TREE]);
    let program = fixture.pid("program.pid");
    let grandchild = fixture.pid("grandchild.pid");
    assert!(live(program, fixture.deadline) && live(grandchild, fixture.deadline));
    process.close_stdin();
    fixture.stopped(program);
    fixture.stopped(grandchild);
    let output = process.finish();
    assert_eq!(output.status.signal(), Some(9));
}

#[test]
fn sigkill_of_the_parent_holding_stdin_stops_the_supervisor_group() {
    let fixture = Fixture::new();
    // Bash's noninteractive job control makes the supervisor a group leader.
    // Only the parent shell opens the FIFO for writing; no helper process can
    // accidentally keep the watchdog alive after that shell receives SIGKILL.
    let mut command = Command::new("/bin/bash");
    command
        .current_dir(fixture.directory.path())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args([
            "-c",
            r#"set -m
mkfifo lifetime
"$1" harness-supervise -- /bin/sh -c "$2" < lifetime &
echo $! > supervisor.pid
exec 3> lifetime
echo ready > parent.ready
wait
"#,
            "parent",
            env!("CARGO_BIN_EXE_smith"),
            TREE,
        ]);
    let mut parent = Process::spawn(&mut command, fixture.deadline);
    let supervisor = fixture.pid("supervisor.pid");
    let _cleanup = Group(supervisor);
    let program = fixture.pid("program.pid");
    let grandchild = fixture.pid("grandchild.pid");
    fixture
        .deadline
        .until(|| fixture.directory.path().join("parent.ready").exists());
    assert!(
        live(supervisor, fixture.deadline)
            && live(program, fixture.deadline)
            && live(grandchild, fixture.deadline)
    );
    parent.signal(Signal::SIGKILL);
    for pid in [supervisor, program, grandchild] {
        fixture.stopped(pid);
    }
    drop(_cleanup);
    let output = parent.finish();
    assert_eq!(output.status.signal(), Some(9));
}

#[test]
fn signals_to_the_supervisor_kill_its_whole_group() {
    for signal in [Signal::SIGTERM, Signal::SIGINT, Signal::SIGHUP] {
        let fixture = Fixture::new();
        let mut process = fixture.supervise("/bin/sh", &["-c", TREE]);
        let program = fixture.pid("program.pid");
        let grandchild = fixture.pid("grandchild.pid");
        process.signal(signal);
        fixture.stopped(program);
        fixture.stopped(grandchild);
        let output = process.finish();
        assert_eq!(output.status.signal(), Some(9), "{signal:?}: {output:?}");
    }
}

#[test]
fn spawn_errors_have_one_stderr_line_and_shell_compatible_status() {
    let fixture = Fixture::new();
    let blocked = fixture.directory.path().join("not-executable");
    fs::write(&blocked, "unexecutable").unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o600)).unwrap();
    for (path, status) in [
        (Path::new("/smith-program-does-not-exist"), 127),
        (blocked.as_path(), 126),
    ] {
        let output = fixture.supervise(path.to_str().unwrap(), &[]).finish();
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        assert!(stderr.contains("os error"), "{stderr}");
    }
}

#[test]
fn missing_separator_or_program_is_a_usage_error() {
    let fixture = Fixture::new();
    for args in [
        vec!["harness-supervise"],
        vec!["harness-supervise", "/bin/true"],
        vec!["harness-supervise", "--"],
    ] {
        let output = Process::spawn(fixture.command().args(args), fixture.deadline).finish();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("usage: smith harness-supervise -- <program> [args...]")
        );
    }
}

#[test]
fn help_and_command_listings_do_not_discover_the_supervisor() {
    let fixture = Fixture::new();
    for path in [
        &[][..],
        &["config"][..],
        &["config", "explain"][..],
        &["config", "modules"][..],
        &["sessions"][..],
        &["sessions", "list"][..],
        &["plugin"][..],
        &["plugin", "marketplace"][..],
        &["setup"][..],
        &["setup", "add-provider"][..],
    ] {
        for help in ["help", "-h", "--help"] {
            let output =
                Process::spawn(fixture.command().args(path).arg(help), fixture.deadline).finish();
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty());
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(text.contains("USAGE:"), "{text}");
            assert!(!text.contains("harness-supervise"), "{text}");
        }
    }
}

#[test]
fn the_program_gets_null_stdin_and_unchanged_arguments_and_directory() {
    let fixture = Fixture::new();
    let output = fixture
        .supervise(
            "/bin/sh",
            &[
                "-c",
                r#"
if read -r line; then exit 9; fi
[ -z "${HOME+x}" ] || exit 8
printf '%s\n' "$PWD" "$@"
"#,
                "child",
                "--",
                "spaces and 'quotes'",
                "two\nlines",
                "é",
            ],
        )
        .finish();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "{}\n--\nspaces and 'quotes'\ntwo\nlines\né\n",
            fixture.directory.path().canonicalize().unwrap().display()
        )
    );
}

#[test]
fn each_headless_signal_reports_cancelled_and_saves_the_interrupted_call() {
    use super::bridge::{BridgeFixture, cleaned, history_parts, log_lines};
    use super::support::{recorded_run, script_files};
    use serde_json::{Value, json};

    for codex in [false, true] {
        for signal in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP] {
            let fixture = BridgeFixture::new(codex, false);
            let deadline = Deadline::new();
            let log = fixture.counting(true, false);
            let pids = fixture.smith.project.path().join("signal-pids.json");
            let (script, record) = script_files(
                fixture.smith.project.path(),
                &json!([
                    {"spawn_child":{"seconds":60,"pid_file":pids}},
                    {"call":{"tool":"mcp__docs__slow_count","arguments":{"seconds":60,"signal_probe":true}}},
                    {"call":{"tool":"mcp__docs__count","arguments":{"must_not_run":true}}}
                ]),
            );
            let mut command = fixture.command(None, "allow-all");
            command
                .env("CLI_STAND_IN_SCRIPT", script)
                .env("CLI_STAND_IN_RECORD", &record);
            let mut process = Process::spawn(&mut command, deadline);
            deadline.until(|| log.with_extension("started.jsonl").exists());
            fixture.cleanup.capture();
            let pid_values: Value = serde_json::from_slice(&fs::read(pids).unwrap()).unwrap();
            let harness = i32::try_from(pid_values["stand_in_pid"].as_u64().unwrap()).unwrap();
            let child = i32::try_from(pid_values["child_pid"].as_u64().unwrap()).unwrap();
            // The supervisor, rather than the stand-in, now leads the group.
            let _cleanup = Group(
                nix::unistd::getpgid(Some(Pid::from_raw(harness)))
                    .unwrap()
                    .as_raw(),
            );
            let recorded: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
            let endpoint = Path::new(recorded["server"]["args"][1].as_str().unwrap()).to_owned();
            process.signal(signal);
            // Observe product cleanup before fixture teardown can hide a leak.
            deadline.until(|| !live(harness, deadline) && !live(child, deadline));
            drop(_cleanup);
            let run = recorded_run(process.finish(), &record);
            let result = run.lines.last().unwrap();
            assert_eq!(
                result["status"], "cancelled",
                "{signal:?}: {:?}",
                run.output
            );
            assert_eq!(
                run.output.status.code(),
                Some(1),
                "{signal:?}: {:?}",
                run.output
            );
            cleaned(&endpoint);
            assert!(log_lines(&log).is_empty());
            let snapshot = fixture.snapshot(result["session_id"].as_str().unwrap());
            assert!(
                history_parts(&snapshot).iter().any(|part| {
                    part["type"] == "tool_result"
                        && part["name"] == "mcp__docs__slow_count"
                        && part["is_error"] == true
                        && part["content"]
                            .to_string()
                            .to_lowercase()
                            .contains("cancel")
                }),
                "{snapshot}"
            );
        }
    }
}
