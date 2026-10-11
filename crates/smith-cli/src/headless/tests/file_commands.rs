//! File-command preflight and provider-bound prompt fixtures.

use std::path::PathBuf;

use smith_config::resolve::Resolution;
use smith_config::trust::{Executable, ExecutableKind, TrustDecision, TrustStore};

use super::*;

const FORMATS: [OutputFormat; 3] = [
    OutputFormat::Text,
    OutputFormat::Json,
    OutputFormat::StreamJson,
];

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
    _home: tempfile::TempDir,
    project: tempfile::TempDir,
    resolution: Resolution,
    provider: Arc<FakeProvider>,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("configuration directory");
        std::fs::write(config_dir.join("config.toml"), CONFIG).expect("configuration");
        let resolution = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved configuration");
        Self {
            _home: home,
            project,
            resolution,
            provider: Arc::new(FakeProvider::text_reply("done")),
        }
    }

    fn user_command(&self) {
        let commands = self.resolution.layout.user_dir.join("commands");
        std::fs::create_dir_all(&commands).expect("user commands directory");
        std::fs::write(commands.join("audit.md"), "Audit $ARGUMENTS for bugs.")
            .expect("user audit command");
    }

    fn project_command(&self) -> PathBuf {
        let commands = self.project.path().join(".smith/commands");
        std::fs::create_dir_all(&commands).expect("project commands directory");
        let path = commands.join("audit.md");
        std::fs::write(&path, "Audit $ARGUMENTS for bugs.").expect("project audit command");
        path
    }

    async fn run(
        &self,
        prompt: &str,
        format: OutputFormat,
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
    ) -> Result<Outcome> {
        let prompt = prepare_prompt(
            prompt,
            &self.resolution.layout.user_dir,
            self.project.path(),
        )?;
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(ProjectWorkspace::new(self.project.path())?)),
            approval: Some(Arc::new(HeadlessApproval::new())),
            provider: Some(self.provider.clone() as Arc<dyn Provider>),
            ..RuntimeRequest::new(self.resolution.config.clone(), HostSurface::Headless)
        };
        let host = Box::pin(smith_runtime::host::start(host_request(
            runtime,
            self.project.path(),
        )))
        .await?;
        Box::pin(run_with_io(
            &host,
            prompt,
            format,
            HeadlessBrokers::default(),
            BackgroundExit::Error,
            stdout,
            stderr,
        ))
        .await
    }
}

fn assert_success_stderr(format: OutputFormat, stderr: &[u8]) {
    let diagnostic = String::from_utf8_lossy(stderr);
    match format {
        OutputFormat::Text => {
            assert!(diagnostic.contains("smith: parent: idle\n"), "{diagnostic}");
            assert!(
                diagnostic.contains("smith: provider attempts: 1 committed · 0 discarded\n"),
                "{diagnostic}"
            );
            assert!(
                diagnostic.contains("smith: activation epoch "),
                "{diagnostic}"
            );
            assert!(
                diagnostic.contains("smith: resume checkpoint: saved "),
                "{diagnostic}"
            );
            assert_eq!(diagnostic.lines().count(), 4, "{diagnostic}");
        }
        OutputFormat::Json | OutputFormat::StreamJson => {
            assert!(stderr.is_empty(), "{diagnostic}");
        }
    }
}

#[tokio::test]
async fn file_commands_send_expanded_or_literal_prompts_to_the_provider() {
    for format in FORMATS {
        for (prompt, expected) in [
            ("/audit src/lib.rs", "Audit src/lib.rs for bugs."),
            ("/usr/bin/env is missing", "/usr/bin/env is missing"),
            ("//audit the plan", "/audit the plan"),
            ("/model", "/model"),
        ] {
            let fixture = Fixture::new();
            fixture.user_command();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let outcome = tokio::time::timeout(
                HEADLESS_TEST_WATCHDOG,
                Box::pin(fixture.run(prompt, format, &mut stdout, &mut stderr)),
            )
            .await
            .expect("headless watchdog")
            .expect("headless outcome");
            assert_eq!(outcome.exit_code, 0);
            assert_success_stderr(format, &stderr);
            let requests = fixture.provider.requests();
            assert_eq!(requests.len(), 1);
            let user_messages = requests[0]
                .messages
                .iter()
                .filter(|message| message.role == Role::User)
                .map(|message| message.joined_text())
                .collect::<Vec<_>>();
            assert_eq!(user_messages, vec![expected.to_owned()], "{prompt}");
        }
    }
}

#[tokio::test]
async fn file_commands_expand_prompts_read_from_stdin() {
    let fixture = Fixture::new();
    fixture.user_command();
    let prompt =
        crate::runtime_host::read_prompt("/audit src/lib.rs".as_bytes()).expect("stdin prompt");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let outcome = tokio::time::timeout(
        HEADLESS_TEST_WATCHDOG,
        Box::pin(fixture.run(&prompt, OutputFormat::Text, &mut stdout, &mut stderr)),
    )
    .await
    .expect("headless watchdog")
    .expect("headless outcome");
    assert_eq!(outcome.exit_code, 0);
    assert_success_stderr(OutputFormat::Text, &stderr);
    let requests = fixture.provider.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].messages.iter().any(|message| {
        message.role == Role::User && message.joined_text() == "Audit src/lib.rs for bugs."
    }));
}

#[tokio::test]
async fn file_commands_refuse_untrusted_and_changed_project_commands_before_host_start() {
    for format in FORMATS {
        for changed in [false, true] {
            let fixture = Fixture::new();
            let path = fixture.project_command();
            if changed {
                let mut trust =
                    TrustStore::open(&fixture.resolution.layout.user_dir).expect("trust store");
                let executable = Executable::from_file(
                    fixture.project.path(),
                    ExecutableKind::SlashCommand,
                    &path,
                )
                .expect("command identity");
                trust
                    .record(fixture.project.path(), &executable, TrustDecision::Allow)
                    .expect("approved original content");
                std::fs::write(&path, "Changed audit instructions.").expect("edited command");
            }
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let error =
                Box::pin(fixture.run("/audit src/lib.rs", format, &mut stdout, &mut stderr))
                    .await
                    .expect_err("unapproved content must fail closed");
            let exit = crate::report_failure(&error, &mut stderr);
            assert_eq!(exit, std::process::ExitCode::FAILURE);
            assert_ne!(exit, std::process::ExitCode::SUCCESS);
            assert!(stdout.is_empty(), "startup refusal emitted protocol output");
            assert_eq!(fixture.provider.requests().len(), 0);
            let diagnostic = String::from_utf8(stderr).expect("UTF-8 diagnostic");
            let reason = if changed {
                "content changed"
            } else {
                "needs approval"
            };
            assert_eq!(
                diagnostic,
                format!("smith: `/audit` cannot run: {reason}: /commands trust audit\n")
            );
        }
    }
}
