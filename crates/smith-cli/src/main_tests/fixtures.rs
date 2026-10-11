use super::mcp::{mcp_context, mcp_project};
use super::skills::{SKILL_BODY, write_skill_body};
use super::*;
use smith_client::status::{PriceReference, PriceTable};

// Before-refactor recordings. The headless tests use the same comparison and normalization.
pub(crate) mod fixture_support;
mod tests;
mod view;

use view::{fixture_raw_and_view, fixture_record, fixture_screen};

async fn fixture_local_host(
    home: &std::path::Path,
    project: &std::path::Path,
    provider: Arc<dyn agent_runtime_core::provider::Provider>,
    sources: smith_runtime::skills::SmithSkillSources,
) -> HostSession {
    let config = resolve(
        &ResolveRequest::new(project)
            .with_known_modules(crate::modules::known_modules())
            .with_home_dir(home),
    )
    .expect("resolution")
    .config;
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(ProjectWorkspace::new(project).expect("workspace"))),
        approval: Some(Arc::new(agent_runtime_core::approval::AllowAll)),
        provider: Some(provider),
        skills: sources,
        clock: Some(Arc::new(fixture_support::FixedClock)),
        ..RuntimeRequest::new(config, HostSurface::Terminal)
    };
    let host = Box::pin(smith_runtime::host::start(
        HostSessionRequest::new(runtime, project).checkpoint_keys(Arc::new(TestCheckpointKeys)),
    ))
    .await
    .expect("host");
    host.set_goal_continuation_enabled(false);
    host
}

fn fixture_local_app(planned: bool, usage: bool) -> App {
    let mut app = App::new("example-model", "<PROJECT>");
    if planned {
        let totals = BTreeMap::from([
            (
                agent_runtime_core::manifest::SegmentKind::new("system_instruction"),
                200,
            ),
            (
                agent_runtime_core::manifest::SegmentKind::new("tool_schema"),
                500,
            ),
            (
                agent_runtime_core::manifest::SegmentKind::new("history"),
                1_300,
            ),
        ]);
        app.status.record_context_plan(ContextPlanUpdate {
            fingerprint: "context-test",
            cache_fingerprint: "cache-test",
            input_tokens: 2_000,
            input_budget_tokens: 123_904,
            reserved_tokens: 4_096,
            segment_count: 3,
            totals: &totals,
            confidence: EstimationConfidence::Estimated,
        });
        app.status.record_registry("registry-test", 6);
        app.status.record_scoped_view("view-test", 4);
        app.status.record_retrieval(
            "resolver-test",
            vec!["tool:read".into(), "tool:search".into()],
        );
        app.status.record_activation(1, vec!["tool:read".into()]);
        app.status.record_compaction(250);
    }
    if usage {
        // These usage fixtures represent one user prompt, not provider attempts.
        app.status.record_user_turn();
        app.status.record_usage(
            &agent_runtime_core::usage::UsageDelta::new()
                .with(CounterKind::InputUncached, 1_000)
                .with(CounterKind::InputCached, 800)
                .with(CounterKind::Output, 120),
        );
    }
    app
}

async fn fixture_command(
    name: &str,
    command: &str,
    mut app: App,
    host: &HostSession,
    project: &std::path::Path,
    mcp: Option<&crate::mcp::McpContext>,
    skills: &crate::skills::SkillContext,
) {
    let parsed = smith_client::commands::parse(command).expect("fixture command parses");
    app.composer.replace(command.to_owned());
    let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    match action {
        Some(Action::Command(actual)) => {
            assert_eq!(
                smith_client::commands::Command::Host(actual.clone()),
                parsed.command
            );
            Box::pin(handle_local_command(
                &mut app, host, project, mcp, skills, actual,
            ))
            .await;
        }
        None => {}
        other => panic!("fixture needs a local result, got {other:?}"),
    }
    let mut normalizer = fixture_support::Normalizer::new(
        host.paths()
            .expect("paths")
            .directory()
            .parent()
            .and_then(std::path::Path::parent)
            .and_then(std::path::Path::parent)
            .expect("isolated home"),
        project,
    );
    normalizer.session(host.session().id().as_str());
    normalizer.profile(&host.runtime().policy().agent_profile_revision);
    if let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
    {
        for (index, child) in coordinator.list().iter().enumerate() {
            normalizer.child_session(child.session.as_str(), index + 1);
        }
    }
    fixture_record(name, &app, &mut normalizer);
}

// inside a test future on the default Rust test-thread stack.
struct FixtureLocal {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
    host: HostSession,
    skills: crate::skills::SkillContext,
}

