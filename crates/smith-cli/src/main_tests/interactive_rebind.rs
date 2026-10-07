use super::*;
use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, usage_event};
use agent_runtime_core::clock::Deadline;
use agent_runtime_core::ids::{AttemptId, EventId, RequestId};
use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
use smith_client::NoticeKind;
use smith_client::commands::SessionControl;
use smith_config::resolve::Overrides;
use smith_tui::app::{ChildState, Overlay};
use smith_tui::questionnaire::{QuestionnaireChoice, QuestionnaireForm, QuestionnaireQuestion};
use smith_tui::status::Activity;

use crate::tui_driver::{
    InteractiveApp, InteractiveExit, InteractiveResources, PresentationOptions,
    prepare_interactive_app, reconfigure_exit,
};

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
    catalog: Arc<smith_config::catalog::CatalogSnapshot>,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
        let mut config = String::from(
            r#"
default_profile = "dev"
profile_order = ["dev", "review"]

[profiles.dev]
provider = "local"
model = "example-model"
use = ["main"]

[profiles.review]
extends = "dev"
use = ["main"]

[providers.local]
kind = "fake"
"#,
        );
        for model in ["example-model", "next-model"] {
            config.push_str(&format!(
                r#"
[models."local/{model}"]
max_output_tokens = 4096
default_context_window = "128k"

[models."local/{model}".context_windows."128k"]
context_tokens = 128000
max_input_tokens = 124000

[models."local/{model}".context_windows."256k"]
context_tokens = 256000
max_input_tokens = 252000

[models."local/{model}".reasoning]
toggle = true
mandatory = false
efforts = ["none", "low", "high"]
default_enabled = true
default_effort = "low"
dialect = "openai-effort"
"#,
            ));
        }
        std::fs::write(project.path().join(".smith/config.toml"), config).expect("config");
        Self {
            home,
            project,
            catalog: Arc::new(
                serde_json::from_str(smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED)
                    .expect("embedded catalog"),
            ),
        }
    }

    async fn start(
        &self,
        resume: Option<SessionId>,
        overrides: Overrides,
    ) -> (HostSession, InteractiveResources) {
        let mut resolution = resolve(
            &ResolveRequest::new(self.project.path())
                .with_home_dir(self.home.path())
                .with_cli(overrides),
        )
        .expect("resolution");
        resolution.config.persistence.enabled.value = true;
        let inventory =
            local_inventory_with_catalog(&resolution, AVAILABLE_ADAPTER_KINDS, Some(&self.catalog))
                .expect("inventory");
        let agents = resolution.config.agent.clone();
        let (sources, skills) = crate::skills::SkillContext::compose(
            smith_runtime::skills::SmithSkillSources::new(),
            &self.home.path().join(".smith"),
            self.project.path(),
        )
        .expect("skills");
        let provider = Arc::new(FakeProvider::new(
            &resolution.config.model.value,
            Capabilities::basic_streaming(),
            vec![ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "finished answer".to_owned(),
                },
                usage_event(400, 20),
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])],
        ));
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(self.project.path()).expect("workspace"),
            )),
            approval: Some(Arc::new(agent_runtime_core::approval::AllowAll)),
            provider: Some(provider),
            skills: sources,
            clock: Some(Arc::new(fixture_support::FixedClock)),
            ..RuntimeRequest::new(resolution.config, HostSurface::Terminal)
        };
        let mut request = HostSessionRequest::new(runtime, self.project.path())
            .checkpoint_keys(Arc::new(TestCheckpointKeys))
            .reasoning_reset(true, true)
            .context_window_reset(true);
        if let Some(session) = resume {
            request = request.resume(session);
        }
        let host = Box::pin(smith_runtime::host::start(request))
            .await
            .expect("host");
        host.set_goal_continuation_enabled(false);
        (
            host,
            InteractiveResources {
                inventory,
                agents,
                sessions: Vec::new(),
                credential_pool: None,
                catalog: self.catalog.clone(),
                mcp: None,
                skills: Arc::new(skills),
                capability_denials: Vec::new(),
            },
        )
    }

    async fn app(
        &self,
        host: &HostSession,
        resources: &InteractiveResources,
        previous: Option<InteractiveApp>,
        reasoning_notice: Option<String>,
    ) -> InteractiveApp {
        Box::pin(prepare_interactive_app(
            host,
            self.project.path(),
            resources,
            &PresentationOptions {
                no_color: true,
                no_motion: true,
                reasoning_notice,
                host_notice: None,
                cache_miss_notices: true,
            },
            previous,
        ))
        .await
    }
}

