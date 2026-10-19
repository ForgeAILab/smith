//! Opt-in continuation proofs using the owner's real provider configuration.
//! See the live provider conformance section in the README.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use smith_runtime::session::SessionPaths;

#[allow(dead_code)]
#[path = "cli_contract/support.rs"]
mod support;

const PROCESS_TIMEOUT: Duration = Duration::from_secs(180);
const TWO_READS: &str = "Use your read tool on marker-a.txt and follow its instruction using another \
    read call. Do not guess or use shell commands. Reply with both markers and nothing else.";
const FOLLOW_UP: &str = "Use your read tool on marker-c.txt, then reply with its marker and the \
    marker from marker-a.txt from earlier in this session, and nothing else.";
const READ_C: &str =
    "Use your read tool on marker-c.txt and reply with its marker and nothing else.";
const READ_B: &str =
    "Use your read tool on marker-b.txt and reply with its marker and nothing else.";
const SEARCH: &str = "Use hosted web search to look up the public IANA example.com domain, and use \
    your read tool on marker-a.txt. Reply with the file's marker and one fact from the web search. \
    Do not use shell commands.";

#[derive(Clone)]
struct Route {
    name: String,
    provider: String,
    model: String,
}

fn routes(raw: &str) -> Vec<Route> {
    let mut seen = BTreeSet::new();
    raw.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter(|name| seen.insert((*name).to_owned()))
        .map(|name| {
            let (provider, model) = name.split_once('/').expect("routes must be provider/model");
            assert!(
                !provider.is_empty() && !model.is_empty(),
                "empty route component"
            );
            Route {
                name: name.into(),
                provider: provider.into(),
                model: model.into(),
            }
        })
        .collect()
}

struct Fixture {
    project: tempfile::TempDir,
    markers: [String; 3],
}

impl Fixture {
    fn new() -> Self {
        let project = tempfile::Builder::new()
            .prefix("smith-conformance-")
            .tempdir_in(std::env::temp_dir())
            .expect("scratch project");
        std::fs::create_dir(project.path().join(".smith")).expect("project config directory");
        // Establish a project boundary without moving or overriding the user config.
        std::fs::write(project.path().join(".smith/config.toml"), "").expect("project boundary");
        let markers = std::array::from_fn(|_| format!("smith-marker-{}", uuid::Uuid::new_v4()));
        for (index, file) in ["marker-a.txt", "marker-b.txt", "marker-c.txt"]
            .iter()
            .enumerate()
        {
            let instruction = if index == 0 {
                "Next, read marker-b.txt.\n"
            } else {
                ""
            };
            std::fs::write(
                project.path().join(file),
                format!("{}\n{instruction}", markers[index]),
            )
            .expect("marker fixture");
        }
        Self { project, markers }
    }
}

#[derive(Default, Serialize)]
struct Turn {
    model: String,
    reported_provider: Option<String>,
    reported_model: Option<String>,
    wall_seconds: f64,
    session_id: Option<String>,
    output: Option<String>,
    usage: Option<Value>,
    usage_provenance: Option<String>,
    cache_read_percent: Option<u64>,
    smith_error: Option<String>,
    errors: Vec<String>,
    stderr_tail: String,
    completed_reads: usize,
    hosted_search: bool,
    interrupted_after_tool_result: bool,
}

#[derive(Serialize)]
struct Scenario {
    name: String,
    models: Vec<String>,
    status: &'static str,
    wall_seconds: f64,
    session_id: Option<String>,
    note: Option<String>,
    turns: Vec<Turn>,
}

impl Scenario {
    fn new(name: &str, models: &[&Route], turns: Vec<Turn>, note: Option<String>) -> Self {
        let status = if turns.is_empty() {
            "skip"
        } else if turns.iter().any(|t| !t.errors.is_empty()) {
            "fail"
        } else {
            "pass"
        };
        Self {
            name: name.into(),
            models: models.iter().map(|route| route.name.clone()).collect(),
            status,
            wall_seconds: turns.iter().map(|turn| turn.wall_seconds).sum(),
            session_id: turns.iter().rev().find_map(|turn| turn.session_id.clone()),
            note,
            turns,
        }
    }
}

#[derive(Default)]
struct Capture {
    stdout: Vec<String>,
    stderr: Vec<String>,
    errors: Vec<String>,
    success: bool,
    killed_on_result: bool,
}

struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        support::kill_group(self.0.id());
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

enum Line {
    Stdout(String),
    Stderr(String),
    Error(String),
}

