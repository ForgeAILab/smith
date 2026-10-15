//! Contracts for the hidden transport, without configuration or credentials.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use super::support::{Deadline, POST_EXIT_DRAIN_GRACE, kill_group};

struct Fixture {
    directory: tempfile::TempDir,
    socket: PathBuf,
    listener: UnixListener,
    deadline: Deadline,
}

impl Fixture {
    fn new() -> Self {
        let deadline = Deadline::after(Duration::from_secs(30));
        // macOS limits Unix socket paths to 104 bytes; its default temp base
        // can consume that budget before the socket's own name is appended.
        let directory = tempfile::Builder::new()
            .prefix("smr-")
            .tempdir_in("/tmp")
            .expect("a short socket directory");
        let socket = directory.path().join("turn.sock");
        let listener = UnixListener::bind(&socket).expect("a socket listener");
        listener.set_nonblocking(true).expect("nonblocking accept");
        let config_dir = directory.path().join(".smith");
        std::fs::create_dir(&config_dir).expect("a config directory");
        std::fs::write(config_dir.join("config.toml"), "invalid TOML [")
            .expect("configuration that must not be read");
        Self {
            directory,
            socket,
            listener,
            deadline,
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

    fn relay(&self) -> Relay {
        Relay::spawn(
            self.command().arg("mcp-relay").arg(&self.socket),
            self.deadline,
        )
    }

    fn accept(&self, deadline: Deadline) -> UnixStream {
        loop {
            deadline.remaining("socket connection");
            match self.listener.accept() {
                Ok((socket, _)) => {
                    // Accepted sockets inherit the listener's non-blocking flag on macOS and the BSDs.
                    socket.set_nonblocking(false).expect("blocking socket I/O");
                    let timeout = Some(deadline.remaining("socket I/O"));
                    socket.set_read_timeout(timeout).expect("read timeout");
                    socket.set_write_timeout(timeout).expect("write timeout");
                    return socket;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
    }
}

struct Relay {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<io::Result<Vec<u8>>>,
    stderr: Receiver<io::Result<Vec<u8>>>,
    pending_stdout: Vec<u8>,
    stdout_closed: bool,
    deadline: Deadline,
}

impl Relay {
    fn spawn(command: &mut Command, deadline: Deadline) -> Self {
        let mut child = command
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("smith spawned");
        let stdin = child.stdin.take();
        let stdout = chunks(child.stdout.take().expect("stdout"));
        let stderr = collect(child.stderr.take().expect("stderr"));
        Self {
            child,
            stdin,
            stdout,
            stderr,
            pending_stdout: Vec::new(),
            stdout_closed: false,
            deadline,
        }
    }

    fn write_stdin(&mut self, bytes: &[u8]) {
        let mut stdin = self.stdin.take().expect("open stdin");
        let bytes = bytes.to_vec();
        let (stdin, result) = self.deadline.work("writing stdin", move || {
            let result = stdin.write_all(&bytes);
            (stdin, result)
        });
        self.stdin = Some(stdin);
        result.expect("stdin bytes written");
    }

    fn next_stdout_chunk(&mut self) {
        let bytes = self
            .deadline
            .receive(&self.stdout, "reading stdout")
            .expect("stdout bytes");
        self.stdout_closed = bytes.is_empty();
        self.pending_stdout.extend_from_slice(&bytes);
    }

    fn read_stdout(&mut self, length: usize) -> Vec<u8> {
        while self.pending_stdout.len() < length {
            assert!(!self.stdout_closed, "stdout closed before the response");
            self.next_stdout_chunk();
        }
        let rest = self.pending_stdout.split_off(length);
        std::mem::replace(&mut self.pending_stdout, rest)
    }

    fn finish(mut self) -> Output {
        // Stdin stays open here so socket-first completion cannot accidentally
        // pass because the test itself closed the input pipe.
        let status = loop {
            self.deadline.remaining("process exit");
            if let Some(status) = self.child.try_wait().expect("process status") {
                break status;
            }
            thread::sleep(Duration::from_millis(10));
        };
        // Descendants can keep a relay's inherited pipes open after its exit.
        kill_group(self.child.id());
        // EOF readers need scheduling grace even if process exit used almost
        // all of the original deadline; an inherited pipe still cannot hang.
        let drain_deadline = Deadline::after(POST_EXIT_DRAIN_GRACE);
        while !self.stdout_closed {
            let bytes = drain_deadline
                .receive(&self.stdout, "relay stdout drain (10 s post-exit grace)")
                .expect("stdout bytes");
            self.stdout_closed = bytes.is_empty();
            self.pending_stdout.extend_from_slice(&bytes);
        }
        let stderr = drain_deadline
            .receive(&self.stderr, "relay stderr drain (10 s post-exit grace)")
            .expect("stderr bytes");
        Output {
            status,
            stdout: std::mem::take(&mut self.pending_stdout),
            stderr,
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        // A failed deadline must leave neither the relay nor its pipe readers
        // running after the test unwinds.
        kill_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn chunks(mut reader: impl Read + Send + 'static) -> Receiver<io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            let result = match reader.read(&mut buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result.map(|count| buffer[..count].to_vec()),
            };
            let finished = match &result {
                Ok(bytes) => bytes.is_empty(),
                Err(_) => true,
            };
            if sender.send(result).is_err() || finished {
                break;
            }
        }
    });
    receiver
}

fn collect(mut reader: impl Read + Send + 'static) -> Receiver<io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader.read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send(result);
    });
    receiver
}

fn payload() -> Vec<u8> {
    let mut bytes = b"first line\nsecond\0line\n".to_vec();
    bytes.extend((0_u8..=255).cycle().take(128 * 1024 + 17));
    bytes
}

fn socket_io<T>(result: io::Result<T>, step: &str) -> T {
    result.unwrap_or_else(|error| match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
            panic!("deadline exceeded during {step}: {error}")
        }
        _ => panic!("{step} failed: {error}"),
    })
}

