//! Black-box contracts for the installed `smith` process.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::Duration;

const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self::with_config(CONFIG)
    }

    fn with_config(config: &str) -> Self {
        let home = tempfile::tempdir().expect("a home");
        let project = tempfile::tempdir().expect("a project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a config directory");
        std::fs::write(config_dir.join("config.toml"), config).expect("a config");
        Self { home, project }
    }

    #[cfg(unix)]
    fn with_private_user_config(config: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Self::unconfigured();
        let config_dir = fixture.home.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a user config directory");
        let path = config_dir.join("config.toml");
        std::fs::write(&path, config).expect("a user config");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("an owner-only user config");
        fixture
    }

    fn unconfigured() -> Self {
        Self {
            home: tempfile::tempdir().expect("a home"),
            project: tempfile::tempdir().expect("a project"),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_smith"));
        command
            .current_dir(self.project.path())
            .env_clear()
            .env("HOME", self.home.path());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("smith ran")
    }
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout was not JSON: {error}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn serve_one_openai_request(listener: TcpListener) -> Vec<u8> {
    let (mut stream, _) = listener.accept().expect("one provider request");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("a read timeout");
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut buffer).expect("request bytes");
        assert!(read > 0, "request ended before its headers");
        request.extend_from_slice(&buffer[..read]);
        if let Some(index) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("content length"))
        })
        .unwrap_or(0);
    while request.len() < header_end + content_length {
        let read = stream.read(&mut buffer).expect("request body");
        assert!(read > 0, "request body ended early");
        request.extend_from_slice(&buffer[..read]);
    }

    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello from the endpoint.\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],",
        "\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":5,",
        "\"prompt_tokens_details\":{\"cached_tokens\":3}}}\n\n",
        "data: [DONE]\n\n",
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("provider response");
    request
}

#[test]
fn json_mode_runs_through_config_and_returns_a_resumable_identity() {
    let fixture = Fixture::new();
    let first = fixture.run(&["-p", "hello", "--output-format", "json"]);
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(first.stderr.is_empty(), "{:?}", first.stderr);
    let result = json(&first);
    assert_eq!(result["schema_version"], 3);
    assert_eq!(result["type"], "result");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["provider"], "local");
    assert_eq!(result["model"], "example-model");
    let activated = result["lifecycle"]["activation"]["capabilities"]
        .as_array()
        .expect("a frozen activation projection");
    assert!(!activated.is_empty());
    assert!(activated.iter().all(|id| {
        id.as_str().is_some_and(|id| {
            !id.contains("edit") && !id.contains("shell") && !id.contains("write_todos")
        })
    }));
    assert!(
        result["output"]
            .as_str()
            .is_some_and(|text| text.contains("deterministic fake provider"))
    );
    let session = result["session_id"].as_str().expect("a session id");

    let listed = fixture.run(&["sessions", "list"]);
    assert!(listed.status.success());
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains(session),
        "session was not listed"
    );

    let resumed = fixture.run(&[
        "-p",
        "again",
        "--resume",
        session,
        "--output-format",
        "json",
    ]);
    assert!(
        resumed.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(json(&resumed)["session_id"], session);
}

#[test]
fn unconfigured_headless_and_non_tty_setup_refuse_without_writing_user_state() {
    let fixture = Fixture::unconfigured();
    let headless = fixture.run(&["-p", "hello", "--output-format", "json"]);
    assert!(!headless.status.success());
    assert!(headless.stdout.is_empty());
    let diagnostic = String::from_utf8_lossy(&headless.stderr);
    assert!(
        diagnostic.contains("no configured provider/model"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("smith setup"), "{diagnostic}");
    assert!(
        !fixture.home.path().join(".smith").exists(),
        "headless refusal mutated user state"
    );

    let setup = fixture.run(&["setup"]);
    assert!(!setup.status.success());
    assert!(setup.stdout.is_empty());
    let diagnostic = String::from_utf8_lossy(&setup.stderr);
    assert!(diagnostic.contains("interactive terminal"), "{diagnostic}");
    assert!(
        !fixture.home.path().join(".smith").exists(),
        "non-TTY setup mutated user state"
    );
}

#[test]
fn bare_resume_refuses_headless_and_piped_use_with_a_session_list_hint() {
    let fixture = Fixture::new();
    for args in [
        &["--resume", "-p", "hello"][..],
        &["--resume", "--no-color"][..],
    ] {
        let output = fixture.run(args);
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains("smith sessions list"), "{diagnostic}");
        assert!(diagnostic.contains("--resume <SESSION_ID>"), "{diagnostic}");
    }
}

