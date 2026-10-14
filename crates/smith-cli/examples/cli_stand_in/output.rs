//! Stream events are cloned from recordings, preserving their field layouts.

use std::io::{self, Write};

use anyhow::Result;
use serde_json::{Value, json};

use crate::arguments::Dialect;

macro_rules! fixture {
    ($dialect:literal, $file:literal) => {
        include_str!(concat!(
            "../../../smith-runtime/tests/fixtures/cli_bridge/",
            $dialect,
            "/",
            $file
        ))
    };
}

pub(crate) fn lines(input: &str) -> Vec<Value> {
    input
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("recorded JSON"))
        .collect()
}

pub(crate) fn stream(dialect: Dialect, kind: &str) -> Vec<Value> {
    let input = match (dialect, kind) {
        (Dialect::ClaudeCode, "call") => fixture!("claude-code-2.1.296", "stream/mcp-call.jsonl"),
        (Dialect::ClaudeCode, "own") => fixture!(
            "claude-code-2.1.296",
            "stream/mcp-call-beside-own-tools.jsonl"
        ),
        (Dialect::ClaudeCode, "denied") => fixture!(
            "claude-code-2.1.296",
            "stream/mcp-call-denied-without-allow-rule.jsonl"
        ),
        (Dialect::ClaudeCode, "timeout") => {
            fixture!("claude-code-2.1.296", "stream/mcp-call-timed-out.jsonl")
        }
        (Dialect::ClaudeCode, "fail") => {
            fixture!("claude-code-2.1.296", "stream/failed-turn.jsonl")
        }
        (Dialect::Codex, "call") => fixture!("codex-0.162.1", "stream/mcp-call.jsonl"),
        (Dialect::Codex, "own") => {
            fixture!("codex-0.162.1", "stream/mcp-call-beside-own-tool.jsonl")
        }
        (Dialect::Codex, "denied") => fixture!(
            "codex-0.162.1",
            "stream/mcp-call-refused-without-approval-mode.jsonl"
        ),
        (Dialect::Codex, "timeout") => fixture!("codex-0.162.1", "stream/mcp-call-timed-out.jsonl"),
        (Dialect::Codex, "fail") => fixture!("codex-0.162.1", "stream/failed-turn.jsonl"),
        _ => unreachable!("known fixture kind"),
    };
    lines(input)
}

pub(crate) fn handshake(dialect: Dialect) -> Vec<Value> {
    let input = match dialect {
        Dialect::ClaudeCode => fixture!("claude-code-2.1.296", "wire/handshake-and-call.jsonl"),
        Dialect::Codex => fixture!("codex-0.162.1", "wire/handshake-and-call.jsonl"),
    };
    lines(input)
        .into_iter()
        .filter(|line| line["dir"] == "in")
        .map(|line| line["msg"].clone())
        .collect()
}

pub(crate) fn cancellation(id: &Value) -> Value {
    let mut frame = lines(fixture!(
        "claude-code-2.1.296",
        "wire/call-cancelled-on-timeout.jsonl"
    ))
    .into_iter()
    .find(|line| line["msg"]["method"] == "notifications/cancelled")
    .expect("recorded cancellation")["msg"]
        .clone();
    frame["params"]["requestId"] = id.clone();
    frame
}

pub(crate) fn substitute(value: &mut Value, old: &str, new: &str) {
    match value {
        Value::String(text) => *text = text.replace(old, new),
        Value::Array(values) => {
            for value in values {
                substitute(value, old, new);
            }
        }
        Value::Object(values) => {
            let old_values = std::mem::take(values);
            for (key, mut value) in old_values {
                substitute(&mut value, old, new);
                values.insert(key.replace(old, new), value);
            }
        }
        _ => {}
    }
}

pub(crate) struct Output {
    pub(crate) dialect: Dialect,
    pub(crate) session: String,
    last_text: String,
}

impl Output {
    pub(crate) fn new(dialect: Dialect, session: String) -> Self {
        Self {
            dialect,
            session,
            last_text: String::new(),
        }
    }

    pub(crate) fn emit(&self, mut event: Value) -> Result<()> {
        if event.get("session_id").is_some() {
            event["session_id"] = json!(self.session);
        }
        if event.get("uuid").is_some() {
            event["uuid"] = json!(uuid::Uuid::new_v4().to_string());
        }
        let mut stdout = io::stdout().lock();
        serde_json::to_writer(&mut stdout, &event)?;
        writeln!(stdout)?;
        stdout.flush()?;
        Ok(())
    }