async fn finish_turn(host: &HostSession, prompt: &str) {
    let turn = host.session().send(UserInput::text(prompt)).expect("turn");
    tokio::time::timeout(Duration::from_secs(10), Box::pin(turn.completed()))
        .await
        .expect("turn completed");
}

fn event(payload: RuntimeEvent) -> EventEnvelope {
    EventEnvelope::new(
        1,
        EventId::new("event-1"),
        SessionId::new("session-1"),
        None,
        Timestamp::ZERO,
        payload,
    )
}

fn assert_notice(app: &App, source: &str, expected: &str) {
    assert!(
        matches!(app.transcript.blocks().last(), Some(Block::Notice { kind: actual, text })
            if actual.label() == source && text == expected),
        "{:?}",
        app.transcript.blocks().last(),
    );
}

#[tokio::test]
async fn model_rebind_keeps_blocks_folding_scroll_composer_and_history() {
    let fixture = Fixture::new();
    let (host, resources) = Box::pin(fixture.start(None, Overrides::default())).await;
    Box::pin(finish_turn(&host, "earlier prompt")).await;
    let mut previous = Box::pin(fixture.app(&host, &resources, None, None)).await;
    previous.app.composer.replace("earlier prompt");
    previous.app.composer.record_current();
    previous.app.composer.replace("unfinished draft");
    previous
        .app
        .transcript
        .push_tool_call("kept-tool", "read", None, &["path".to_owned()]);
    previous.app.set_tool_result_preview(
        "kept-tool",
        "first result line\nsecond result line\nthird result line",
    );
    previous
        .app
        .transcript
        .complete_tool_call("kept-tool", smith_tui::transcript::ToolStatus::Ok);
    Box::pin(handle_local_command(
        &mut previous.app,
        &host,
        fixture.project.path(),
        None,
        &resources.skills,
        HostCommand::Status,
    ))
    .await;
    previous.app.following = false;
    previous.app.scroll_back = 7;
    previous.app.open_overlay(Overlay::Shortcuts);
    let blocks = format!("{:?}", previous.app.transcript.blocks());
    let count = previous.app.transcript.len();
    let session = host.session().id().clone();
    Box::pin(host.shutdown()).await.expect("shutdown");

    let (host, resources) = Box::pin(fixture.start(
        Some(session),
        Overrides {
            model: Some("next-model".to_owned()),
            ..Overrides::default()
        },
    ))
    .await;
    let mut rebound = Box::pin(fixture.app(&host, &resources, Some(previous), None)).await;
    assert_eq!(
        format!("{:?}", &rebound.app.transcript.blocks()[..count]),
        blocks
    );
    assert_eq!(rebound.app.transcript.len(), count + 1);
    assert_notice(
        &rebound.app,
        "provider",
        "changed · local/example-model → local/next-model · prior cache not transferable",
    );
    assert_eq!(rebound.app.status.model, "next-model");
    assert!(!rebound.app.work_details, "folded detail stays folded");
    assert!(!rebound.app.following);
    assert_eq!(rebound.app.scroll_back, 7);
    assert_eq!(rebound.app.composer.text(), "unfinished draft");
    assert!(rebound.app.overlay.is_none());
    assert!(!rebound.app.is_busy());
    rebound.app.composer.clear();
    assert!(rebound.app.composer.recall_previous());
    assert_eq!(rebound.app.composer.text(), "earlier prompt");
    rebound.app.work_details = true;
    let rebound = Box::pin(fixture.app(&host, &resources, Some(rebound), None)).await;
    assert!(rebound.app.work_details, "expanded detail stays expanded");
    assert_eq!(
        rebound.app.transcript.len(),
        count + 1,
        "no repeated model notice"
    );
    Box::pin(host.shutdown()).await.expect("shutdown");
}