impl FixtureLocal {
    async fn command(&self, name: &str, command: &str, app: App) {
        Box::pin(fixture_command(
            name,
            command,
            app,
            &self.host,
            self.project.path(),
            None,
            &self.skills,
        ))
        .await;
    }

    async fn commands(&self, cases: &[(&str, &str)]) {
        for (name, command) in cases {
            Box::pin(self.command(name, command, fixture_local_app(false, false))).await;
        }
    }

    async fn replay(&self, command: &str) {
        let mut app = fixture_local_app(false, false);
        let smith_client::commands::Command::Host(command) = smith_client::commands::parse(command)
            .expect("replayed command")
            .command
        else {
            panic!("replay requires a host command");
        };
        Box::pin(handle_local_command(
            &mut app,
            &self.host,
            self.project.path(),
            None,
            &self.skills,
            command,
        ))
        .await;
    }

    // Ephemeral cases intentionally call the handler directly.
    // They have no persisted session paths for fixture_command to inspect.
    async fn direct_commands(&self, cases: &[(&str, &str)]) {
        for (name, command) in cases {
            let mut app = fixture_local_app(false, false);
            match smith_client::commands::parse(command)
                .expect("direct command")
                .command
            {
                smith_client::commands::Command::Host(command) => {
                    Box::pin(handle_local_command(
                        &mut app,
                        &self.host,
                        self.project.path(),
                        None,
                        &self.skills,
                        command,
                    ))
                    .await;
                }
                other => panic!("fixture requires a host command: {other:?}"),
            }
            let mut normalizer =
                fixture_support::Normalizer::new(self.home.path(), self.project.path());
            normalizer.session(self.host.session().id().as_str());
            normalizer.profile(&self.host.runtime().policy().agent_profile_revision);
            fixture_record(name, &app, &mut normalizer);
        }
    }

    async fn shutdown(self) {
        Box::pin(self.host.shutdown()).await.expect("shutdown");
    }
}

async fn fixture_local_base() -> FixtureLocal {
    use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, usage_event};
    use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
    let (home, project) = mcp_project("");
    let (sources, skills) = crate::skills::SkillContext::compose(
        smith_runtime::skills::SmithSkillSources::new(),
        &home.path().join(".smith"),
        project.path(),
    )
    .expect("skills");
    let host = Box::pin(fixture_local_host(
        home.path(),
        project.path(),
        Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "done".into(),
                    },
                    usage_event(9, 3),
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "child done".into(),
                    },
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
            ],
        )),
        sources,
    ))
    .await;
    FixtureLocal {
        home,
        project,
        host,
        skills,
    }
}

async fn fixture_local_populated() -> FixtureLocal {
    use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
    use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
    let (home, project) = mcp_project("");
    // The existing roots and skill helpers keep config and trust isolated.
    write_skill_body(&home.path().join(".smith"), "rust-review", SKILL_BODY);
    write_skill_body(&home.path().join(".smith"), "broken", "no frontmatter\n");
    write_skill_body(&project.path().join(".smith"), "deploy", SKILL_BODY);
    let (sources, skills) = crate::skills::SkillContext::compose(
        smith_runtime::built_in_skills::built_in_sources(),
        &home.path().join(".smith"),
        project.path(),
    )
    .expect("populated skills");
    std::fs::write(project.path().join(".smith/config.toml"), format!(
        "{LOCAL_COMMAND_CONFIG}\n[mcp.servers.docs]\ncommand = \"docs-mcp\"\nargs = [\"--stdio\"]\nenv = {{ DOCS_TOKEN = \"keychain:smith/docs\" }}\n[mcp.servers.off]\ncommand = \"off-mcp\"\nenabled = false\n"
    )).expect("MCP config");
    let mut edit = tool_call_fragments(
        0,
        "call-edit",
        "edit",
        r#"{"path":"tracked.txt","old_string":"before","new_string":"after"}"#,
    );
    edit.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(edit),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "edited".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    );
    let host = Box::pin(fixture_local_host(
        home.path(),
        project.path(),
        Arc::new(provider),
        sources,
    ))
    .await;
    FixtureLocal {
        home,
        project,
        host,
        skills,
    }
}

const FIXTURE_GOAL_COMMANDS: &[(&str, &str)] = &[
    ("goal-create", "/goal finish the fixture"),
    ("goal-active", "/goal"),
    ("goal-create-conflict", "/goal a conflicting objective"),
    ("goal-edit", "/goal edit revised fixture objective"),
    ("goal-budget", "/goal budget 100"),
    ("goal-budget-none", "/goal budget none"),
    ("goal-pause", "/goal pause"),
    ("goal-paused", "/goal"),
    ("goal-resume", "/goal resume"),
];
const FIXTURE_GOAL_FINISH_COMMANDS: &[(&str, &str)] =
    &[("goal-usage", "/goal"), ("goal-clear", "/goal clear")];