    pub(crate) fn session(&self, server: Option<&str>, cwd: &str, flags: &Value) -> Result<()> {
        let mut event = stream(self.dialect, "call").remove(0);
        match self.dialect {
            Dialect::ClaudeCode => {
                event["cwd"] = json!(cwd);
                if let Some(model) = flags.get("--model") {
                    event["model"] = model.clone();
                }
                if let Some(mode) = flags.get("--permission-mode") {
                    event["permissionMode"] = mode.clone();
                }
                event["mcp_servers"] = match server {
                    Some(name) => json!([{"name":name,"status":"connected","source":"dynamic"}]),
                    None => json!([]),
                };
                // The first line precedes connection, so no recorded spike tools
                // are claimed to be present before the actual list arrives.
                event["tools"] = json!(["Bash", "Edit", "Read", "ToolSearch", "Write"]);
            }
            Dialect::Codex => event["thread_id"] = json!(self.session),
        }
        self.emit(event)?;
        if self.dialect == Dialect::Codex {
            self.emit(
                stream(self.dialect, "call")
                    .into_iter()
                    .find(|v| v["type"] == "turn.started")
                    .expect("turn start"),
            )?;
        }
        Ok(())
    }

    pub(crate) fn say(&mut self, text: &str) -> Result<()> {
        self.last_text = text.to_owned();
        let mut event = stream(self.dialect, "call")
            .into_iter()
            .find(|v| match self.dialect {
                Dialect::ClaudeCode => {
                    v["type"] == "assistant" && v["message"]["content"][0]["type"] == "text"
                }
                Dialect::Codex => v["item"]["type"] == "agent_message",
            })
            .expect("assistant text");
        match self.dialect {
            Dialect::ClaudeCode => event["message"]["content"][0]["text"] = json!(text),
            Dialect::Codex => {
                event["item"]["text"] = json!(text);
                event["item"]["id"] = json!(uuid::Uuid::new_v4().to_string());
            }
        }
        self.emit(event)
    }

    pub(crate) fn terminal(&self, failure: Option<&str>) -> Result<()> {
        let kind = if failure.is_some() { "fail" } else { "call" };
        let mut event = stream(self.dialect, kind).pop().expect("terminal event");
        match self.dialect {
            Dialect::ClaudeCode => event["result"] = json!(failure.unwrap_or(&self.last_text)),
            Dialect::Codex => {
                if let Some(message) = failure {
                    let mut error = stream(self.dialect, "fail")
                        .into_iter()
                        .find(|v| v["type"] == "error")
                        .expect("failed error");
                    error["message"] = json!(message);
                    self.emit(error)?;
                    event["error"]["message"] = json!(message);
                }
            }
        }
        self.emit(event)
    }

    pub(crate) fn required_server_failed(&self, server: &str, timeout: i64) -> Result<()> {
        writeln!(
            io::stderr().lock(),
            "Error: thread/start: thread/start failed: error creating thread: Fatal error: Failed to initialize session: required MCP servers failed to initialize: {server}: timed out handshaking with MCP server after {timeout}s (code -32603)"
        )?;
        Ok(())
    }

    pub(crate) fn call_event(
        &self,
        kind: &str,
        completed: bool,
        server: &str,
        tool: &str,
        id: &str,
        args: &Value,
    ) -> Value {
        let events = stream(self.dialect, kind);
        let start = events
            .iter()
            .find(|v| match self.dialect {
                Dialect::ClaudeCode => v["message"]["content"][0]["name"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("mcp__")),
                Dialect::Codex => {
                    v["type"] == "item.started" && v["item"]["type"] == "mcp_tool_call"
                }
            })
            .expect("recorded call start");
        let old_id = match self.dialect {
            Dialect::ClaudeCode => start["message"]["content"][0]["id"].as_str(),
            Dialect::Codex => start["item"]["id"].as_str(),
        }
        .expect("call id");
        let mut event = if completed {
            events
                .iter()
                .find(|v| match self.dialect {
                    Dialect::ClaudeCode => {
                        v["type"] == "user" && v["message"]["content"][0]["tool_use_id"] == old_id
                    }
                    Dialect::Codex => v["type"] == "item.completed" && v["item"]["id"] == old_id,
                })
                .expect("recorded call result")
                .clone()
        } else {
            start.clone()
        };
        substitute(&mut event, old_id, id);
        let normalized: String = tool
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        match self.dialect {
            Dialect::ClaudeCode => {
                let old_name = start["message"]["content"][0]["name"]
                    .as_str()
                    .expect("name");
                substitute(
                    &mut event,
                    old_name,
                    &format!("mcp__{server}__{normalized}"),
                );
                substitute(&mut event, "\"spike\"", &format!("\"{server}\""));
                substitute(&mut event, "\"slow_wait\"", &format!("\"{tool}\""));
                if !completed {
                    event["message"]["content"][0]["input"] = args.clone();
                    if let Some(inputs) = event.get_mut("wire_tool_inputs") {
                        inputs[id] = args.clone();
                    }
                    if let Some(meta) = event.get_mut("tool_use_meta") {
                        meta[0]["server_display_name"] = json!(server);
                        meta[0]["display_name"] = json!(tool);
                    }
                }
            }
            Dialect::Codex => {
                substitute(&mut event, "spike/slow_wait", &format!("{server}/{tool}"));
                event["item"]["server"] = json!(server);
                event["item"]["tool"] = json!(tool);
                event["item"]["arguments"] = args.clone();
            }
        }
        event
    }

