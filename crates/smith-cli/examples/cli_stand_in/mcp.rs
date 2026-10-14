//! Blocking stdio keeps the stand-in independent of Smith's async transport.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::Record;
use crate::arguments::{Dialect, Server};
use crate::output;

pub(crate) struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<std::io::Result<String>>,
    pending: Vec<Value>,
    next_id: u64,
    call_template: Value,
}

impl Client {
    pub(crate) fn connect(server: &Server, dialect: Dialect, record: &mut Record) -> Result<Self> {
        let mut child = Command::new(&server.command)
            .args(&server.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                let result = reader.read_line(&mut line).map(|_| line);
                let done = result.as_ref().map_or(true, String::is_empty);
                if sender.send(result).is_err() || done {
                    break;
                }
            }
        });
        let handshake = output::handshake(dialect);
        let mut client = Self {
            child,
            stdin,
            stdout: receiver,
            pending: Vec::new(),
            next_id: 2,
            call_template: handshake
                .iter()
                .find(|v| v["method"] == "tools/call")
                .expect("call frame")
                .clone(),
        };
        record.value["server_spawned"] = json!(true);
        record.save()?;
        let mut discovered = false;
        for frame in handshake
            .into_iter()
            .filter(|v| v["method"] != "tools/call")
        {
            if discovered && frame["method"] == "initialize" {
                continue;
            }
            client.send(&frame, record)?;
            if let Some(id) = frame.get("id") {
                let response = client.response(id, record)?;
                match frame["method"].as_str() {
                    Some("server/discover") => {
                        discovered = response.get("result").is_some();
                        record.value["handshake_path"] =
                            json!(if discovered { "discover" } else { "initialize" });
                        if discovered {
                            record.value["instructions"] =
                                response["result"]["instructions"].clone();
                        }
                    }
                    Some("initialize") => {
                        if response.get("error").is_some() {
                            bail!("initialize refused: {response}");
                        }
                        record.value["instructions"] = response["result"]["instructions"].clone();
                        if dialect == Dialect::Codex {
                            record.value["handshake_path"] = json!("initialize");
                        }
                    }
                    Some("tools/list") => {
                        if response.get("error").is_some() {
                            bail!("tools/list refused: {response}");
                        }
                        record.value["tools"] = response["result"]["tools"].clone();
                        record.value["tool_names"] = json!(
                            response["result"]["tools"]
                                .as_array()
                                .ok_or_else(|| anyhow::anyhow!(
                                    "tools/list returned no tools array"
                                ))?
                                .iter()
                                .map(|tool| tool["name"].clone())
                                .collect::<Vec<_>>()
                        );
                    }
                    _ => {}
                }
                record.save()?;
            }
        }
        Ok(client)
    }

    pub(crate) fn send(&mut self, frame: &Value, record: &mut Record) -> Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("MCP stdin is closed"))?;
        serde_json::to_writer(&mut *stdin, frame)?;
        writeln!(stdin)?;
        stdin.flush()?;
        record.wire("sent", frame.clone())
    }

    pub(crate) fn response(&mut self, id: &Value, record: &mut Record) -> Result<Value> {
        let timeout = std::env::var("CLI_STAND_IN_WAIT_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(15_000);
        self.response_timeout(id, record, Duration::from_millis(timeout))?
            .ok_or_else(|| anyhow::anyhow!("MCP response deadline exceeded: {id}"))
    }

    pub(crate) fn response_timeout(
        &mut self,
        id: &Value,
        record: &mut Record,
        timeout: Duration,
    ) -> Result<Option<Value>> {
        let deadline = Instant::now() + timeout;
        if let Some(index) = self.pending.iter().position(|v| v.get("id") == Some(id)) {
            return Ok(Some(self.pending.remove(index)));
        }
        loop {
            // The reader only queues bytes; the script still sends every
            // call_together request before consuming or recording a response.
            let line = match self
                .stdout
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(line) => line?,
                Err(mpsc::RecvTimeoutError::Timeout) => return Ok(None),
                Err(error) => bail!("MCP response reader closed: {error}"),
            };
            if line.is_empty() {
                bail!("MCP stdout closed while waiting for response {id}");
            }
            let frame: Value = serde_json::from_str(&line)?;
            record.wire("received", frame.clone())?;
            if frame.get("id") == Some(id) {
                return Ok(Some(frame));
            }
            if frame.get("id").is_some() {
                self.pending.push(frame);
            }
        }
    }

    pub(crate) fn call(
        &mut self,
        dialect: Dialect,
        tool: &str,
        args: &Value,
        echo_id: &str,
        record: &mut Record,
    ) -> Result<Value> {
        let request_id = self.next_id;
        let id = json!(request_id);
        self.next_id += 1;
        let mut frame = self.call_template.clone();
        frame["id"] = id.clone();
        frame["params"]["name"] = json!(tool);
        frame["params"]["arguments"] = args.clone();
        let meta = &mut frame["params"]["_meta"];
        match dialect {
            Dialect::ClaudeCode => {
                meta["progressToken"] = id.clone();
                meta["claudecode/toolUseId"] = json!(echo_id);
            }
            Dialect::Codex => {
                meta["progressToken"] = json!(request_id - 1);
                meta["callId"] = json!(format!("exec-{}", uuid::Uuid::new_v4()));
            }
        }
        self.send(&frame, record)?;
        Ok(id)
    }

    pub(crate) fn close(&mut self, outstanding: &Value, record: &mut Record) -> Result<()> {
        drop(self.stdin.take());
        record.wire(
            "stdin_closed",
            json!({"outstanding_request_id":outstanding}),
        )
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // A scripted exit cannot leave a slow server holding the CLI's pipes.
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
