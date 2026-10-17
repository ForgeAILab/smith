//! What a stdio MCP server writes outside the protocol.
//!
//! A server's stderr is a third party writing to whatever descriptor it was
//! given. Inherited, it lands on Smith's terminal and paints over the TUI, so
//! Smith pipes it instead and drains it here: continuously, so a chatty server
//! never blocks on a full pipe, and into a bounded tail, so a flood costs a
//! fixed amount of memory. Lines go to the session log, and the tail is what
//! a failure report quotes. Values Smith passed to the server through its
//! environment are redacted before a line is stored or logged.

use std::collections::VecDeque;
use std::process::ExitStatus;
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{oneshot, watch};

/// Lines kept per server.
const TAIL_LINES: usize = 64;
/// Bytes kept per server, and the longest partial line held before a break.
const TAIL_BYTES: usize = 16 * 1024;
/// The longest stored line, in characters.
const LINE_CHARS: usize = 512;
/// Shorter environment values are too likely to be ordinary words to redact.
const REDACT_MIN_CHARS: usize = 6;
const REDACTED: &str = "[redacted]";
/// How long the drain keeps reading after Smith stops the process.
const STOP_GRACE: Duration = Duration::from_millis(500);
/// How long a report waits for a gone server's last lines and exit status.
const SETTLE: Duration = Duration::from_secs(1);

/// How the process's life ended, as far as Smith knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    /// Smith stopped it: closing, reconfiguring, or exiting.
    Stopped,
    /// Its output closed while Smith still wanted it.
    Lost,
}

#[derive(Debug, Clone)]
struct Progress {
    phase: Phase,
    drained: bool,
    status: Option<ExitStatus>,
}

#[derive(Default)]
struct Tail {
    lines: VecDeque<String>,
    bytes: usize,
}

/// The bounded, redacted stderr of one stdio MCP server process.
pub struct McpStderr {
    server: String,
    tail: Mutex<Tail>,
    progress: watch::Sender<Progress>,
}

impl std::fmt::Debug for McpStderr {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpStderr")
            .field("server", &self.server)
            .field("lines", &self.tail.lock().expect("MCP stderr").lines.len())
            .finish()
    }
}

impl McpStderr {
    pub(super) fn new(server: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            tail: Mutex::default(),
            progress: watch::Sender::new(Progress {
                phase: Phase::Running,
                drained: false,
                status: None,
            }),
        }
    }

    /// Every retained line, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.last_lines(TAIL_LINES)
    }

    /// The last `count` retained lines, oldest first.
    pub fn last_lines(&self, count: usize) -> Vec<String> {
        let tail = self.tail.lock().expect("MCP stderr");
        tail.lines
            .iter()
            .skip(tail.lines.len().saturating_sub(count))
            .cloned()
            .collect()
    }

    /// Waits, at most a bounded time, for the drain to read its last line.
    pub async fn drained(&self) {
        let mut progress = self.progress.subscribe();
        let _ = tokio::time::timeout(SETTLE, progress.wait_for(|progress| progress.drained)).await;
    }

    /// Resolves once the process is gone: with why, when it went on its own,
    /// or with nothing when Smith stopped it.
    pub async fn lost(&self) -> Option<String> {
        let mut progress = self.progress.subscribe();
        let phase = progress
            .wait_for(|progress| progress.phase != Phase::Running)
            .await
            .map_or(Phase::Stopped, |progress| progress.phase);
        if phase != Phase::Lost {
            return None;
        }
        // The last lines and the exit status arrive just after the output
        // closes; waiting briefly lets the report carry both.
        let _ = tokio::time::timeout(
            SETTLE,
            progress.wait_for(|progress| progress.drained && progress.status.is_some()),
        )
        .await;
        let status = self.progress.borrow().status;
        Some(match status {
            Some(status) => format!("exited unexpectedly ({status})"),
            None => "closed its output unexpectedly".to_owned(),
        })
    }

    /// Smith is stopping the process; its end is not a failure.
    pub(super) fn mark_stopped(&self) {
        self.advance(Phase::Stopped);
    }

    /// The process's output closed while Smith still wanted it. Returns
    /// whether this is the first word of its end.
    pub(super) fn mark_lost(&self) -> bool {
        self.advance(Phase::Lost)
    }

    pub(super) fn record_status(&self, status: ExitStatus) {
        self.progress
            .send_modify(|progress| progress.status = Some(status));
    }

    fn advance(&self, phase: Phase) -> bool {
        self.progress.send_if_modified(|progress| {
            let running = progress.phase == Phase::Running;
            if running {
                progress.phase = phase;
            }
            running
        })
    }

    fn push(&self, line: String) {
        tracing::info!(server = %self.server, "MCP server stderr: {line}");
        let mut tail = self.tail.lock().expect("MCP stderr");
        while !tail.lines.is_empty()
            && (tail.lines.len() >= TAIL_LINES || tail.bytes + line.len() > TAIL_BYTES)
        {
            if let Some(dropped) = tail.lines.pop_front() {
                tail.bytes -= dropped.len();
            }
        }
        tail.bytes += line.len();
        tail.lines.push_back(line);
    }
}