#[tokio::test]
async fn rebound_status_equals_fresh_seed_and_does_not_merge_live_status() {
    let fixture = Fixture::new();
    let (host, resources) = Box::pin(fixture.start(None, Overrides::default())).await;
    Box::pin(finish_turn(&host, "accounted turn")).await;
    let mut previous = Box::pin(fixture.app(&host, &resources, None, None)).await;
    previous.app.status.record_registry("stale-registry", 99);
    previous.app.status.record_usage(
        &agent_runtime_core::usage::UsageDelta::new().with(CounterKind::InputUncached, 9999),
    );
    previous.app.status.activity = Activity::Ended;
    let session = host.session().id().clone();
    Box::pin(host.shutdown()).await.expect("shutdown");
    let (host, resources) = Box::pin(fixture.start(
        Some(session),
        Overrides {
            model: Some("next-model".to_owned()),
            ..Overrides::default()
        },
    ))
    .await;
    let rebound = Box::pin(fixture.app(&host, &resources, Some(previous), None)).await;
    let fresh = Box::pin(fixture.app(&host, &resources, None, None)).await;
    assert_eq!(
        format!("{:?}", rebound.app.status),
        format!("{:?}", fresh.app.status)
    );
    assert_eq!(rebound.app.resources, fresh.app.resources);
    assert_eq!(rebound.app.children, fresh.app.children);
    assert_eq!(rebound.app.status.session_usage().turns, 1);
    assert_eq!(rebound.app.status.activity, Activity::Idle);
    Box::pin(host.shutdown()).await.expect("shutdown");
}

#[tokio::test]
async fn tab_profile_rebind_keeps_transcript_and_scroll() {
    let fixture = Fixture::new();
    let (host, resources) = Box::pin(fixture.start(None, Overrides::default())).await;
    let mut previous = Box::pin(fixture.app(&host, &resources, None, None)).await;
    previous
        .app
        .transcript
        .push_notice(NoticeKind::Local, "keep this result");
    previous.app.following = false;
    previous.app.scroll_back = 3;
    let Some(Action::Reconfigure(SessionControl::Reconfigure(command))) = previous
        .app
        .on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
    else {
        panic!("Tab must cycle the profile");
    };
    assert_eq!(command, SelectionCommand::Profile("review".to_owned()));
    let mut selection = Selection::default();
    let mut resume = None;
    apply_palette_command(
        &mut selection,
        &mut resume,
        host.session().id().as_str().to_owned(),
        command,
    );
    Box::pin(host.shutdown()).await.expect("shutdown");
    let (host, resources) = Box::pin(fixture.start(
        resume.map(SessionId::new),
        Overrides {
            profile: selection.profile,
            ..Overrides::default()
        },
    ))
    .await;
    let rebound = Box::pin(fixture.app(&host, &resources, Some(previous), None)).await;
    assert_eq!(rebound.app.transcript.len(), 2);
    assert!(
        matches!(&rebound.app.transcript.blocks()[0], Block::Notice { text, .. } if text == "keep this result")
    );
    assert_notice(&rebound.app, "profile", "changed · dev → review");
    assert!(!rebound.app.following);
    assert_eq!(rebound.app.scroll_back, 3);
    Box::pin(host.shutdown()).await.expect("shutdown");
}