fn read_lines(reader: impl Read + Send + 'static, sender: mpsc::Sender<Line>, stderr: bool) {
    thread::spawn(move || {
        for line in BufReader::new(reader).split(b'\n') {
            let message = match line {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    if stderr {
                        Line::Stderr(text)
                    } else {
                        Line::Stdout(text)
                    }
                }
                Err(error) => Line::Error(format!("reading process pipe: {error}")),
            };
            if sender.send(message).is_err() {
                break;
            }
        }
    });
}

fn payload(line: &Value) -> &Value {
    &line["event"]["payload"]
}

fn successful_read(line: &Value) -> bool {
    payload(line)["event"] == "tool_call_completed"
        && payload(line)["name"] == "read"
        && payload(line)["is_error"] == false
}

// Unlike the contract helper's assertion-based deadline, preserve partial output
// on timeout so a failed provider cannot prevent the remaining scenarios/report.
fn capture(command: &mut Command, interrupt: bool) -> Capture {
    let mut captured = Capture::default();
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => OwnedChild(child),
        Err(error) => {
            captured.errors.push(format!("starting Smith: {error}"));
            return captured;
        }
    };
    let (sender, receiver) = mpsc::channel();
    read_lines(
        child.0.stdout.take().expect("stdout pipe"),
        sender.clone(),
        false,
    );
    read_lines(child.0.stderr.take().expect("stderr pipe"), sender, true);
    let mut deadline = Instant::now() + PROCESS_TIMEOUT;
    let mut exited = false;
    let mut root_turn = None;
    loop {
        if Instant::now() >= deadline {
            captured.errors.push(if exited {
                "process pipes did not close".into()
            } else {
                "Smith exceeded the 180 second process timeout".into()
            });
            break;
        }
        match receiver.recv_timeout(Duration::from_millis(5)) {
            Ok(Line::Stdout(line)) => {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if payload(&value)["event"] == "turn_started" && root_turn.is_none() {
                        root_turn = value["event"]["turn"].as_str().map(str::to_owned);
                    }
                    if interrupt
                        && !captured.killed_on_result
                        && payload(&value)["event"] == "tool_call_completed"
                        && root_turn
                            .as_deref()
                            .is_some_and(|turn| value["event"]["turn"] == turn)
                    {
                        match child.0.kill() {
                            Ok(()) => captured.killed_on_result = true,
                            Err(error) => captured
                                .errors
                                .push(format!("SIGKILL after tool result: {error}")),
                        }
                        support::kill_group(child.0.id());
                    }
                }
                captured.stdout.push(line);
            }
            Ok(Line::Stderr(line)) => {
                if !line.to_ascii_lowercase().contains("api_key") {
                    captured.stderr.push(line);
                }
            }
            Ok(Line::Error(error)) => captured.errors.push(error),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if exited {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
        if !exited {
            match child.0.try_wait() {
                Ok(Some(status)) => {
                    captured.success = status.success();
                    exited = true;
                    support::kill_group(child.0.id());
                    deadline = Instant::now() + support::POST_EXIT_DRAIN_GRACE;
                }
                Ok(None) => {}
                Err(error) => {
                    captured.errors.push(format!("waiting for Smith: {error}"));
                    break;
                }
            }
        }
    }
    captured
}

fn parse_turn(captured: Capture, route: &Route, elapsed: Duration, interrupt: bool) -> Turn {
    let mut turn = Turn {
        model: route.name.clone(),
        wall_seconds: elapsed.as_secs_f64(),
        stderr_tail: captured
            .stderr
            .into_iter()
            .rev()
            .take(20)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n"),
        errors: captured.errors,
        interrupted_after_tool_result: captured.killed_on_result,
        ..Turn::default()
    };
    let mut result = None;
    let mut completed = false;
    let mut root_turn = None;
    for line in captured.stdout {
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                turn.errors.push(format!("invalid JSON output: {error}"));
                continue;
            }
        };
        if turn.session_id.is_none() {
            turn.session_id = value["event"]["session"].as_str().map(str::to_owned);
        }
        if payload(&value)["event"] == "turn_started" && root_turn.is_none() {
            root_turn = value["event"]["turn"].as_str().map(str::to_owned);
        }
        if root_turn
            .as_deref()
            .is_some_and(|id| value["event"]["turn"] == id)
        {
            turn.completed_reads += usize::from(successful_read(&value));
            turn.hosted_search |= payload(&value)["event"] == "hosted_search";
            completed |= payload(&value)["event"] == "turn_completed";
        }
        if value["type"] == "result" {
            result = Some(value);
        }
    }
    if let Some(result) = &result {
        turn.reported_provider = result["provider"].as_str().map(str::to_owned);
        turn.reported_model = result["model"].as_str().map(str::to_owned);
        if result["provider"] != route.provider || result["model"] != route.model {
            turn.errors
                .push("result did not use the requested provider/model route".into());
        }
        turn.session_id = result["session_id"]
            .as_str()
            .map(str::to_owned)
            .or(turn.session_id);
        turn.output = result["output"].as_str().map(str::to_owned);
        turn.usage = result["usage"].get("current_turn").cloned();
        turn.usage_provenance = result["usage"]["current_turn_provenance"]
            .as_str()
            .map(str::to_owned);
        turn.cache_read_percent = result["cache"]["cache_read_percent"].as_u64();
        turn.smith_error = result["error"].as_str().map(str::to_owned);
    }
    if interrupt {
        if !captured.killed_on_result || turn.completed_reads == 0 {
            turn.errors
                .push("no successful tool result was observed before interruption".into());
        }
        if completed || result.is_some() {
            turn.errors
                .push("turn finished before SIGKILL; stranded history was not proven".into());
        }
    } else if !captured.success
        || result.as_ref().is_none_or(|r| r["status"] != "ok")
        || turn.smith_error.is_some()
    {
        turn.errors
            .push("Smith did not exit successfully with a status=ok result".into());
    }
    if turn.session_id.is_none() {
        turn.errors.push("Smith reported no session id".into());
    }
    turn
}

