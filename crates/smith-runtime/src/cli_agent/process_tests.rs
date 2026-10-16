//! Process lifetime and stderr diagnostics tests using shell launchers.

use std::os::unix::fs::PermissionsExt as _;
use std::time::Duration;

use agent_runtime::agent::external::{
    ExternalAgentBackend, ExternalAgentEvent, ExternalToolDispatch, ExternalTurnRequest,
};
use agent_runtime_core::cancel::{CancelReason, Cancellation};
use agent_runtime_core::content::UserInput;
use agent_runtime_core::ids::TurnId;
use futures_util::StreamExt as _;
use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use super::{CliAgentBackend, CliAgentKind, CliAgentSettings};

struct Launcher {
    directory: tempfile::TempDir,
    cleanup: bool,
}

impl Launcher {
    fn new() -> Self {
        Self::with_script(
            r#"#!/bin/sh
sleep 300 &
printf '%s %s\n' "$$" "$!" > "$PID_FILE"
printf '%s\n' 'cancellation stderr noise' >&2
printf '%s\n' '{"type":"system","subtype":"init","session_id":"launcher-session"}'
wait
"#,
        )
    }

    fn with_script(contents: &str) -> Self {
        let directory = tempfile::tempdir().expect("a launcher directory");
        let script = directory.path().join("launcher");
        std::fs::write(&script, contents).expect("a launcher script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .expect("an executable launcher");
        Self {
            directory,
            cleanup: true,
        }
    }

    fn backend(&self) -> CliAgentBackend {
        self.backend_for(CliAgentKind::ClaudeCode)
    }

    fn backend_for(&self, kind: CliAgentKind) -> CliAgentBackend {
        CliAgentBackend::new(
            kind,
            CliAgentSettings {
                executable: self.directory.path().join("launcher"),
                model: None,
                args: Vec::new(),
                cwd: self.directory.path().to_owned(),
                env: [(
                    "PID_FILE".to_owned(),
                    self.directory.path().join("pids").display().to_string(),
                )]
                .into(),
                allow_own_tools: false,
                instructions: None,
                bridge: None,
                relay_executable: None,
            },
        )
    }

    fn pids(&self) -> Option<Vec<Pid>> {
        let contents = std::fs::read_to_string(self.directory.path().join("pids")).ok()?;
        contents
            .split_whitespace()
            .map(|pid| {
                pid.parse::<i32>()
                    .ok()
                    .filter(|pid| *pid > 0)
                    .map(Pid::from_raw)
            })
            .collect()
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        // A failed assertion must not leave the fixture's sleep running,
        // even if the backend's cleanup is the regression under test.
        if self.cleanup
            && let Some(pids) = self.pids()
        {
            for pid in pids {
                let _ = nix::sys::signal::killpg(pid, Signal::SIGKILL);
                let _ = kill(pid, Signal::SIGKILL);
            }
        }
    }
}

async fn stopped(pids: &[Pid]) {
    loop {
        if pids.iter().all(|pid| kill(*pid, None) == Err(Errno::ESRCH)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn stop_launcher(drop_stream: bool) {
    let mut launcher = Launcher::new();
    let cancel = Cancellation::new();
    let mut stream = launcher
        .backend()
        .run_turn(ExternalTurnRequest {
            input: UserInput::text("hello"),
            resume: None,
            turn: TurnId::new("launcher-turn"),
            cancel: cancel.clone(),
            capabilities: Default::default(),
            bridge: None,
            tools: ExternalToolDispatch::disabled(),
        })
        .await
        .expect("a running launcher");
    loop {
        let event = stream.next().await.expect("the launcher's session event");
        if matches!(event, ExternalAgentEvent::SessionStarted { .. }) {
            break;
        }
    }
    let pids = launcher.pids().expect("the launcher and child pids");
    assert_eq!(pids.len(), 2);
    for pid in &pids {
        assert_eq!(kill(*pid, None), Ok(()), "fixture process {pid} is running");
    }

    if drop_stream {
        drop(stream);
    } else {
        cancel.cancel(CancelReason::UserRequested);
        match stream.next().await {
            Some(ExternalAgentEvent::Failed { message }) => {
                assert_eq!(message, "CLI agent exited before reporting a result");
            }
            event => panic!("expected cancellation's failure, got {event:?}"),
        }
        // Keep the stream alive while checking cancellation's effect.
        stopped(&pids).await;
        drop(stream);
    }
    stopped(&pids).await;
    launcher.cleanup = false;
}

#[tokio::test]
async fn cancelling_a_turn_stops_the_launcher_and_its_child() {
    tokio::time::timeout(Duration::from_secs(10), stop_launcher(false))
        .await
        .expect("cancellation stops both processes before the deadline");
}

#[tokio::test]
async fn dropping_a_turn_stream_stops_the_launcher_and_its_child() {
    tokio::time::timeout(Duration::from_secs(10), stop_launcher(true))
        .await
        .expect("dropping the stream stops both processes before the deadline");
}

async fn launcher_events(script: &str, kind: CliAgentKind) -> Vec<ExternalAgentEvent> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let launcher = Launcher::with_script(script);
        let stream = launcher
            .backend_for(kind)
            .run_turn(ExternalTurnRequest {
                input: UserInput::text("hello"),
                resume: None,
                turn: TurnId::new("stderr-turn"),
                cancel: Cancellation::new(),
                capabilities: Default::default(),
                bridge: None,
                tools: ExternalToolDispatch::disabled(),
            })
            .await
            .expect("a running stderr launcher");
        stream.collect().await
    })
    .await
    .expect("the stderr launcher ends before the deadline")
}

fn failure_message(events: &[ExternalAgentEvent]) -> &str {
    match events {
        [ExternalAgentEvent::Failed { message }] => message,
        events => panic!("expected one failure, got {events:?}"),
    }
}

#[tokio::test]
async fn an_exit_without_a_result_reports_the_last_stderr_line() {
    let events = launcher_events(
        "#!/bin/sh\nprintf 'first reason\\n\\t last reason \\r\\n\\n' >&2\nexit 1\n",
        CliAgentKind::ClaudeCode,
    )
    .await;
    assert_eq!(
        failure_message(&events),
        "CLI agent exited before reporting a result: last reason"
    );
}

#[tokio::test]
async fn a_codex_mcp_startup_failure_preserves_its_stderr_reason() {
    let reason = "Error: thread/start: thread/start failed: error creating thread: Fatal error: Failed to initialize session: required MCP servers failed to initialize: smith: timed out handshaking with MCP server after 60s (code -32603)";
    let events = launcher_events(
        &format!("#!/bin/sh\nprintf '%s\\n' '{reason}' >&2\nexit 1\n"),
        CliAgentKind::Codex,
    )
    .await;
    let message = failure_message(&events);
    assert!(message.contains("required MCP servers failed to initialize"));
    assert!(message.ends_with(reason));
}

#[tokio::test]
async fn a_megabyte_of_stderr_is_drained_and_the_final_reason_is_bounded() {
    let reason = "é".repeat(600);
    let events = launcher_events(
        &format!(
            "#!/bin/sh\ndd if=/dev/zero bs=1048576 count=1 2>/dev/null | tr '\\000' x >&2\nprintf '\\n%s\\n' '{reason}' >&2\nexit 1\n"
        ),
        CliAgentKind::ClaudeCode,
    )
    .await;
    assert_eq!(
        failure_message(&events),
        format!(
            "CLI agent exited before reporting a result: {}…",
            "é".repeat(499)
        )
    );
}

#[tokio::test]
async fn a_claude_result_ignores_stderr_noise() {
    let events = launcher_events(
        r#"#!/bin/sh
printf '%s\n' 'routine stderr noise' >&2
printf '%s\n' '{"type":"result","is_error":false,"result":"done"}'
"#,
        CliAgentKind::ClaudeCode,
    )
    .await;
    assert!(matches!(events.as_slice(), [ExternalAgentEvent::Completed]));
}

#[tokio::test]
async fn a_cli_reported_failure_ignores_stderr_noise() {
    let events = launcher_events(
        r#"#!/bin/sh
printf '%s\n' 'routine stderr noise' >&2
printf '%s\n' '{"type":"result","is_error":true,"result":"reported failure"}'
"#,
        CliAgentKind::ClaudeCode,
    )
    .await;
    assert_eq!(failure_message(&events), "reported failure");
}

#[tokio::test]
async fn an_exit_with_no_stderr_keeps_the_existing_failure_message() {
    let events = launcher_events("#!/bin/sh\nexit 1\n", CliAgentKind::ClaudeCode).await;
    assert_eq!(
        failure_message(&events),
        "CLI agent exited before reporting a result"
    );
}

#[tokio::test]
async fn an_exit_with_only_stderr_control_characters_has_no_reason_suffix() {
    let events = launcher_events(
        "#!/bin/sh\nprintf '\\001\\002\\t\\r\\n' >&2\nexit 1\n",
        CliAgentKind::ClaudeCode,
    )
    .await;
    assert_eq!(
        failure_message(&events),
        "CLI agent exited before reporting a result"
    );
}

async fn disabled_bridge_argv(kind: CliAgentKind) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let launcher = Launcher::with_script(
            r#"#!/bin/sh
printf '%s\n' "$@" > "$ARGV_FILE"
printf '%s\n' "$TERMINAL"
"#,
        );
        let mut backend = launcher.backend_for(kind);
        let recorded = launcher.directory.path().join("argv");
        backend
            .settings
            .env
            .insert("ARGV_FILE".into(), recorded.display().to_string());
        let terminal = match kind {
            CliAgentKind::ClaudeCode => r#"{"type":"result","is_error":false}"#,
            CliAgentKind::Codex => r#"{"type":"turn.completed","usage":{}}"#,
        };
        backend
            .settings
            .env
            .insert("TERMINAL".into(), terminal.into());
        let expected = backend.argv("hello", None).expect("not-bridged argv");
        let stream = backend
            .run_turn(ExternalTurnRequest {
                input: UserInput::text("hello"),
                resume: None,
                turn: TurnId::new("disabled-bridge-turn"),
                cancel: Cancellation::new(),
                capabilities: Default::default(),
                bridge: None,
                tools: ExternalToolDispatch::disabled(),
            })
            .await
            .expect("recording launcher");
        let events: Vec<_> = stream.collect().await;
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ExternalAgentEvent::Completed))
        );
        let argv = std::fs::read_to_string(recorded).expect("recorded argv");
        assert_eq!(argv.lines().collect::<Vec<_>>(), expected);
        assert!(
            !argv.contains("mcp"),
            "an empty bridge must not declare an endpoint: {argv}"
        );
    })
    .await
    .expect("disabled bridge launch ends before the deadline");
}

