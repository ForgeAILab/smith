//! The shipped Claude Code plugin through path and local-git installs.

use std::path::Path;
use std::sync::Arc;

use agent_runtime::provider::fake::{FakeProvider, ScriptedStream};
use agent_runtime::runtime::StartSession;
use agent_runtime_core::content::UserInput;
use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};

use super::Fixture;

pub(super) fn git_available() -> bool {
    if std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        true
    } else {
        eprintln!("skipping local-git plugin fixture: git is unavailable");
        false
    }
}

pub(super) fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".git" {
            continue;
        }
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

pub(super) fn init_repo(root: &Path) {
    for args in [
        vec!["init"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Plugin Fixture",
            "-c",
            "user.email=fixture@example.test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "plugin fixture",
        ],
    ] {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(&args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

async fn exercise(use_git: bool) {
    if use_git && !git_available() {
        return;
    }
    let fixture = Fixture::new(true);
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/claude-code/plugins/smith")
        .canonicalize()
        .unwrap();
    let repository = tempfile::tempdir().unwrap();
    let source_text = if use_git {
        let root = repository.path().canonicalize().unwrap();
        copy_tree(&source, &root);
        init_repo(&root);
        format!("file://{}", root.display())
    } else {
        source.display().to_string()
    };
    let mut output = Vec::new();
    crate::plugin_command::run_with(
        crate::cli::PluginAction::Add {
            source: source_text,
            yes: true,
        },
        smith_config::resolve::ResolveRequest::new(&fixture.project).with_home_dir(&fixture.home),
        false,
        &mut std::io::Cursor::new(""),
        &mut output,
    )
    .unwrap();
    let config = fixture.config(None);
    let installed = crate::plugins::open_store(&fixture.user)
        .list()
        .unwrap()
        .into_iter()
        .find(|installed| installed.name == "smith")
        .unwrap();
    assert_eq!(installed.resolved_commit.is_some(), use_git);
    assert!(!installed.install_directory.starts_with(&source));
    let expanded = crate::headless::prepare_prompt_with_config(
        "/smith:profiles",
        &fixture.user,
        &fixture.project,
        &config,
    )
    .unwrap();
    assert!(
        expanded.contains("Present the table above to the user as-is."),
        "{expanded}"
    );
    assert!(
        expanded.contains(
            &installed
                .install_directory
                .join("scripts/smith-profiles")
                .display()
                .to_string()
        ),
        "{expanded}"
    );
    assert!(
        expanded.contains("!`"),
        "shell-looking template text stays ordinary prompt content"
    );
    assert!(!expanded.contains("${CLAUDE_PLUGIN_ROOT}"));
    let mut request = fixture.request(config);
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(vec![
                ProviderStreamEvent::ToolCallDelta {
                    index: 0,
                    id: Some("activate-shipped-skill".into()),
                    name: Some("registry.activate".into()),
                    arguments_fragment: r#"{"ids":["skill:smith:smith-profiles"]}"#.into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::ToolCalls,
                },
            ]),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta { text: "ok".into() },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    request.provider = Some(provider.clone());
    let runtime = smith_runtime::factory::build(
        smith_runtime::harness::resolve(smith_runtime::harness::HarnessSpec::trusted(request))
            .unwrap(),
    )
    .await
    .unwrap();
    assert!(
        runtime
            .skill_index()
            .iter()
            .any(|skill| skill.name() == "smith:smith-profiles" && skill.activatable)
    );
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session.run(UserInput::text(&expanded)).await.unwrap();
    session.shutdown().await.unwrap();
    let requests = format!("{:?}", provider.requests());
    assert!(
        requests.contains("Present the table above to the user as-is."),
        "{requests}"
    );
    assert!(
        requests.contains("# Choosing a Smith profile"),
        "{requests}"
    );
    assert!(
        requests.contains(
            &installed
                .install_directory
                .join("scripts/smith-profiles")
                .display()
                .to_string()
        )
    );
    assert!(!requests.contains("${CLAUDE_PLUGIN_ROOT}"));
}

#[tokio::test]
async fn shipped_plugin_path_install_runs_command_and_activates_skill() {
    exercise(false).await;
}

#[tokio::test]
async fn shipped_plugin_git_install_runs_command_and_activates_skill() {
    exercise(true).await;
}