struct Suite {
    fixture: Fixture,
    binary: PathBuf,
    paths: BTreeMap<String, SessionPaths>,
    sessions: BTreeSet<(String, String)>,
}

impl Suite {
    fn command(&self, route: &Route, args: &[&str]) -> Command {
        let mut command = Command::new(&self.binary);
        command.args(args);
        command.current_dir(self.fixture.project.path()).args([
            "--project",
            self.fixture
                .project
                .path()
                .to_str()
                .expect("UTF-8 scratch path"),
            "--provider",
            &route.provider,
            "--model",
            &route.model,
            "--approval",
            "deny",
        ]);
        // Keep HOME, credential environment variables, and the owner's profiles intact.
        for (name, _) in std::env::vars_os() {
            if name
                .to_string_lossy()
                .starts_with("SMITH_LIVE_CONFORMANCE_")
            {
                command.env_remove(name);
            }
        }
        // Scoped auto-approval rules otherwise take precedence over deny.
        command.env("SMITH_APPROVAL_AUTO", "[]");
        command.env("SMITH_APPROVAL_AUTO_APPROVE", "");
        command
    }

    fn session_paths(&mut self, route: &Route) -> Result<(), String> {
        if self.paths.contains_key(&route.name) {
            return Ok(());
        }
        let mut command = self.command(route, &["config", "explain", "persistence.sessions_dir"]);
        // Query the tested binary, including custom binaries and configured store locations.
        let captured = capture(&mut command, false);
        if !captured.success || !captured.errors.is_empty() {
            return Err(format!(
                "cannot locate session store: {}; {}",
                captured.errors.join("; "),
                captured.stderr.join("\n")
            ));
        }
        let path = captured
            .stdout
            .iter()
            .find_map(|line| line.strip_prefix("persistence.sessions_dir = "))
            .ok_or("config explain did not report persistence.sessions_dir")?;
        let root = PathBuf::from(path);
        let root = if root.is_absolute() {
            root
        } else {
            self.fixture.project.path().join(root)
        };
        let project = smith_runtime::host::project_id(self.fixture.project.path())
            .map_err(|e| e.to_string())?;
        self.paths.insert(
            route.name.clone(),
            SessionPaths::from_sessions_dir(root, &project),
        );
        Ok(())
    }

    fn run(
        &mut self,
        route: &Route,
        session: Option<&str>,
        prompt: &str,
        interrupt: bool,
        markers: &[usize],
        reads: usize,
    ) -> Turn {
        let start = Instant::now();
        if let Err(error) = self.session_paths(route) {
            return Turn {
                model: route.name.clone(),
                errors: vec![error],
                wall_seconds: start.elapsed().as_secs_f64(),
                ..Turn::default()
            };
        }
        let mut command = self.command(route, &["-p", prompt, "--output-format", "stream-json"]);
        if let Some(session) = session {
            command.args(["--resume", session]);
        }
        let mut turn = parse_turn(
            capture(&mut command, interrupt),
            route,
            start.elapsed(),
            interrupt,
        );
        if let Some(id) = &turn.session_id {
            self.sessions.insert((route.name.clone(), id.clone()));
            if session.is_some_and(|expected| expected != id) {
                turn.errors
                    .push("resume returned a different session id".into());
            }
        }
        if !interrupt {
            let output = turn
                .output
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            for &index in markers {
                if !output.contains(&self.fixture.markers[index].to_ascii_lowercase()) {
                    turn.errors
                        .push(format!("final output omitted marker {}", index + 1));
                }
            }
            if turn.completed_reads < reads {
                turn.errors
                    .push(format!("expected at least {reads} successful read results"));
            }
        }
        turn
    }

