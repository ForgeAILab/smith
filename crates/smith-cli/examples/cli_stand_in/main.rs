//! A fixture-backed CLI and counting MCP server for process-boundary contracts.

mod approvals;
mod arguments;
mod counting;
mod mcp;
mod native;
mod output;

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};

use arguments::{Dialect, Invocation};
use mcp::Client;
use output::Output;

fn wait_file(path: &std::path::Path) -> Result<()> {
    let timeout = std::env::var("CLI_STAND_IN_WAIT_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(15_000);
    let deadline = Instant::now() + Duration::from_millis(timeout);
    while !path.is_file() {
        anyhow::ensure!(
            Instant::now() < deadline,
            "fixture wait timed out: {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

pub(crate) struct Record {
    path: Option<PathBuf>,
    value: Value,
}

impl Record {
    fn new(argv: &[String]) -> Result<Self> {
        let env: serde_json::Map<String, Value> = std::env::vars()
            .map(|(name, value)| (name, json!(value)))
            .collect();
        let record = Self {
            path: std::env::var_os("CLI_STAND_IN_RECORD").map(PathBuf::from),
            value: json!({"argv":argv,"dialect":null,"cwd":std::env::current_dir()?,
                "environment":env,"server":null,"prompt":null,"resume":null,
                "server_spawned":false,"wire":[],"steps":[],
                "pid":std::process::id(),
                "started":SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos().to_string()}),
        };
        record.save()?;
        Ok(record)
    }

    fn save(&self) -> Result<()> {
        if let Some(path) = &self.path {
            // A reader polling a running script sees a complete snapshot.
            let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
            fs::write(&temporary, serde_json::to_vec_pretty(&self.value)?)?;
            fs::rename(temporary, path)?;
            let archive = path.with_extension(format!("{}.invocation.json", std::process::id()));
            let temporary = archive.with_extension("tmp");
            fs::write(&temporary, serde_json::to_vec_pretty(&self.value)?)?;
            fs::rename(temporary, archive)?;
        }
        Ok(())
    }

    fn wire(&mut self, direction: &str, frame: Value) -> Result<()> {
        let events = self.value["wire"].as_array_mut().expect("wire array");
        events.push(json!({"sequence":events.len(),"direction":direction,"frame":frame}));
        self.save()
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    tool: String,
    arguments: Value,
}

#[derive(Deserialize)]
struct Abandon {
    tool: String,
    arguments: Value,
    after_ms: u64,
    how: String,
    #[serde(default)]
    started_file: Option<PathBuf>,
}

#[derive(Deserialize)]
struct CallExit {
    tool: String,
    arguments: Value,
    code: i32,
    #[serde(default)]
    started_file: Option<PathBuf>,
}

#[derive(Deserialize)]
struct SpawnChild {
    seconds: u64,
    pid_file: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    Say(String),
    SayLast(bool),
    ServerNeverStarts(bool),
    ListTools(bool),
    Call(Call),
    CallTogether(Vec<Call>),
    TogetherCancel(TogetherCancel),
    Abandon(Abandon),
    OwnTool { name: String },
    OwnRead(PathBuf),
    SpawnChild(SpawnChild),
    SleepMs(u64),
    WaitFile(PathBuf),
    Exit { code: i32 },
    CallThenExit(CallExit),
    Fail(String),
}

#[derive(Deserialize)]
struct TogetherCancel {
    calls: Vec<Call>,
    cancel_index: usize,
    after_ms: u64,
}

struct Runner {
    invocation: Invocation,
    output: Output,
    client: Option<Client>,
    server_unavailable: bool,
    session_started: bool,
    children: Vec<Child>,
}

impl Runner {
    fn session(&mut self, record: &Record) -> Result<()> {
        if !self.session_started {
            let server = self
                .invocation
                .server
                .as_ref()
                .filter(|_| !self.server_unavailable)
                .map(|server| server.name.as_str());
            self.output.session(
                server,
                record.value["cwd"].as_str().expect("cwd"),
                &self.invocation.flags,
            )?;
            self.session_started = true;
        }
        Ok(())
    }

    fn client(&mut self, record: &mut Record) -> Result<&mut Client> {
        anyhow::ensure!(!self.server_unavailable, "MCP server is unavailable");
        if self.client.is_none() {
            let server = self
                .invocation
                .server
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("script needs an MCP server declaration"))?;
            self.client = Some(Client::connect(server, self.output.dialect, record)?);
        }
        Ok(self.client.as_mut().expect("connected client"))
    }

    fn start_call(&mut self, call: &Call, record: &mut Record) -> Result<Option<(Value, String)>> {
        if self.server_unavailable {
            self.output.say("UNAVAILABLE")?;
            return Ok(None);
        }
        let server = self
            .invocation
            .server
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("call needs an MCP server declaration"))?
            .name
            .clone();
        let dialect = self.output.dialect;
        let echo_id = format!("toolu_{}", uuid::Uuid::new_v4().simple());
        // Both recorded CLIs load the server before applying their call gate.
        self.client(record)?;
        if !self.invocation.allowed(dialect) {
            self.output
                .refused(&server, &call.tool, &echo_id, &call.arguments)?;
            record.wire(
                "refused",
                json!({"tool":call.tool,"arguments":call.arguments}),
            )?;
            return Ok(None);
        }
        self.output
            .call_start(&server, &call.tool, &echo_id, &call.arguments)?;
        let request_id =
            self.client(record)?
                .call(dialect, &call.tool, &call.arguments, &echo_id, record)?;
        Ok(Some((request_id, echo_id)))
    }

    fn finish_call(
        &mut self,
        call: &Call,
        id: &Value,
        echo_id: &str,
        record: &mut Record,
    ) -> Result<()> {
        let default_ms = std::env::var("CLI_STAND_IN_DEFAULT_TOOL_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(2_000);
        let configured_ms = match self.output.dialect {
            Dialect::ClaudeCode => std::env::var("MCP_TOOL_TIMEOUT")
                .ok()
                .and_then(|value| value.parse::<u64>().ok()),
            Dialect::Codex => self
                .invocation
                .server
                .as_ref()
                .and_then(|server| server.tool_timeout_sec)
                .and_then(|seconds| u64::try_from(seconds).ok())
                .map(|seconds| seconds.saturating_mul(1_000)),
        };
        let ignore = std::env::var("CLI_STAND_IN_IGNORE_TOOL_TIMEOUT").as_deref() == Ok("true");
        let timeout_ms = if ignore {
            default_ms
        } else {
            configured_ms.unwrap_or(default_ms)
        };
        record.value["tool_timeout_ms"] = json!(timeout_ms);
        record.save()?;
        let response =
            self.client(record)?
                .response_timeout(id, record, Duration::from_millis(timeout_ms))?;
        let Some(response) = response else {
            record.value["tool_timed_out"] = json!(true);
            record.save()?;
            if self.output.dialect == Dialect::ClaudeCode {
                self.client(record)?
                    .send(&output::cancellation(id), record)?;
            }
            let server = &self.invocation.server.as_ref().expect("server").name;
            self.output.emit(self.output.call_event(
                "timeout",
                true,
                server,
                &call.tool,
                echo_id,
                &call.arguments,
            ))?;
            return Ok(());
        };
        record.value["last_result"] = decode_result(&response);
        record.save()?;
        let server = &self
            .invocation
            .server
            .as_ref()
            .expect("declared server")
            .name;
        self.output
            .call_result(server, &call.tool, echo_id, &call.arguments, &response)
    }

    fn step(&mut self, step: Step, record: &mut Record) -> Result<Option<(i32, &'static str)>> {
        // A required-server startup failure happens before Codex emits stdout.
        if !matches!(step, Step::ServerNeverStarts(_)) {
            self.session(record)?;
        }
        match step {
            Step::Say(text) => self.output.say(&text)?,
            Step::SayLast(enabled) => {
                anyhow::ensure!(enabled, "say_last must be true");
                self.output.say(&record.value["last_result"].to_string())?;
            }
            Step::ServerNeverStarts(enabled) => {
                anyhow::ensure!(enabled, "server_never_starts must be true");
                anyhow::ensure!(self.client.is_none(), "server already started");
                let server =
                    self.invocation.server.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("startup needs an MCP server declaration")
                    })?;
                self.server_unavailable = true;
                record.value["tools"] = json!([]);
                record.value["tool_names"] = json!([]);
                if self.output.dialect == Dialect::Codex && server.required == Some(true) {
                    // Smith's startup budget also covers scripts omitting it.
                    self.output.required_server_failed(
                        &server.name,
                        server.startup_timeout_sec.unwrap_or(60),
                    )?;
                    return Ok(Some((1, "required_server_failed")));
                }
                self.session(record)?;
            }
            Step::ListTools(enabled) => {
                anyhow::ensure!(enabled, "list_tools must be true");
                if !self.server_unavailable && !record.value["tools"].is_array() {
                    self.client(record)?;
                }
            }
            Step::Call(call) => {
                if let Some((id, echo_id)) = self.start_call(&call, record)? {
                    self.finish_call(&call, &id, &echo_id, record)?;
                }
            }
            Step::CallTogether(calls) => {
                let mut pending = Vec::new();
                for call in calls {
                    if let Some((id, echo_id)) = self.start_call(&call, record)? {
                        pending.push((call, id, echo_id));
                    }
                }
                for (call, id, echo_id) in pending {
                    self.finish_call(&call, &id, &echo_id, record)?;
                }
            }
            Step::TogetherCancel(together) => {
                let mut pending = Vec::new();
                for call in together.calls {
                    if let Some((id, echo_id)) = self.start_call(&call, record)? {
                        pending.push((call, id, echo_id));
                    }
                }
                let (_, id, _) = pending
                    .get(together.cancel_index)
                    .ok_or_else(|| anyhow::anyhow!("cancel_index outside calls"))?;
                thread::sleep(Duration::from_millis(together.after_ms));
                self.client(record)?
                    .send(&output::cancellation(id), record)?;
                for (index, (call, id, echo_id)) in pending.into_iter().enumerate() {
                    if index != together.cancel_index {
                        self.finish_call(&call, &id, &echo_id, record)?;
                    }
                }
            }
            Step::Abandon(abandon) => {
                anyhow::ensure!(
                    matches!(abandon.how.as_str(), "cancel" | "close"),
                    "abandon.how must be cancel or close"
                );
                let call = Call {
                    tool: abandon.tool,
                    arguments: abandon.arguments,
                };
                if let Some((id, echo_id)) = self.start_call(&call, record)? {
                    if let Some(path) = abandon.started_file {
                        wait_file(&path)?;
                    }
                    thread::sleep(Duration::from_millis(abandon.after_ms));
                    if abandon.how == "cancel" {
                        self.client(record)?
                            .send(&output::cancellation(&id), record)?;
                    } else {
                        self.client(record)?.close(&id, record)?;
                        drop(self.client.take());
                    }
                    let server = &self.invocation.server.as_ref().expect("server").name;
                    self.output.emit(self.output.call_event(
                        "timeout",
                        true,
                        server,
                        &call.tool,
                        &echo_id,
                        &call.arguments,
                    ))?;
                }
            }
            Step::OwnTool { name } => self.output.own_tool(&name)?,
            Step::OwnRead(path) => {
                let contents = fs::read_to_string(&path)?;
                self.output.own_read(&path, &contents)?;
            }
            Step::SpawnChild(spawn) => {
                let child = Command::new("/bin/sleep")
                    .arg(spawn.seconds.to_string())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()?;
                let child_pid = child.id();
                self.children.push(child);
                let temporary = spawn.pid_file.with_extension("tmp");
                fs::write(
                    &temporary,
                    serde_json::to_vec(
                        &json!({"stand_in_pid":std::process::id(),"child_pid":child_pid}),
                    )?,
                )?;
                fs::rename(temporary, spawn.pid_file)?;
            }
            Step::WaitFile(path) => {
                wait_file(&path)?;
            }
            Step::SleepMs(ms) => thread::sleep(Duration::from_millis(ms)),
            Step::Exit { code } => return Ok(Some((code, "exited"))),
            Step::CallThenExit(exit) => {
                self.start_call(
                    &Call {
                        tool: exit.tool,
                        arguments: exit.arguments,
                    },
                    record,
                )?;
                if let Some(path) = exit.started_file {
                    wait_file(&path)?;
                }
                return Ok(Some((exit.code, "exited_with_call_outstanding")));
            }
            Step::Fail(message) => {
                self.output.terminal(Some(&message))?;
                return Ok(Some((1, "failed_turn")));
            }
        }
        Ok(None)
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        // Normal script completion also cleans children; interruption tests
        // kill the whole group while the script is still sleeping.
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn decode_result(response: &Value) -> Value {
    let text = response["result"]["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

/// A JSON pointer into the preceding tool result avoids guessing runtime IDs.
fn resolve_last(value: &mut Value, last: &Value) -> Result<()> {
    if let Some(pointer) = value.get("$last").and_then(Value::as_str) {
        *value = last
            .pointer(pointer)
            .ok_or_else(|| anyhow::anyhow!("last result has no pointer {pointer}: {last}"))?
            .clone();
    } else {
        match value {
            Value::Array(values) => {
                for value in values {
                    resolve_last(value, last)?;
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    resolve_last(value, last)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn run(args: &[String], record: &mut Record) -> Result<i32> {
    let dialect = Dialect::detect(args)?;
    record.value["dialect"] = json!(dialect);
    record.save()?;
    let invocation = Invocation::parse(args, dialect)?;
    record.value["server"] = json!(invocation.server);
    record.value["prompt"] = json!(invocation.prompt);
    record.value["resume"] = json!(invocation.resume);
    record.value["flags"] = invocation.flags.clone();
    let session = invocation
        .resume
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    record.value["session_id"] = json!(session);
    record.value["session_source"] = json!(if invocation.resume.is_some() {
        "resume"
    } else {
        "fresh"
    });
    record.save()?;
    let path = std::env::var_os("CLI_STAND_IN_SCRIPT")
        .ok_or_else(|| anyhow::anyhow!("CLI_STAND_IN_SCRIPT is required"))?;
    let scripts: Value = serde_json::from_slice(&fs::read(path)?)?;
    // An array keeps the original grammar. An object selects a script for a
    // task (harness child), a fresh root turn, or a resumed/internal turn.
    let selected = if scripts.is_array() {
        &scripts
    } else if let Some(task) = scripts["by_prompt"].get(&invocation.prompt) {
        task
    } else if invocation.resume.is_some() {
        &scripts["resumed"]
    } else {
        &scripts["initial"]
    };
    let script: Vec<Value> = serde_json::from_value(selected.clone())?;
    let output = Output::new(dialect, session);
    let mut runner = Runner {
        invocation,
        output,
        client: None,
        server_unavailable: false,
        session_started: false,
        children: Vec::new(),
    };
    for (index, mut value) in script.into_iter().enumerate() {
        resolve_last(&mut value, &record.value["last_result"])?;
        record.value["current_step"] = json!(index);
        record.value["steps"]
            .as_array_mut()
            .expect("steps")
            .push(json!({"index":index,"step":value,"outcome":"running"}));
        record.save()?;
        let result = serde_json::from_value(value)
            .map_err(anyhow::Error::from)
            .and_then(|step| runner.step(step, record));
        match result {
            Ok(outcome) => {
                record.value["steps"][index]["outcome"] =
                    json!(outcome.map_or("completed", |(_, outcome)| outcome));
                record.save()?;
                if let Some((code, _)) = outcome {
                    return Ok(code);
                }
            }
            Err(error) => {
                record.value["steps"][index]["outcome"] = json!({"error":error.to_string()});
                record.save()?;
                return Err(error);
            }
        }
    }
    runner.session(record)?;
    runner.output.terminal(None)?;
    Ok(0)
}

fn main() {
    if let Some(result) = native::probe() {
        finish(result);
    }
    let argv: Vec<String> = std::env::args().collect();
    let args = &argv[1..];
    let result = if args.first().is_some_and(|arg| arg == "--approval-host") {
        approvals::serve(args).map(|()| 0)
    } else if args
        .first()
        .is_some_and(|arg| arg == "--counting-mcp-server")
    {
        counting::serve(args).map(|()| 0)
    } else if args
        .first()
        .is_some_and(|arg| arg == "--native-contract-provider")
    {
        native::serve(args)
    } else {
        Record::new(&argv).and_then(|mut record| {
            let result = run(args, &mut record);
            if let Err(error) = &result {
                record.value["error"] = json!(error.to_string());
                record.save()?;
            }
            result
        })
    };
    finish(result);
}

fn finish(result: Result<i32>) -> ! {
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            eprint!("{error}");
            if !error.to_string().ends_with('\n') {
                eprintln!();
            }
            2
        }
    };
    std::process::exit(code);
}