/// Reads `stderr` into `output` until it closes, or until shortly after
/// `stop` fires (or its sender is dropped).
///
/// The grace lets a stopped server's last words arrive without letting an
/// escaped descendant that still holds the pipe keep the reader alive.
pub(super) async fn drain(
    mut stderr: impl AsyncRead + Unpin,
    output: std::sync::Arc<McpStderr>,
    secrets: Vec<String>,
    mut stop: oneshot::Receiver<()>,
) {
    let secrets: Vec<String> = secrets
        .into_iter()
        .filter(|secret| secret.chars().count() >= REDACT_MIN_CHARS)
        .collect();
    let mut buffer = [0; 4096];
    let mut pending = Vec::new();
    let mut deadline = None;
    loop {
        let read = match deadline {
            None => tokio::select! {
                biased;
                _ = &mut stop => {
                    deadline = Some(tokio::time::Instant::now() + STOP_GRACE);
                    continue;
                }
                read = stderr.read(&mut buffer) => read,
            },
            Some(deadline) => {
                match tokio::time::timeout_at(deadline, stderr.read(&mut buffer)).await {
                    Ok(read) => read,
                    Err(_) => break,
                }
            }
        };
        let Ok(count) = read else { break };
        if count == 0 {
            break;
        }
        for &byte in &buffer[..count] {
            if byte == b'\n' {
                emit(&output, &pending, &secrets);
                pending.clear();
            } else {
                pending.push(byte);
                if pending.len() >= TAIL_BYTES {
                    emit(&output, &pending, &secrets);
                    pending.clear();
                }
            }
        }
    }
    emit(&output, &pending, &secrets);
    output
        .progress
        .send_modify(|progress| progress.drained = true);
}

fn emit(output: &McpStderr, bytes: &[u8], secrets: &[String]) {
    if let Some(line) = clean_line(&String::from_utf8_lossy(bytes), secrets) {
        output.push(line);
    }
}

/// What a terminal would leave visible of `raw`, redacted and bounded.
fn clean_line(raw: &str, secrets: &[String]) -> Option<String> {
    let raw = raw.strip_suffix('\r').unwrap_or(raw);
    // A carriage return redraws the line: progress spinners leave only the
    // last frame on a terminal, so only that frame is kept here.
    let raw = raw.rsplit('\r').next().unwrap_or(raw);
    let mut line = strip_escapes(raw);
    for secret in secrets {
        if line.contains(secret.as_str()) {
            line = line.replace(secret.as_str(), REDACTED);
        }
    }
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    Some(if line.chars().count() > LINE_CHARS {
        let mut bounded: String = line.chars().take(LINE_CHARS - 1).collect();
        bounded.push('…');
        bounded
    } else {
        line.to_owned()
    })
}

/// Removes terminal escape sequences and other control characters.
fn strip_escapes(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.next() {
                // CSI: parameters and intermediates up to a final byte.
                Some('[') => {
                    for ch in chars.by_ref() {
                        if ('@'..='~').contains(&ch) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or the string terminator.
                Some(']') => {
                    while let Some(ch) = chars.next() {
                        if ch == '\u{7}' || (ch == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\t' => text.push(' '),
            ch if ch.is_control() => {}
            ch => text.push(ch),
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    async fn drained(input: &[u8], secrets: &[&str]) -> Arc<McpStderr> {
        let output = Arc::new(McpStderr::new("test"));
        let (_stop, stopped) = oneshot::channel();
        drain(
            input,
            Arc::clone(&output),
            secrets.iter().map(|secret| (*secret).to_owned()).collect(),
            stopped,
        )
        .await;
        output
    }

    #[tokio::test]
    async fn lines_are_cleaned_redacted_and_kept_in_order() {
        let output = drained(
            b"\x1b[33mnpm\x1b[0m notice New version\r\n\
              spinner \\\rspinner |\rinstalled\n\
              \x1b]0;title\x07token=sk-live-123456 rejected\n\
              \n   \t\nshort=abc\nlast without newline",
            &["sk-live-123456", "abc"],
        )
        .await;
        assert_eq!(
            output.lines(),
            [
                "npm notice New version",
                "installed",
                "token=[redacted] rejected",
                "short=abc",
                "last without newline",
            ]
        );
        assert_eq!(output.last_lines(2), ["short=abc", "last without newline"]);
    }

    #[tokio::test]
    async fn the_tail_is_bounded_by_lines_bytes_and_line_length() {
        let many: String = (0..200).map(|line| format!("line {line}\n")).collect();
        let output = drained(many.as_bytes(), &[]).await;
        let lines = output.lines();
        assert_eq!(lines.len(), TAIL_LINES);
        assert_eq!(lines.last().unwrap(), "line 199");

        let long = "x".repeat(TAIL_BYTES * 3);
        let output = drained(long.as_bytes(), &[]).await;
        let lines = output.lines();
        assert!(lines.iter().all(|line| line.chars().count() <= LINE_CHARS));
        let tail = output.tail.lock().unwrap();
        assert!(tail.bytes <= TAIL_BYTES, "{}", tail.bytes);
    }

    #[tokio::test]
    async fn a_stopped_drain_ends_within_its_grace_even_while_the_pipe_stays_open() {
        let (reader, mut writer) = tokio::io::duplex(64);
        let output = Arc::new(McpStderr::new("test"));
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(drain(reader, Arc::clone(&output), Vec::new(), stopped));
        tokio::io::AsyncWriteExt::write_all(&mut writer, b"before stop\n")
            .await
            .unwrap();
        drop(stop);
        tokio::time::timeout(STOP_GRACE * 4, task)
            .await
            .expect("the drain must not wait for EOF")
            .unwrap();
        assert_eq!(output.lines(), ["before stop"]);
        drop(writer);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn only_a_lost_process_is_reported() {
        use std::os::unix::process::ExitStatusExt;

        let stopped = McpStderr::new("test");
        stopped.mark_stopped();
        stopped.mark_lost();
        assert_eq!(stopped.lost().await, None);

        let lost = McpStderr::new("test");
        lost.mark_lost();
        lost.mark_stopped();
        lost.progress
            .send_modify(|progress| progress.drained = true);
        lost.record_status(ExitStatus::from_raw(3 << 8));
        assert_eq!(
            lost.lost().await.as_deref(),
            Some("exited unexpectedly (exit status: 3)")
        );
    }
}