fn write_socket(mut socket: UnixStream, bytes: &[u8], deadline: Deadline) -> UnixStream {
    let bytes = bytes.to_vec();
    let (socket, result) = deadline.work("writing socket", move || {
        let result = socket.write_all(&bytes);
        (socket, result)
    });
    socket_io(result, "writing socket");
    socket
}

fn assert_clean_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn stdin_bytes_reach_the_socket_unchanged() {
    let fixture = Fixture::new();
    let mut relay = fixture.relay();
    let socket = fixture.accept(relay.deadline);
    let expected = payload();
    // The reader runs concurrently because this payload exceeds pipe and
    // socket buffers on platforms with small default capacities.
    let received = collect(socket);
    relay.write_stdin(&expected);
    drop(relay.stdin.take());
    let received = socket_io(
        relay.deadline.receive(&received, "socket input and EOF"),
        "socket input and EOF",
    );
    assert_eq!(received, expected);
    let output = relay.finish();
    assert_clean_success(&output);
    assert!(output.stdout.is_empty());
}

#[test]
fn socket_bytes_reach_stdout_unchanged() {
    let fixture = Fixture::new();
    let relay = fixture.relay();
    let socket = fixture.accept(relay.deadline);
    let expected = payload();
    let socket = write_socket(socket, &expected, relay.deadline);
    drop(socket);
    let output = relay.finish();
    assert_clean_success(&output);
    assert_eq!(output.stdout, expected);
}

#[test]
fn request_response_is_interactive_with_stdin_open() {
    let fixture = Fixture::new();
    let mut relay = fixture.relay();
    let mut socket = fixture.accept(relay.deadline);
    let request = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n";
    let response = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n";
    relay.write_stdin(request);
    let (socket, received, result) = relay.deadline.work("request line", move || {
        let mut received = Vec::new();
        let result = BufReader::new(&mut socket).read_until(b'\n', &mut received);
        (socket, received, result)
    });
    socket_io(result, "request line");
    assert_eq!(received.as_slice(), request);
    let socket = write_socket(socket, response, relay.deadline);
    assert_eq!(relay.read_stdout(response.len()).as_slice(), response);
    assert!(relay.stdin.is_some());
    drop(socket);
    let output = relay.finish();
    assert_clean_success(&output);
    assert!(output.stdout.is_empty());
}