#[tokio::test]
async fn resume_other_session_replaces_transcript_and_keeps_history() {
    let fixture = Fixture::new();
    let (host, _) = Box::pin(fixture.start(None, Overrides::default())).await;
    Box::pin(finish_turn(&host, "other session prompt")).await;
    let other = host.session().id().clone();
    Box::pin(host.shutdown()).await.expect("shutdown");
    let (host, resources) = Box::pin(fixture.start(None, Overrides::default())).await;
    let mut previous = Box::pin(fixture.app(&host, &resources, None, None)).await;
    previous
        .app
        .transcript
        .push_notice(NoticeKind::Local, "old screen");
    previous.app.composer.replace("recall across sessions");
    previous.app.composer.record_current();
    let mut selection = Selection::default();
    let mut resume = None;
    apply_palette_command(
        &mut selection,
        &mut resume,
        host.session().id().as_str().to_owned(),
        SelectionCommand::Resume(other.as_str().to_owned()),
    );
    Box::pin(host.shutdown()).await.expect("shutdown");
    let (host, resources) =
        Box::pin(fixture.start(resume.map(SessionId::new), Overrides::default())).await;
    let mut switched = Box::pin(fixture.app(&host, &resources, Some(previous), None)).await;
    assert!(
        matches!(&switched.app.transcript.blocks()[0], Block::User { text } if text == "other session prompt")
    );
    assert!(
        matches!(&switched.app.transcript.blocks()[1], Block::Assistant { text, .. } if text == "finished answer")
    );
    assert_eq!(switched.app.transcript.len(), 2);
    assert!(switched.app.composer.is_empty());
    assert!(switched.app.composer.recall_previous());
    assert_eq!(switched.app.composer.text(), "recall across sessions");
    Box::pin(host.shutdown()).await.expect("shutdown");
}

#[tokio::test]
async fn new_session_clears_transcript_and_keeps_recallable_pastes() {
    let fixture = Fixture::new();
    let (host, resources) = Box::pin(fixture.start(None, Overrides::default())).await;
    let mut previous = Box::pin(fixture.app(&host, &resources, None, None)).await;
    previous
        .app
        .transcript
        .push_notice(NoticeKind::Local, "old screen");
    previous.app.on_paste("first line\nsecond line\nthird line");
    let recalled = previous.app.composer.text().to_owned();
    previous.app.composer.record_current();
    let mut selection = Selection::default();
    let mut resume = Some(host.session().id().as_str().to_owned());
    apply_palette_command(
        &mut selection,
        &mut resume,
        host.session().id().as_str().to_owned(),
        SelectionCommand::NewSession,
    );
    assert!(resume.is_none());
    Box::pin(host.shutdown()).await.expect("shutdown");
    let (host, resources) = Box::pin(fixture.start(None, Overrides::default())).await;
    let mut fresh = Box::pin(fixture.app(&host, &resources, Some(previous), None)).await;
    assert!(fresh.app.transcript.is_empty());
    assert!(fresh.app.composer.recall_previous());
    assert_eq!(fresh.app.composer.text(), recalled);
    let Some(Action::Submit { submission, .. }) = fresh
        .app
        .on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("recalled paste must remain submittable");
    };
    assert_eq!(submission.display_text(), recalled);
    assert!(matches!(
        &submission.input_without_files().parts[0],
        agent_runtime_core::content::ContentPart::Text { text }
            if text == "first line\nsecond line\nthird line"
    ));
    Box::pin(host.shutdown()).await.expect("shutdown");
}