    async fn cleanup(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for (route, id) in &self.sessions {
            let paths = &self.paths[route];
            let session = agent_runtime_core::ids::SessionId::from(id.as_str());
            if let Err(error) = paths.remove_session_files(&session).await {
                errors.push(error.to_string());
            }
            match paths.changes(&session) {
                Ok(path) => {
                    if let Err(error) = std::fs::remove_file(path)
                        && error.kind() != std::io::ErrorKind::NotFound
                    {
                        errors.push(error.to_string());
                    }
                }
                Err(error) => errors.push(error.to_string()),
            }
            // Each run also leaves its diagnostic log under the state root.
            if let Some(home) = std::env::var_os("HOME") {
                match smith_runtime::session::session_log_path(
                    Path::new(&home).join(".smith"),
                    &session,
                ) {
                    Ok(path) => {
                        if let Err(error) = std::fs::remove_file(path)
                            && error.kind() != std::io::ErrorKind::NotFound
                        {
                            errors.push(error.to_string());
                        }
                    }
                    Err(error) => errors.push(error.to_string()),
                }
            }
        }
        // The scratch project's session directory, once nothing is left in it.
        for paths in self.paths.values() {
            let _ = std::fs::remove_dir(paths.directory());
        }
        errors
    }
}

#[derive(Serialize)]
struct Report {
    scenarios: Vec<Scenario>,
    cleanup_errors: Vec<String>,
}

fn cell(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "&#124;")
        .replace('\n', "<br>")
        .replace('\r', "")
}

fn write_report(directory: &Path, report: &Report) {
    let mut markdown = String::from(
        "# Smith live provider conformance\n\nUsage/cache are per turn (in execution order); unavailable values are null. Cache percentages are informational only.\n\n| Scenario | Models | Status | Seconds | Session | Usage | Cache read % | Answers / note |\n| --- | --- | --- | ---: | --- | --- | --- | --- |\n",
    );
    for scenario in &report.scenarios {
        let usage = scenario
            .turns
            .iter()
            .map(|turn| turn.usage.as_ref().map_or("null".into(), Value::to_string))
            .collect::<Vec<_>>()
            .join(" / ");
        let cache = scenario
            .turns
            .iter()
            .map(|turn| {
                turn.cache_read_percent
                    .map_or("null".into(), |n| n.to_string())
            })
            .collect::<Vec<_>>()
            .join(" / ");
        let answers = scenario
            .turns
            .iter()
            .map(|turn| turn.output.as_deref().unwrap_or("(no final answer)"))
            .collect::<Vec<_>>()
            .join(" / ");
        markdown.push_str(&format!(
            "| {} | {} | {} | {:.2} | {} | {} | {} | {} |\n",
            cell(&scenario.name),
            cell(&scenario.models.join(" → ")),
            scenario.status,
            scenario.wall_seconds,
            cell(scenario.session_id.as_deref().unwrap_or("—")),
            cell(&usage),
            cell(&cache),
            cell(scenario.note.as_deref().unwrap_or(&answers))
        ));
    }
    markdown.push_str("\n## Failures\n");
    for scenario in report.scenarios.iter().filter(|s| s.status == "fail") {
        markdown.push_str(&format!(
            "\n### {} ({})\n",
            cell(&scenario.name),
            cell(&scenario.models.join(" → "))
        ));
        for turn in &scenario.turns {
            if turn.errors.is_empty() {
                continue;
            }
            markdown.push_str(&format!("\n{}\n\n", cell(&turn.model)));
            // Indented blocks preserve complete multiline errors, including embedded fences.
            for (label, text) in [
                ("Checks", turn.errors.join("\n")),
                (
                    "Smith result error",
                    turn.smith_error.clone().unwrap_or_default(),
                ),
                (
                    "Stderr (last 20 lines, api_key lines removed)",
                    turn.stderr_tail.clone(),
                ),
            ] {
                markdown.push_str(&format!("{label}:\n\n"));
                for line in text.lines() {
                    markdown.push_str(&format!("    {line}\n"));
                }
                markdown.push('\n');
            }
        }
    }
    for error in &report.cleanup_errors {
        markdown.push_str(&format!("\nCleanup: {}\n", cell(error)));
    }
    std::fs::create_dir_all(directory).expect("report directory");
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(report).expect("report JSON"),
    )
    .expect("JSON report written");
    std::fs::write(directory.join("report.md"), &markdown).expect("Markdown report written");
    println!("{markdown}");
}

