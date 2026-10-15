//! Exercise the real hidden command without configuration or fixture teardown
//! masking a surviving server.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::thread;
use std::time::Duration;

use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::{Pid, getpgid};
use serde_json::json;

use super::super::support::{Deadline, Process};

struct Fixture {
    directory: tempfile::TempDir,
    deadline: Deadline,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join(".smith")).unwrap();
        fs::write(directory.path().join(".smith/config.toml"), "invalid [").unwrap();
        Self {
            directory,
            deadline: Deadline::after(Duration::from_secs(30)),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_smith"));
        command
            .current_dir(self.directory.path())
            .env_clear()
            .env("HOME", self.directory.path())
            .env("PATH", "/usr/bin:/bin")
            .env("WATCHDOG_TEST", "inherited");
        command
    }

    fn watchdog(&self, program: &str, args: &[&str]) -> Process {
        Process::spawn(
            self.command()
                .args([
                    "mcp-watchdog",
                    "--parent",
                    &std::process::id().to_string(),
                    "--",
                    program,
                ])
                .args(args),
            self.deadline,
        )
    }

    fn pid(&self, name: &str) -> u32 {
        let path = self.directory.path().join(name);
        loop {
            self.deadline.remaining("watchdog child readiness");
            if let Ok(text) = fs::read_to_string(&path)
                && let Ok(pid) = text.trim().parse()
            {
                return pid;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn signal(pid: u32, signal: Signal) {
    kill(Pid::from_raw(pid as i32), signal).unwrap();
}

fn reaped(pid: u32, deadline: Deadline) {
    loop {
        if kill(Pid::from_raw(pid as i32), None) == Err(Errno::ESRCH) {
            return;
        }
        deadline.remaining("watchdog must kill and reap its server");
        thread::sleep(Duration::from_millis(10));
    }
}

struct Cleanup(u32);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = kill(Pid::from_raw(self.0 as i32), Signal::SIGKILL);
    }
}

const BUSY: &str = "trap '' TERM; echo $$ > server.pid; while :; do :; done";

#[test]
fn parent_death_kills_and_reaps_a_busy_server() {
    let fixture = Fixture::new();
    let mut command = Command::new("/bin/sh");
    command.current_dir(fixture.directory.path()).args([
        "-c",
        r#""$1" mcp-watchdog --parent "$$" -- /bin/sh -c "$2" &
echo $! > watchdog.pid
wait "$!"
"#,
        "parent",
        env!("CARGO_BIN_EXE_smith"),
        BUSY,
    ]);
    let parent = Process::spawn(&mut command, fixture.deadline);
    let watchdog = fixture.pid("watchdog.pid");
    let server = fixture.pid("server.pid");
    let _watchdog_cleanup = Cleanup(watchdog);
    let _server_cleanup = Cleanup(server);
    assert_eq!(
        getpgid(Some(Pid::from_raw(server as i32)))
            .unwrap()
            .as_raw(),
        parent.id() as i32
    );
    signal(parent.id(), Signal::SIGKILL);
    // Assert before Process::finish, which kills the fixture's process group.
    reaped(server, Deadline::after(Duration::from_secs(5)));
    let output = parent.finish(None);
    assert_eq!(output.status.signal(), Some(9), "{output:?}");
}

#[test]
fn sigterm_stops_and_reaps_server_with_its_status() {
    for (script, expected) in [
        (
            "trap 'exit 42' TERM; echo $$ > server.pid; while :; do :; done",
            42,
        ),
        (BUSY, 137),
    ] {
        let fixture = Fixture::new();
        let watchdog = fixture.watchdog("/bin/sh", &["-c", script]);
        let server = fixture.pid("server.pid");
        let _cleanup = Cleanup(server);
        signal(watchdog.id(), Signal::SIGTERM);
        reaped(server, fixture.deadline);
        let output = watchdog.finish(None);
        assert_eq!(output.status.code(), Some(expected), "{output:?}");
    }
}

#[test]
fn interrupt_and_hangup_leave_watchdog_and_stdio_usable() {
    let fixture = Fixture::new();
    let mut watchdog = fixture.watchdog(
        "/bin/sh",
        &[
            "-c",
            "echo $$ > server.pid; while read line; do printf '%s\\n' \"$line\"; done",
        ],
    );
    let server = fixture.pid("server.pid");
    let _cleanup = Cleanup(server);
    for ignored in [Signal::SIGINT, Signal::SIGHUP] {
        signal(watchdog.id(), ignored);
        watchdog
            .write_stdin(b"{\"echo\":true}\n", "echo through watchdog")
            .unwrap();
        assert_eq!(
            watchdog.next("echo response after signal"),
            json!({"echo":true})
        );
    }
    watchdog.close_stdin();
    reaped(server, fixture.deadline);
    assert!(watchdog.finish(None).status.success());
}

#[test]
fn child_status_stdio_environment_and_directory_pass_through() {
    let fixture = Fixture::new();
    let mut watchdog = fixture.watchdog(
        "/bin/sh",
        &[
            "-c",
            r#"echo $$ > server.pid
read line
printf '%s\n' "$line"
printf '%s\n' "$WATCHDOG_TEST" >&2
test -f .smith/config.toml || exit 99
exit 23"#,
        ],
    );
    let server = fixture.pid("server.pid");
    watchdog
        .write_stdin(b"{\"stdio\":true}\n", "write echo line")
        .unwrap();
    assert_eq!(watchdog.next("read echo line"), json!({"stdio":true}));
    reaped(server, fixture.deadline);
    let output = watchdog.finish(None);
    assert_eq!(output.status.code(), Some(23), "{output:?}");
    assert_eq!(output.stderr, b"inherited\n");

    let watchdog = fixture.watchdog("/bin/sh", &["-c", "kill -USR1 $$"]);
    assert_eq!(
        watchdog.finish(None).status.code(),
        Some(128 + Signal::SIGUSR1 as i32)
    );
}

#[test]
fn spawn_errors_follow_shell_status_conventions() {
    let fixture = Fixture::new();
    let denied = fixture.directory.path().join("denied");
    fs::write(&denied, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o600)).unwrap();
    let invalid = fixture.directory.path().join("invalid");
    // execvp may retry text without a shebang through a shell on macOS.
    fs::write(&invalid, b"\0not an executable format").unwrap();
    fs::set_permissions(&invalid, fs::Permissions::from_mode(0o700)).unwrap();
    for (path, expected) in [
        (fixture.directory.path().join("missing"), 127),
        (denied, 126),
        (invalid, 126),
    ] {
        let watchdog = fixture.watchdog(path.to_str().unwrap(), &[]);
        let output = watchdog.finish(None);
        assert_eq!(
            output.status.code(),
            Some(expected),
            "{}: {output:?}",
            path.display()
        );
    }
}

#[test]
fn already_dead_parent_still_stops_server_and_exits_one() {
    let fixture = Fixture::new();
    let watchdog = Process::spawn(
        fixture
            .command()
            .args(["mcp-watchdog", "--parent", "1", "--", "/bin/sh", "-c", BUSY]),
        fixture.deadline,
    );
    assert_eq!(watchdog.finish(None).status.code(), Some(1));
}