#[tokio::test]
async fn unchanged_rebind_suppresses_setup_notices_and_reports_cleared_override() {
    let fixture = Fixture::new();
    let (host, resources) = Box::pin(fixture.start(
        None,
        Overrides {
            reasoning_effort: Some("high".to_owned()),
            ..Overrides::default()
        },
    ))
    .await;
    let mut previous = Box::pin(fixture.app(&host, &resources, None, None)).await;
    let count = previous.app.transcript.len();
    assert_eq!(count, 1, "one initial override notice");
    previous
        .app
        .composer
        .replace("draft survives recomposition");
    let previous = Box::pin(fixture.app(&host, &resources, Some(previous), None)).await;
    assert_eq!(previous.app.transcript.len(), count);
    assert_eq!(previous.app.composer.text(), "draft survives recomposition");
    let session = host.session().id().clone();
    Box::pin(host.shutdown()).await.expect("shutdown");
    let (host, resources) = Box::pin(fixture.start(Some(session), Overrides::default())).await;
    let notice = "cleared the saved thinking/effort override because the selected provider/model cannot represent it";
    let rebound =
        Box::pin(fixture.app(&host, &resources, Some(previous), Some(notice.to_owned()))).await;
    assert_eq!(rebound.app.transcript.len(), count + 1);
    assert_notice(&rebound.app, "reasoning", notice);
    Box::pin(host.shutdown()).await.expect("shutdown");
}

#[tokio::test]
async fn reasoning_and_context_rebind_notices_describe_only_changed_values() {
    let fixture = Fixture::new();
    let (host, resources) = Box::pin(fixture.start(None, Overrides::default())).await;
    let mut previous = Box::pin(fixture.app(&host, &resources, None, None)).await;
    previous
        .app
        .transcript
        .push_notice(NoticeKind::Local, "kept screen");
    let session = host.session().id().clone();
    Box::pin(host.shutdown()).await.expect("shutdown");
    let (host, resources) = Box::pin(fixture.start(
        Some(session),
        Overrides {
            reasoning_effort: Some("high".to_owned()),
            context_window: Some("256k".to_owned()),
            ..Overrides::default()
        },
    ))
    .await;
    let rebound = Box::pin(fixture.app(&host, &resources, Some(previous), None)).await;
    assert_eq!(rebound.app.transcript.len(), 3);
    assert!(
        matches!(&rebound.app.transcript.blocks()[1], Block::Notice { kind: source, text }
        if source.label() == "reasoning" && text.starts_with("thinking on · effort high ·"))
    );
    assert_notice(&rebound.app, "context", "window changed · 128k → 256k");
    assert_eq!(rebound.app.status.context_window.as_deref(), Some("256k"));
    assert_eq!(
        rebound.app.status.reasoning_hint.as_deref(),
        Some("think on · effort high")
    );
    let rebound = Box::pin(fixture.app(&host, &resources, Some(rebound), None)).await;
    assert_eq!(
        rebound.app.transcript.len(),
        3,
        "unchanged settings stay quiet"
    );
    Box::pin(host.shutdown()).await.expect("shutdown");
}