#[test]
fn session_list_prints_bounded_local_picker_metadata() {
    let fixture = Fixture::new();
    let run = fixture.run(&["-p", "distinguish this session", "--output-format", "json"]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let listed = fixture.run(&["sessions", "list"]);
    assert!(listed.status.success());
    let text = String::from_utf8(listed.stdout).expect("UTF-8 listing");
    let columns = text.trim_end().split('\t').collect::<Vec<_>>();
    assert_eq!(columns.len(), 5, "{text}");
    assert_eq!(columns[2], "1", "{text}");
    assert_eq!(columns[3], "local/example-model", "{text}");
    assert!(columns[4].contains("distinguish this session"), "{text}");
}

#[test]
fn stream_json_is_json_lines_with_a_terminal_result_after_shutdown() {
    let fixture = Fixture::new();
    let output = fixture.run(&["-p", "hello", "--output-format", "stream-json"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .expect("UTF-8 stdout")
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON value per line"))
        .collect();
    assert!(lines.len() > 3, "too few events: {lines:?}");
    assert_eq!(lines.last().expect("a result")["type"], "result");
    assert_eq!(lines.last().expect("a result")["status"], "ok");
    let streamed = &lines[..lines.len() - 1];
    assert!(
        streamed.iter().all(|line| matches!(
            line["type"].as_str(),
            Some("runtime_event" | "cache_controller")
        )),
        "stream contained an undocumented envelope: {streamed:?}"
    );
    let controller_positions = streamed
        .iter()
        .enumerate()
        .filter_map(|(position, line)| (line["type"] == "cache_controller").then_some(position))
        .collect::<Vec<_>>();
    assert_eq!(controller_positions.len(), 1, "{streamed:?}");
    let terminal_result = lines.last().expect("a result");
    let controller_envelope = &streamed[controller_positions[0]];
    assert!(
        controller_envelope["controller"].is_object(),
        "controller envelope omitted its bounded projection"
    );
    assert_eq!(
        controller_envelope["controller"], terminal_result["cache"]["controller"],
        "stream controller and terminal cache projection diverged"
    );
    let shutdown_position = streamed
        .iter()
        .position(|line| line["event"]["payload"]["event"] == "session_shutdown")
        .expect("stream omitted ordered shutdown");
    assert!(
        shutdown_position < controller_positions[0],
        "final controller must be captured after shutdown drains: {streamed:?}"
    );
}

#[test]
fn stdin_prompt_keeps_machine_stdout_clean() {
    let fixture = Fixture::new();
    let mut child = fixture
        .command()
        .args(["-p", "-", "--output-format", "json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("smith spawned");
    {
        use std::io::Write;
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"hello from stdin")
            .expect("prompt written");
    }
    let output = child.wait_with_output().expect("smith completed");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(json(&output)["status"], "ok");
    assert!(!output.stdout.contains(&0x1b), "stdout contained ANSI");
}

#[test]
fn config_explain_reports_the_selected_profile_source() {
    let fixture = Fixture::new();
    let output = fixture.run(&["config", "explain", "model"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("text output");
    assert!(text.contains("model = example-model"), "{text}");
    assert!(text.contains("selected profile"), "{text}");
}

/// A binding with an exact dialect and an advertised ladder, reached through
/// a credential that does not exist: anything the run rejects about the effort
/// is therefore proven to happen before the credential is looked up.
const LADDER_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "remote"
model = "example-model"

[providers.remote]
kind = "openai-compatible"
base_url = "http://127.0.0.1:1/v1"
credential = "env:SMITH_TEST_KEY_THAT_DOES_NOT_EXIST"

[models."remote/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[models."remote/example-model".reasoning]
toggle = true
efforts = ["low", "medium", "high"]
dialect = "openai-effort"
"#;

#[test]
fn help_lists_the_effort_flag() {
    let fixture = Fixture::new();
    let output = fixture.run(&["--help"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("text help");
    assert!(text.contains("--effort <NAME>"), "{text}");
}

#[test]
fn an_unsupported_invocation_effort_fails_with_alternatives_and_no_provider_work() {
    let fixture = Fixture::with_config(LADDER_CONFIG);

    let refused = fixture.run(&[
        "-p",
        "hello",
        "--effort",
        "ludicrous",
        "--output-format",
        "json",
    ]);
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(stderr.contains("ludicrous"), "{stderr}");
    assert!(stderr.contains("low, medium, high"), "{stderr}");
    // The credential is unresolvable and the endpoint is closed, so naming the
    // effort proves the refusal preceded both.
    assert!(
        !stderr.contains("SMITH_TEST_KEY_THAT_DOES_NOT_EXIST"),
        "{stderr}"
    );

    // A model with no adjustable reasoning at all says so rather than
    // ignoring the flag.
    let fixed = Fixture::new();
    let refused = fixed.run(&["-p", "hello", "--effort", "low", "--output-format", "json"]);
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(stderr.contains("reasoning"), "{stderr}");
    assert!(stderr.contains("not adjustable"), "{stderr}");
}

/// A live local endpoint whose model advertises an exact effort ladder and
/// whose profile already pins one of its rungs.
fn effort_ladder_config(address: std::net::SocketAddr) -> String {
    format!(
        r#"
default_profile = "prod"

[profiles.prod]
provider = "remote"
model = "test-model"

[profiles.prod.reasoning]
effort = "low"

[providers.remote]
kind = "openai-compatible"
base_url = "http://{address}/v1"
credential = "env:ACME_API_KEY"

[models."remote/test-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[models."remote/test-model".reasoning]
toggle = true
efforts = ["low", "medium", "high"]
dialect = "openai-effort"
"#
    )
}

#[test]
fn an_invocation_effort_overrides_a_profiles_effort_all_the_way_to_the_wire() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = listener.local_addr().expect("a listener address");
    let server = thread::spawn(move || serve_one_openai_request(listener));
    let fixture = Fixture::with_config(&effort_ladder_config(address));

    let output = fixture
        .command()
        .args([
            "-p",
            "hello",
            "--profile",
            "prod",
            "--effort",
            "high",
            "--output-format",
            "json",
        ])
        .env("ACME_API_KEY", "sk-effort-contract")
        .output()
        .expect("smith ran");
    let request = String::from_utf8_lossy(&server.join().expect("provider server")).into_owned();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = json(&output);
    assert_eq!(result["reasoning"]["effort"], "high");
    assert_eq!(
        result["reasoning"]["source"],
        "command-line flag `--effort`"
    );
    assert!(
        request.contains(r#""reasoning_effort":"high""#),
        "{request}"
    );
    // Everything else the profile pins is still the profile's.
    assert_eq!(result["provider"], "remote");
    assert_eq!(result["model"], "test-model");
}

#[test]
fn without_the_flag_the_profiles_effort_is_unchanged() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = listener.local_addr().expect("a listener address");
    let server = thread::spawn(move || serve_one_openai_request(listener));
    let fixture = Fixture::with_config(&effort_ladder_config(address));

    let output = fixture
        .command()
        .args(["-p", "hello", "--output-format", "json"])
        .env("ACME_API_KEY", "sk-effort-contract")
        .output()
        .expect("smith ran");
    let request = String::from_utf8_lossy(&server.join().expect("provider server")).into_owned();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = json(&output);
    assert_eq!(result["reasoning"]["effort"], "low");
    assert_eq!(result["reasoning"]["source"], "profile");
    assert!(request.contains(r#""reasoning_effort":"low""#), "{request}");
}

#[test]
fn config_explain_names_the_flag_that_supplied_an_effort() {
    let fixture = Fixture::with_config(LADDER_CONFIG);
    let explained = fixture.run(&["config", "explain", "reasoning.effort", "--effort", "high"]);
    assert!(
        explained.status.success(),
        "{}",
        String::from_utf8_lossy(&explained.stderr)
    );
    let text = String::from_utf8(explained.stdout).expect("text output");
    assert!(text.contains("reasoning.effort = high"), "{text}");
    assert!(
        text.contains("source: command-line flag `--effort`"),
        "{text}"
    );
}

#[test]
fn interactive_startup_failure_happens_before_any_terminal_escape() {
    let fixture = Fixture::with_config(
        r#"
[providers.local]
kind = "fake"
"#,
    );
    let output = fixture.run(&[]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "startup wrote to terminal stdout");
    assert!(!output.stderr.contains(&0x1b), "stderr contained ANSI");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("`provider` is not set"),
        "unexpected diagnostic: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn repository_approval_cannot_self_authorize_but_an_explicit_flag_can() {
    let fixture = Fixture::with_config(&format!("{CONFIG}\n[approval]\nmode = \"allow-all\"\n"));

    let refused = fixture.run(&["-p", "hello", "--output-format", "json"]);
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("cannot grant tool execution authority"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );

    let explicit = fixture.run(&[
        "-p",
        "hello",
        "--approval",
        "allow-all",
        "--output-format",
        "json",
    ]);
    assert!(
        explicit.status.success(),
        "{}",
        String::from_utf8_lossy(&explicit.stderr)
    );
    assert_eq!(json(&explicit)["status"], "ok");

    let yolo = fixture.run(&["-p", "hello", "--yolo", "--output-format", "json"]);
    assert!(
        yolo.status.success(),
        "{}",
        String::from_utf8_lossy(&yolo.stderr)
    );
    assert_eq!(json(&yolo)["status"], "ok");

    let auto = Fixture::with_config(&format!(
        "{CONFIG}\n[approval]\nauto_approve = [\"edit\"]\n"
    ));
    let refused = auto.run(&["-p", "hello", "--output-format", "json"]);
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("approval.auto_approve"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
}

#[test]
fn yolo_does_not_widen_a_read_only_plan_profile() {
    let fixture = Fixture::with_config(&format!(
        "{CONFIG}\n[profiles.plan]\nextends = \"dev\"\nposture = \"plan\"\nuse = [\"main\"]\n"
    ));

    let output = fixture.run(&[
        "-p",
        "Edit denied-proof.txt, then run a shell command",
        "--profile",
        "plan",
        "--yolo",
        "--output-format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let activated = json(&output)["lifecycle"]["activation"]["capabilities"]
        .as_array()
        .expect("a frozen activation projection")
        .clone();
    assert!(activated.iter().all(|id| {
        id.as_str().is_some_and(|id| {
            !id.contains("edit") && !id.contains("shell") && !id.contains("write_todos")
        })
    }));
    assert!(!fixture.project.path().join("denied-proof.txt").exists());
}

#[test]
fn repository_configuration_cannot_redirect_user_session_state() {
    let outside = tempfile::tempdir().expect("an unrelated directory");
    let target = outside.path().join("repo-selected-sessions");
    let fixture = Fixture::with_config(&format!(
        "{CONFIG}\n[persistence]\nsessions_dir = {:?}\n",
        target.to_string_lossy()
    ));

    let output = fixture.run(&["-p", "hello", "--output-format", "json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        !target.exists(),
        "repository-selected state path was created"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("user-scoped persistence"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let listed = fixture.run(&["sessions", "list"]);
    assert!(!listed.status.success());
    assert!(listed.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&listed.stderr).contains("user-scoped persistence"),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    assert!(
        !target.exists(),
        "listing created or read repository-selected session state"
    );
}

#[test]
fn user_configuration_may_choose_its_session_directory() {
    let fixture = Fixture::new();
    let target = fixture.home.path().join("private-smith-sessions");
    let user_dir = fixture.home.path().join(".smith");
    std::fs::create_dir_all(&user_dir).expect("a user config directory");
    std::fs::write(
        user_dir.join("config.toml"),
        format!(
            "[persistence]\nsessions_dir = {:?}\n",
            target.to_string_lossy()
        ),
    )
    .expect("a user config");

    let output = fixture.run(&["-p", "hello", "--output-format", "json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(target.is_dir(), "user-selected session directory missing");
}

#[test]
fn production_openai_compatible_stream_reaches_the_cli_with_usage_and_no_key_leak() {
    const KEY: &str = "sk-live-cli-contract-secret";
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = listener.local_addr().expect("a listener address");
    let server = thread::spawn(move || serve_one_openai_request(listener));
    let config = format!(
        r#"
default_profile = "prod"

[profiles.prod]
provider = "remote"
model = "test-model"

[providers.remote]
kind = "openai-compatible"
base_url = "http://{address}/v1"
credential = "env:ACME_API_KEY"

[models."remote/test-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#
    );
    let fixture = Fixture::with_config(&config);
    let output = fixture
        .command()
        .args(["-p", "hello", "--output-format", "json"])
        .env("ACME_API_KEY", KEY)
        .output()
        .expect("smith ran");
    let request = server.join().expect("provider server");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rendered = String::from_utf8(output.stdout).expect("UTF-8 JSON");
    assert!(!rendered.contains(KEY), "stdout leaked the credential");
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains(KEY),
        "stderr leaked the credential"
    );
    let result: serde_json::Value = serde_json::from_str(&rendered).expect("a result");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["output"], "Hello from the endpoint.");
    assert_eq!(result["usage"]["current_turn"]["input_uncached"], 8);
    assert_eq!(result["usage"]["current_turn"]["input_cached"], 3);
    assert_eq!(result["usage"]["current_turn"]["output"], 5);

    let request = String::from_utf8_lossy(&request);
    assert!(request.starts_with("POST /v1/chat/completions "));
    assert!(request.contains(&format!("Bearer {KEY}")));
}

#[cfg(unix)]
#[test]
fn owner_only_inline_key_runs_headless_without_a_resolver_and_machine_output_is_redacted() {
    const KEY: &str = "sk-inline-cli-contract-secret";
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = listener.local_addr().expect("a listener address");
    let server = thread::spawn(move || serve_one_openai_request(listener));
    let config = format!(
        r#"
default_profile = "prod"

[profiles.prod]
provider = "remote"
model = "test-model"

[providers.remote]
kind = "openai-compatible"
base_url = "http://{address}/v1"
api_key = "{KEY}"

[models."remote/test-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#
    );
    let fixture = Fixture::with_private_user_config(&config);
    let output = fixture.run(&["-p", "hello", "--output-format", "json"]);
    let request = server.join().expect("provider server");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 JSON");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(!stdout.contains(KEY), "stdout leaked the inline key");
    assert!(!stderr.contains(KEY), "stderr leaked the inline key");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout).expect("result")["status"],
        "ok"
    );
    let request = String::from_utf8_lossy(&request);
    assert!(request.contains(&format!("Bearer {KEY}")));

    let explained = fixture.run(&["config", "explain", "providers.remote.api_key"]);
    assert!(
        explained.status.success(),
        "{}",
        String::from_utf8_lossy(&explained.stderr)
    );
    let explanation = String::from_utf8(explained.stdout).expect("explanation");
    assert!(explanation.contains("[redacted]"), "{explanation}");
    assert!(!explanation.contains(KEY), "{explanation}");
}