async fn fixture_local_goal_turn(fixture: &FixtureLocal, record: bool) {
    for (name, command) in FIXTURE_GOAL_COMMANDS {
        if record {
            Box::pin(fixture.command(name, command, fixture_local_app(false, false))).await;
        } else {
            Box::pin(fixture.replay(command)).await;
        }
    }
    Box::pin(fixture.host.session().run(UserInput::text("a fixed turn")))
        .await
        .expect("turn");
    for (name, command) in FIXTURE_GOAL_FINISH_COMMANDS {
        if record {
            Box::pin(fixture.command(name, command, fixture_local_app(false, false))).await;
        } else {
            Box::pin(fixture.replay(command)).await;
        }
    }
}

fn fixture_local_git_setup(fixture: &FixtureLocal) {
    git(
        fixture.project.path(),
        &[
            "init",
            "--initial-branch=fixture",
            "--template=",
            "--object-format=sha1",
        ],
    );
    for arguments in [
        ["config", "user.email", "smith@example.invalid"],
        ["config", "user.name", "Smith Test"],
        ["config", "commit.gpgsign", "false"],
        ["config", "core.autocrlf", "false"],
        ["config", "core.attributesFile", "/dev/null"],
        ["config", "diff.algorithm", "myers"],
        ["config", "diff.context", "3"],
        ["config", "diff.noprefix", "false"],
        ["config", "diff.mnemonicPrefix", "false"],
        ["config", "diff.indentHeuristic", "false"],
        ["config", "color.ui", "false"],
    ] {
        git(fixture.project.path(), &arguments);
    }
    std::fs::write(fixture.project.path().join(".gitignore"), ".smith/\n").expect("ignore state");
    std::fs::write(fixture.project.path().join("tracked.txt"), "before\n").expect("tracked");
    git(
        fixture.project.path(),
        &["add", ".gitignore", "tracked.txt"],
    );
    git(
        fixture.project.path(),
        &["-c", "core.hooksPath=/dev/null", "commit", "-m", "initial"],
    );
}

async fn fixture_local_git_edit(fixture: &FixtureLocal) {
    Box::pin(
        fixture
            .host
            .session()
            .run(UserInput::text("edit tracked.txt")),
    )
    .await
    .expect("scripted edit");
    assert_eq!(
        std::fs::read_to_string(fixture.project.path().join("tracked.txt")).expect("edited"),
        "after\n"
    );
    std::fs::write(fixture.project.path().join("untracked.txt"), "new file\n").expect("untracked");
}

const FIXTURE_GIT_DIRTY_COMMANDS: &[(&str, &str)] = &[
    ("status-dirty", "/status"),
    ("diff-dirty", "/diff"),
    ("diff-unstaged", "/diff unstaged"),
    ("diff-staged-empty", "/diff staged"),
    ("diff-untracked", "/diff untracked"),
    ("diff-file", "/diff tracked.txt"),
    ("diff-hunk", "/diff tracked.txt#1"),
    ("diff-last-turn", "/diff last-turn"),
    ("review-dirty", "/review"),
    ("review-file", "/review tracked.txt"),
    ("undo-preview", "/undo"),
    ("revert-file", "/revert tracked.txt"),
    ("revert-hunk", "/revert tracked.txt#1"),
    ("revert-untracked", "/revert untracked.txt"),
    ("revert-missing", "/revert missing.txt"),
];

#[derive(Clone, Copy)]
enum FixtureGitGroup {
    DiffReview,
    Recovery,
    Timeline,
}

// Replay every preview in the original order, but let each test record only its
// own group's paths. In particular, timeline-recovery includes both undo
// previews and the three successful revert previews preceding it.
async fn fixture_local_git_dirty(fixture: &FixtureLocal, group: FixtureGitGroup) {
    for (name, command) in FIXTURE_GIT_DIRTY_COMMANDS {
        let recovery = command.starts_with("/undo") || command.starts_with("/revert");
        let record = match group {
            FixtureGitGroup::DiffReview => !recovery,
            FixtureGitGroup::Recovery => recovery,
            FixtureGitGroup::Timeline => false,
        };
        if record {
            Box::pin(fixture.command(name, command, fixture_local_app(false, false))).await;
        } else {
            Box::pin(fixture.replay(command)).await;
        }
    }
}