#[test]
fn reconfigure_is_refused_while_busy_or_a_prompt_is_pending() {
    let command = || {
        SessionControl::Reconfigure(SelectionCommand::Model {
            provider: Some("local".to_owned()),
            model: "next-model".to_owned(),
        })
    };
    let mut app = App::new("example-model", "project");
    app.composer.replace("draft");
    app.apply(&event(RuntimeEvent::TurnStarted));
    assert!(reconfigure_exit(&mut app, command()).is_none());
    assert!(app.is_busy(), "the existing turn keeps running");
    assert_eq!(app.composer.text(), "draft");
    let assert_refused = |app: &App| {
        assert_eq!(
            app.feedback_notice().map(|notice| notice.text.as_str()),
            Some("/model requires an idle turn; draft preserved"),
        );
        assert!(app.transcript.is_empty());
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20))
            .expect("test terminal");
        terminal
            .draw(|frame| smith_tui::render::draw(frame, app, smith_tui::Theme::new()))
            .expect("frame");
        let buffer = terminal.backend().buffer();
        let hint = (0..100)
            .map(|x| buffer[(x, 19)].symbol())
            .collect::<String>();
        assert!(
            hint.contains("/model requires an idle turn; draft preserved"),
            "{hint}"
        );
    };
    assert_refused(&app);
    app.reset_live_turn();
    app.open_overlay(Overlay::Shortcuts);
    let questionnaire = |id| {
        QuestionnaireForm::new(
            id,
            vec![QuestionnaireQuestion::new(
                "choice",
                "Choice",
                "Choose a direction",
                vec![QuestionnaireChoice::new("yes", "Yes")],
            )],
            Deadline::never(),
        )
        .expect("questionnaire")
    };
    app.present_questionnaire(questionnaire("visible-question"));
    assert!(
        matches!(&app.overlay, Some(Overlay::Questionnaire { state })
        if state.form().request_id == "visible-question")
    );
    assert_eq!(app.queued_prompt_count(), 0);
    assert!(reconfigure_exit(&mut app, command()).is_none());
    assert_refused(&app);
    assert!(
        matches!(&app.overlay, Some(Overlay::Questionnaire { state })
        if state.form().request_id == "visible-question")
    );
    app.present_questionnaire(questionnaire("queued-question"));
    assert!(app.has_pending_prompt());
    assert_eq!(app.queued_prompt_count(), 1);
    assert!(reconfigure_exit(&mut app, command()).is_none());
    assert_refused(&app);
    assert!(
        matches!(&app.overlay, Some(Overlay::Questionnaire { state })
        if state.form().request_id == "visible-question")
    );
    assert_eq!(app.queued_prompt_count(), 1);
    assert_eq!(app.pending_questionnaire_count(), 2);
    assert_eq!(app.composer.text(), "draft");
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        matches!(&app.overlay, Some(Overlay::Questionnaire { state })
        if state.form().request_id == "queued-question")
    );
    assert_eq!(app.queued_prompt_count(), 0);
    let mut idle = App::new("example-model", "project");
    assert!(matches!(
        reconfigure_exit(&mut idle, command()),
        Some(InteractiveExit::Reconfigure(_))
    ));
}

#[test]
fn child_rebind_keeps_only_coordinator_listed_inspector_logs() {
    let mut app = App::new("example-model", "project");
    for child in ["kept", "removed"] {
        app.restore_child(child, ChildState::Idle, None);
        app.apply_child(
            child,
            &event(RuntimeEvent::TextDelta {
                request: RequestId::new("request-1"),
                attempt: AttemptId::new("attempt-1"),
                text: format!("{child} log"),
            }),
        );
        app.apply_child(
            child,
            &event(RuntimeEvent::ProviderAttemptOutputCommitted {
                request: RequestId::new("request-1"),
                attempt: AttemptId::new("attempt-1"),
            }),
        );
    }
    let kept = format!("{:?}", app.child_blocks("kept"));
    app.inspected_child = Some("kept".to_owned());
    app.replace_children([(
        "kept".to_owned(),
        ChildState::Interrupted { resumable: true },
        Some("new host detail".to_owned()),
    )]);
    assert_eq!(format!("{:?}", app.child_blocks("kept")), kept);
    assert!(app.child_blocks("removed").is_empty());
    assert_eq!(app.inspected_child.as_deref(), Some("kept"));
    assert_eq!(
        app.children["kept"].state,
        ChildState::Interrupted { resumable: true }
    );
    assert_eq!(
        app.children["kept"].detail.as_deref(),
        Some("new host detail")
    );
    app.replace_children([]);
    assert!(app.inspected_child.is_none());
}