    pub(crate) fn call_start(
        &self,
        server: &str,
        tool: &str,
        id: &str,
        args: &Value,
    ) -> Result<()> {
        self.emit(self.call_event("call", false, server, tool, id, args))
    }

    pub(crate) fn call_result(
        &self,
        server: &str,
        tool: &str,
        id: &str,
        args: &Value,
        response: &Value,
    ) -> Result<()> {
        let failed = response.get("error").is_some() || response["result"]["isError"] == true;
        let mut event = self.call_event("call", true, server, tool, id, args);
        let content = response["result"]
            .get("content")
            .cloned()
            .unwrap_or_else(|| json!([{"type":"text","text":response["error"].to_string()}]));
        match self.dialect {
            Dialect::ClaudeCode => {
                event["message"]["content"][0]["content"] = content.clone();
                event["tool_use_result"] = content;
                if failed {
                    event["message"]["content"][0]["is_error"] = json!(true);
                }
            }
            Dialect::Codex => {
                if failed {
                    event["item"]["result"] = Value::Null;
                    event["item"]["error"] = json!({"message":content.to_string()});
                    event["item"]["status"] = json!("failed");
                } else {
                    event["item"]["result"]["content"] = content;
                    event["item"]["result"]["structured_content"] =
                        response["result"]["structuredContent"].clone();
                }
            }
        }
        self.emit(event)
    }

    pub(crate) fn refused(&self, server: &str, tool: &str, id: &str, args: &Value) -> Result<()> {
        self.call_start(server, tool, id, args)?;
        if self.dialect == Dialect::ClaudeCode {
            let mut event = stream(self.dialect, "denied")
                .into_iter()
                .find(|v| v["subtype"] == "permission_denied")
                .expect("permission denial");
            let old_id = event["tool_use_id"].as_str().expect("id").to_owned();
            let old_name = event["tool_name"].as_str().expect("name").to_owned();
            let name = self.call_event("call", false, server, tool, id, args)["message"]["content"]
                [0]["name"]
                .as_str()
                .expect("name")
                .to_owned();
            substitute(&mut event, &old_id, id);
            substitute(&mut event, &old_name, &name);
            self.emit(event)?;
        }
        self.emit(self.call_event("denied", true, server, tool, id, args))
    }

    pub(crate) fn own_tool(&self, label: &str) -> Result<()> {
        self.own_tool_output(label, None)
    }

    pub(crate) fn own_read(&self, path: &std::path::Path, contents: &str) -> Result<()> {
        // Replay the recorded shell-tool shape: the stand-in actually reads
        // the file before reporting the equivalent `cat` command and output.
        self.own_tool_output(&format!("cat {}", path.display()), Some(contents))
    }

    fn own_tool_output(&self, label: &str, contents: Option<&str>) -> Result<()> {
        let events = stream(self.dialect, "own");
        let start = events
            .iter()
            .find(|v| match self.dialect {
                Dialect::ClaudeCode => v["message"]["content"][0]["name"] == "Bash",
                Dialect::Codex => {
                    v["type"] == "item.started" && v["item"]["type"] == "command_execution"
                }
            })
            .expect("own tool start");
        let old_id = match self.dialect {
            Dialect::ClaudeCode => start["message"]["content"][0]["id"].as_str(),
            Dialect::Codex => start["item"]["id"].as_str(),
        }
        .expect("own tool id");
        let result = events
            .iter()
            .find(|v| match self.dialect {
                Dialect::ClaudeCode => {
                    v["type"] == "user" && v["message"]["content"][0]["tool_use_id"] == old_id
                }
                Dialect::Codex => v["type"] == "item.completed" && v["item"]["id"] == old_id,
            })
            .expect("own tool result");
        let id = uuid::Uuid::new_v4().to_string();
        for mut event in [start.clone(), result.clone()] {
            substitute(&mut event, old_id, &id);
            match self.dialect {
                Dialect::ClaudeCode => {
                    if event["type"] == "assistant" {
                        if contents.is_some() {
                            event["message"]["content"][0]["input"]["command"] = json!(label);
                            event["wire_tool_inputs"][&id]["command"] = json!(label);
                        }
                        event["message"]["content"][0]["input"]["description"] = json!(label);
                        event["wire_tool_inputs"][&id]["description"] = json!(label);
                    } else if let Some(contents) = contents {
                        event["message"]["content"][0]["content"] = json!(contents);
                    }
                }
                Dialect::Codex => {
                    event["item"]["command"] = json!(label);
                    if event["type"] == "item.completed"
                        && let Some(contents) = contents
                    {
                        event["item"]["aggregated_output"] = json!(contents);
                    }
                }
            }
            self.emit(event)?;
        }
        Ok(())
    }
}