#[tokio::test]
async fn claude_disabled_bridge_preserves_the_launchers_exact_argv() {
    disabled_bridge_argv(CliAgentKind::ClaudeCode).await;
}

#[tokio::test]
async fn codex_disabled_bridge_preserves_the_launchers_exact_argv() {
    disabled_bridge_argv(CliAgentKind::Codex).await;
}

/// Inspect the real run_turn spawn request, substituting only the OS spawn.
/// A native cat keeps reading stdin until EOF, so it also probes guard lifetime
/// without implementing a second supervisor in a shell fixture.
fn inspect_supervised_spawn(
    command: &mut tokio::process::Command,
) -> std::io::Result<tokio::process::Child> {
    use std::ffi::OsStr;
    use std::process::Stdio;

    let command = command.as_std();
    assert_eq!(command.get_program(), OsStr::new("/injected/smith"));
    let env = command
        .get_envs()
        .collect::<std::collections::BTreeMap<_, _>>();
    let value = |key: &str| env[OsStr::new(key)].expect("test overlay value");
    let expected: Vec<String> =
        serde_json::from_str(value("EXPECTED_ARGV").to_str().unwrap()).unwrap();
    let actual = command
        .get_args()
        .map(|s| s.to_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert_eq!(
        command.get_current_dir(),
        Some(std::path::Path::new(value("EXPECTED_CWD")))
    );
    assert_eq!(value("SMITH_TEST_OVERLAY"), "unchanged overlay");
    let child = tokio::process::Command::new("/bin/cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()?;
    std::fs::write(value("PID_FILE"), child.id().unwrap().to_string())?;
    Ok(child)
}

#[tokio::test]
async fn run_turn_wraps_unchanged_argv_and_retains_stdin_until_stream_drop() {
    tokio::time::timeout(Duration::from_secs(10), async {
        for kind in [CliAgentKind::ClaudeCode, CliAgentKind::Codex] {
            let launcher = Launcher::new();
            let mut backend = launcher.backend_for(kind);
            backend.settings.relay_executable = Some("/injected/smith".into());
            backend.settings.args = vec![
                "--owner-option".into(),
                "spaces 'quotes'\nand unicode é".into(),
                "--".into(),
            ];
            backend.spawn_override = Some(inspect_supervised_spawn);
            let prompt = "prompt with --flags\nand 'quotes'";
            let cli_args = backend.argv(prompt, None).unwrap();
            let expected = [
                "harness-supervise".to_owned(),
                "--".to_owned(),
                backend.settings.executable.display().to_string(),
            ]
            .into_iter()
            .chain(cli_args)
            .collect::<Vec<_>>();
            backend.settings.env.insert(
                "EXPECTED_ARGV".into(),
                serde_json::to_string(&expected).unwrap(),
            );
            backend.settings.env.insert(
                "EXPECTED_CWD".into(),
                launcher.directory.path().display().to_string(),
            );
            backend
                .settings
                .env
                .insert("SMITH_TEST_OVERLAY".into(), "unchanged overlay".into());
            let mut stream = backend
                .run_turn(ExternalTurnRequest {
                    input: UserInput::text(prompt),
                    resume: None,
                    turn: TurnId::new("supervised-turn"),
                    cancel: Cancellation::new(),
                    capabilities: Default::default(),
                    bridge: None,
                    tools: ExternalToolDispatch::disabled(),
                })
                .await
                .expect("supervised stream");
            let pids = launcher.pids().unwrap();
            assert_eq!(pids.len(), 1);
            // If the guard discarded ChildStdin, cat would receive EOF and
            // run_turn would immediately yield a failure instead of waiting.
            assert!(
                tokio::time::timeout(Duration::from_millis(50), stream.next())
                    .await
                    .is_err()
            );
            assert_eq!(kill(pids[0], None), Ok(()));
            drop(stream);
            stopped(&pids).await;
        }
    })
    .await
    .expect("supervised spawn checks finish before deadline");
}

#[tokio::test]
async fn missing_smith_path_spawns_directly_and_warns_once() {
    let launcher = Launcher::with_script(
        "#!/bin/sh\nprintf '%s\\n' '{\"type\":\"result\",\"is_error\":false}'\n",
    );
    let backend = launcher.backend();
    let events = tokio::time::timeout(Duration::from_secs(10), async {
        let mut events = Vec::new();
        for turn in ["first", "second"] {
            let stream = backend
                .run_turn(ExternalTurnRequest {
                    input: UserInput::text("hello"),
                    resume: None,
                    turn: TurnId::new(turn),
                    cancel: Cancellation::new(),
                    capabilities: Default::default(),
                    bridge: None,
                    tools: ExternalToolDispatch::disabled(),
                })
                .await
                .unwrap();
            events.extend(stream.collect::<Vec<_>>().await);
        }
        events
    })
    .await
    .expect("direct turns finish before deadline");
    assert!(matches!(
        events.as_slice(),
        [ExternalAgentEvent::Completed, ExternalAgentEvent::Completed]
    ));
    assert_eq!(
        backend
            .missing_executable_warning_count
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert!(super::invocation::MISSING_EXECUTABLE_WARNING.contains("CLI may outlive Smith"));
}

#[derive(Debug)]
struct EndpointProbe;

#[async_trait::async_trait]
impl agent_runtime_core::tool::Tool for EndpointProbe {
    fn spec(&self) -> agent_runtime_core::tool::ToolSpec {
        agent_runtime_core::tool::ToolSpec::new(
            "endpoint_probe",
            "Endpoint startup probe",
            serde_json::json!({"type":"object"}),
            agent_runtime_core::tool::ToolEffects::read_only(),
        )
    }

    async fn invoke(
        &self,
        _: agent_runtime_core::tool::PreparedToolCall,
        _: &agent_runtime_core::tool::InvocationContext,
    ) -> Result<agent_runtime_core::tool::ToolOutcome, agent_runtime_core::error::RuntimeError>
    {
        panic!("binding failure must never dispatch a tool")
    }
}

fn endpoint_probe_runtime(
    backend: CliAgentBackend,
) -> (
    agent_runtime::runtime::Runtime,
    std::sync::Arc<agent_runtime::provider::fake::FakeProvider>,
) {
    use agent_runtime::provider::fake::FakeProvider;
    use agent_runtime::runtime::RuntimeBuilder;
    use agent_runtime_core::catalog::{ModelLimits, ResolvedModelProfile};
    use agent_runtime_core::provider::{Capabilities, ModelId};
    use std::sync::Arc;

    let provider = Arc::new(FakeProvider::new(
        "fake",
        Capabilities::basic_streaming(),
        vec![],
    ));
    let runtime = RuntimeBuilder::new(ModelId::new("fake"))
        .provider(provider.clone())
        .model_profile(ResolvedModelProfile::explicit(
            "fake",
            ModelId::new("fake"),
            ModelLimits::new(128_000, 128_000, 4_096),
        ))
        .tool(Arc::new(EndpointProbe))
        // This isolated startup fixture needs explicit typed-tool authority
        // before it can reach the endpoint path it is testing.
        .legacy_approval_authority()
        .external_agent(Arc::new(backend))
        .build()
        .unwrap();
    (runtime, provider)
}

#[tokio::test]
async fn missing_smith_executable_skips_endpoint_even_with_authorized_tools() {
    tokio::time::timeout(Duration::from_secs(10), async {
        use agent_runtime::runtime::StartSession;

        let launcher = Launcher::with_script(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$ARGV_FILE\"\nprintf '%s\\n' '{\"type\":\"result\",\"is_error\":false}'\n",
        );
        let recorded = launcher.directory.path().join("argv");
        let mut backend = launcher.backend();
        backend.settings.env.insert("ARGV_FILE".into(), recorded.display().to_string());
        // Binding here would fail. A host without Smith must not even try,
        // despite a non-empty authorized bridge view.
        backend.endpoint_base_override = Some(launcher.directory.path().join("missing-base"));
        let expected = backend.argv("hello", None).unwrap();
        let (runtime, provider) = endpoint_probe_runtime(backend);
        let session = runtime.start_session(StartSession::new()).await.unwrap();
        session.run(UserInput::text("hello")).await.unwrap();
        let argv = std::fs::read_to_string(recorded).expect("direct CLI ran");
        assert_eq!(argv.lines().collect::<Vec<_>>(), expected);
        assert!(provider.requests().is_empty());
        session.shutdown().await.unwrap();
    }).await.expect("unbridged turn finishes before deadline");
}

#[tokio::test]
async fn endpoint_bind_failure_fails_turn_before_cli_start() {
    tokio::time::timeout(Duration::from_secs(10), async {
        use agent_runtime::runtime::StartSession;

        let launcher = Launcher::with_script("#!/bin/sh\nprintf '%s\\n' \"$$\" > \"$PID_FILE\"\nprintf started > \"$START_MARKER\"\n");
        let marker = launcher.directory.path().join("cli-started");
        let base = launcher.directory.path().join("x".repeat(120));
        std::fs::create_dir(&base).unwrap();
        let supervisor = launcher.directory.path().join("test-supervisor");
        // A usable supervisor makes the start marker meaningful: if run_turn
        // were to launch before binding, the fake CLI could actually start.
        std::fs::write(&supervisor, "#!/bin/sh\nshift\nshift\nexec \"$@\"\n").unwrap();
        std::fs::set_permissions(&supervisor, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut backend = launcher.backend();
        backend.settings.relay_executable = Some(supervisor);
        backend
            .settings
            .env
            .insert("START_MARKER".into(), marker.display().to_string());
        backend.endpoint_base_override = Some(base.clone());
        let (runtime, provider) = endpoint_probe_runtime(backend);
        let session = runtime.start_session(StartSession::new()).await.unwrap();
        let mut events = session.subscribe();
        // External backend startup errors are projected as TurnFinish::Failed;
        // inspect the canonical event instead of requiring run() to Err.
        let outcome = session.run(UserInput::text("bind endpoint")).await;
        let mut reason = outcome.err().map(|error| error.to_string());
        let mut failed = false;
        while let Some(event) = events.next().await {
            match &event.payload {
                agent_runtime_core::event::RuntimeEvent::Error { error } => {
                    reason = Some(error.to_string())
                }
                agent_runtime_core::event::RuntimeEvent::TurnCompleted { finish, .. } => {
                    failed = matches!(finish, agent_runtime_core::event::TurnFinish::Failed);
                    break;
                }
                _ => {}
            }
        }
        assert!(failed, "endpoint failure must fail the turn");
        let reason = reason.expect("endpoint failure reason");
        assert!(reason.contains("bridge socket"), "{reason}");
        assert!(reason.contains("sun_path"), "{reason}");
        assert!(reason.contains(base.to_str().unwrap()), "{reason}");
        assert!(
            !marker.exists(),
            "CLI started before endpoint binding succeeded"
        );
        assert!(provider.requests().is_empty());
        assert_eq!(
            std::fs::read_dir(base).unwrap().count(),
            0,
            "failed endpoint leaked its directory"
        );
        session.shutdown().await.unwrap();
    })
    .await
    .expect("endpoint failure test deadline");
}