#[tokio::test]
async fn input_during_model_rebuild_reaches_composer_in_order_and_quit_is_kept() {
    use crossterm::event::Event;

    let fixture = Fixture::new();
    let (host, resources) = Box::pin(fixture.start(None, Overrides::default())).await;
    let mut previous = Box::pin(fixture.app(&host, &resources, None, None)).await;
    // The fake provider has no catalog inventory for inactive named windows;
    // supply the selectable row whose configured host the test will rebuild.
    previous.app.resources.models.push(ResourceEntry::new(
        "local/next-model",
        "next-model",
        "local",
    ));
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut keys = Box::pin(futures_util::stream::unfold(
        receiver,
        |mut receiver| async { receiver.recv().await.map(|event| (event, receiver)) },
    ));
    let send_text = |text: &str| {
        for character in text.chars() {
            sender
                .send(Ok(Event::Key(KeyEvent::new(
                    KeyCode::Char(character),
                    KeyModifiers::NONE,
                ))))
                .expect("queued key");
        }
    };
    let send_key = |code, modifiers| {
        sender
            .send(Ok(Event::Key(KeyEvent::new(code, modifiers))))
            .expect("queued key");
    };
    send_text("/model");
    send_key(KeyCode::Enter, KeyModifiers::NONE);
    send_text("next-model");
    send_key(KeyCode::Enter, KeyModifiers::NONE);
    // Also queue a key behind the confirmation: it must remain unread when
    // the old loop returns, rather than being discarded with that loop.
    send_text("t");
    let (exit, app) = Box::pin(crate::tui_driver::run_scripted_tui(
        &host,
        fixture.project.path(),
        &resources,
        previous.app,
        &mut keys,
        false,
    ))
    .await
    .expect("model picker loop");
    assert!(matches!(
        exit,
        InteractiveExit::Reconfigure(SelectionCommand::Model { ref model, .. })
            if model == "next-model"
    ));
    assert!(
        app.composer.is_empty(),
        "keys after confirmation stay queued"
    );
    previous.app = app;
    let session = host.session().id().clone();
    Box::pin(host.shutdown()).await.expect("shutdown");

    // No loop is reading while the host is rebuilt. Mix keys, editing, and a
    // paste so reordering or partial delivery changes the resulting draft.
    send_text("yped right after the switcX");
    send_key(KeyCode::Backspace, KeyModifiers::NONE);
    sender.send(Ok(Event::Paste("h".into()))).expect("paste");
    let (host, resources) = Box::pin(fixture.start(
        Some(session),
        Overrides {
            model: Some("next-model".to_owned()),
            ..Overrides::default()
        },
    ))
    .await;
    let mut rebound = Box::pin(fixture.app(&host, &resources, Some(previous), None)).await;
    let (exit, app) = Box::pin(crate::tui_driver::run_scripted_tui(
        &host,
        fixture.project.path(),
        &resources,
        rebound.app,
        &mut keys,
        true,
    ))
    .await
    .expect("rebuilt loop");
    assert!(matches!(exit, InteractiveExit::CapabilitiesChanged));
    assert_eq!(app.composer.text(), "typed right after the switch");
    rebound.app = app;
    Box::pin(host.shutdown()).await.expect("shutdown");

    // A second same-session rebuild keeps the existing draft as well as new
    // input. Clear only by an explicit user key, then quit from the queue.
    send_text(" again");
    let (host, resources) = Box::pin(fixture.start(
        Some(host.session().id().clone()),
        Overrides {
            model: Some("next-model".to_owned()),
            ..Overrides::default()
        },
    ))
    .await;
    let mut rebound = Box::pin(fixture.app(&host, &resources, Some(rebound), None)).await;
    let (_, app) = Box::pin(crate::tui_driver::run_scripted_tui(
        &host,
        fixture.project.path(),
        &resources,
        rebound.app,
        &mut keys,
        true,
    ))
    .await
    .expect("recomposition loop");
    assert_eq!(app.composer.text(), "typed right after the switch again");
    rebound.app = app;
    send_key(KeyCode::Char('u'), KeyModifiers::CONTROL);
    send_text("/quit");
    send_key(KeyCode::Enter, KeyModifiers::NONE);
    let (exit, _) = Box::pin(crate::tui_driver::run_scripted_tui(
        &host,
        fixture.project.path(),
        &resources,
        rebound.app,
        &mut keys,
        false,
    ))
    .await
    .expect("queued quit");
    assert!(matches!(exit, InteractiveExit::Quit(..)));
    Box::pin(host.shutdown()).await.expect("shutdown");
}