#[test]
fn socket_chunks_are_flushed_without_a_newline() {
    let fixture = Fixture::new();
    let mut relay = fixture.relay();
    let socket = fixture.accept(relay.deadline);
    let bytes = b"partial\0response";
    let socket = write_socket(socket, bytes, relay.deadline);
    assert_eq!(relay.read_stdout(bytes.len()).as_slice(), bytes);
    drop(socket);
    assert_clean_success(&relay.finish());
}

#[test]
fn stdin_eof_half_closes_the_socket_and_drains_the_response() {
    let fixture = Fixture::new();
    let mut relay = fixture.relay();
    let mut socket = fixture.accept(relay.deadline);
    drop(relay.stdin.take());
    let (socket, result) = relay.deadline.work("socket EOF", move || {
        let mut byte = [0_u8; 1];
        let result = socket.read(&mut byte);
        (socket, result)
    });
    assert_eq!(socket_io(result, "socket EOF"), 0);
    assert!(relay.child.try_wait().expect("process status").is_none());
    let response = b"response after stdin EOF\n";
    let socket = write_socket(socket, response, relay.deadline);
    assert_eq!(relay.read_stdout(response.len()).as_slice(), response);
    drop(socket);
    assert_clean_success(&relay.finish());
}

#[test]
fn socket_close_exits_while_stdin_is_still_open() {
    let fixture = Fixture::new();
    let relay = fixture.relay();
    let socket = fixture.accept(relay.deadline);
    assert!(relay.stdin.is_some());
    drop(socket);
    let output = relay.finish();
    assert_clean_success(&output);
    assert!(output.stdout.is_empty());
}

#[test]
fn socket_close_with_unread_input_exits_cleanly() {
    let fixture = Fixture::new();
    let mut relay = fixture.relay();
    let mut socket = fixture.accept(relay.deadline);
    relay.write_stdin(b"input the server will not finish reading\n");
    let (socket, byte, result) = relay.deadline.work("first input byte", move || {
        let mut byte = [0_u8; 1];
        let result = socket.read_exact(&mut byte);
        (socket, byte, result)
    });
    socket_io(result, "first input byte");
    assert_eq!(byte, [b'i']);
    assert!(relay.stdin.is_some());
    drop(socket);
    let output = relay.finish();
    assert_clean_success(&output);
    assert!(output.stdout.is_empty());
}

#[test]
fn a_missing_listener_reports_the_path_and_os_error_only_on_stderr() {
    let fixture = Fixture::new();
    let missing = fixture.directory.path().join("missing.sock");
    let os_error = UnixStream::connect(&missing)
        .expect_err("no listener")
        .to_string();
    let output = Relay::spawn(
        fixture.command().arg("mcp-relay").arg(&missing),
        fixture.deadline,
    )
    .finish();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr text");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.contains(&missing.display().to_string()), "{stderr}");
    assert!(stderr.contains(&os_error), "{stderr}");
}

#[test]
fn wrong_argument_counts_and_relative_paths_are_usage_errors() {
    let fixture = Fixture::new();
    for args in [
        vec!["mcp-relay"],
        vec!["mcp-relay", "/tmp/turn.sock", "extra"],
        vec!["mcp-relay", "relative.sock"],
    ] {
        let output = Relay::spawn(fixture.command().args(&args), fixture.deadline).finish();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).expect("usage text");
        assert!(
            stderr.contains("usage: smith mcp-relay <absolute-socket-path>"),
            "{stderr}"
        );
    }
}

#[test]
fn help_and_command_listings_do_not_discover_the_relay() {
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
            let args = path.iter().copied().chain([help]);
            let output = Relay::spawn(fixture.command().args(args), fixture.deadline).finish();
            assert_clean_success(&output);
            let text = String::from_utf8(output.stdout).expect("help text");
            assert!(text.contains("USAGE:"), "{text}");
            assert!(!text.contains("mcp-relay"), "{text}");
            assert!(!text.contains("wasm-engine-diagnostic"), "{text}");
        }
    }
}