#[tokio::test]
#[ignore = "requires SMITH_LIVE_CONFORMANCE_MODELS and spends real provider quota"]
async fn configured_providers_conform_across_turns_and_resumes() {
    let Ok(raw) = std::env::var("SMITH_LIVE_CONFORMANCE_MODELS") else {
        return;
    };
    let models = routes(&raw);
    if models.is_empty() {
        return;
    }
    let search = std::env::var("SMITH_LIVE_CONFORMANCE_SEARCH").unwrap_or_default();
    let search: BTreeSet<_> = search
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    assert!(
        search
            .iter()
            .all(|name| models.iter().any(|route| route.name == *name)),
        "search routes must be a subset of conformance models"
    );
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let directory = std::env::var_os("SMITH_LIVE_CONFORMANCE_REPORT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target/live-conformance"));
    let directory = if directory.is_absolute() {
        directory
    } else {
        workspace.join(directory)
    };
    let binary = std::env::var_os("SMITH_LIVE_CONFORMANCE_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_smith")));
    let binary = if binary.is_absolute() {
        binary
    } else {
        std::env::current_dir()
            .expect("current directory")
            .join(binary)
    };
    let mut suite = Suite {
        fixture: Fixture::new(),
        binary,
        paths: BTreeMap::new(),
        sessions: BTreeSet::new(),
    };
    let mut scenarios = Vec::new();
    for model in &models {
        let first = suite.run(model, None, TWO_READS, false, &[0, 1], 2);
        let session = first.session_id.clone();
        scenarios.push(Scenario::new("tool_turn", &[model], vec![first], None));
        let turns = session
            .as_deref()
            .map(|id| vec![suite.run(model, Some(id), FOLLOW_UP, false, &[0, 2], 1)])
            .unwrap_or_default();
        scenarios.push(Scenario::new(
            "continuation",
            &[model],
            turns,
            session
                .is_none()
                .then(|| "tool_turn reported no session to resume".into()),
        ));

        #[cfg(unix)]
        {
            let first = suite.run(model, None, TWO_READS, true, &[], 0);
            let mut turns = Vec::new();
            if first.interrupted_after_tool_result
                && let Some(id) = first.session_id.as_deref()
            {
                turns.push(suite.run(model, Some(id), READ_C, false, &[2], 1));
            }
            turns.insert(0, first);
            scenarios.push(Scenario::new("stranded_resume", &[model], turns, None));
        }
        #[cfg(not(unix))]
        scenarios.push(Scenario::new(
            "stranded_resume",
            &[model],
            Vec::new(),
            Some("requires Unix SIGKILL".into()),
        ));

        if search.contains(model.name.as_str()) {
            let mut first = suite.run(model, None, SEARCH, false, &[0], 1);
            if !first.hosted_search {
                first
                    .errors
                    .push("no hosted_search event was observed".into());
            }
            let mut turns = Vec::new();
            if let Some(id) = first.session_id.as_deref() {
                turns.push(suite.run(model, Some(id), READ_B, false, &[1], 1));
            }
            turns.insert(0, first);
            scenarios.push(Scenario::new("search_and_tool", &[model], turns, None));
        } else {
            scenarios.push(Scenario::new(
                "search_and_tool",
                &[model],
                Vec::new(),
                Some("route not in SMITH_LIVE_CONFORMANCE_SEARCH".into()),
            ));
        }
    }
    for source in &models {
        for destination in &models {
            if source.name == destination.name {
                continue;
            }
            let first = suite.run(source, None, TWO_READS, false, &[0, 1], 2);
            let mut turns = Vec::new();
            if let Some(id) = first.session_id.as_deref() {
                turns.push(suite.run(destination, Some(id), FOLLOW_UP, false, &[0, 2], 1));
            }
            turns.insert(0, first);
            scenarios.push(Scenario::new(
                "cross_provider",
                &[source, destination],
                turns,
                None,
            ));
        }
    }
    if models.len() < 2 {
        scenarios.push(Scenario::new(
            "cross_provider",
            &[&models[0]],
            Vec::new(),
            Some("requires at least two distinct routes".into()),
        ));
    }
    let report = Report {
        scenarios,
        cleanup_errors: suite.cleanup().await,
    };
    write_report(&directory, &report);
    assert!(
        report.scenarios.iter().all(|s| s.status != "fail") && report.cleanup_errors.is_empty(),
        "live conformance failed; see {}",
        directory.display()
    );
}
