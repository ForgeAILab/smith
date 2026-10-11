use super::*;

// local commands behavior tests.

use crate::local_command::context::{
    display_categories as context_display_categories, display_category as context_display_category,
    report as context_report,
};
use crate::local_command::diagnostics::{cache_controller_rows, context_section, harness_section};
use smith_client::context_report::{ContextCategoryKind, render_plain as render_context_plain};
use smith_client::diagnostics_report::{DiagnosticsReport, DiagnosticsRow, DiagnosticsSection};
use smith_client::status::{PriceReference, PriceTable, SessionUsage};

fn diagnostics_section_plain(section: DiagnosticsSection) -> String {
    smith_client::diagnostics_report::render_plain(&DiagnosticsReport {
        sections: vec![section],
    })
}

#[test]
fn status_cost_reports_unknown_without_assuming_a_price() {
    // usage-accounting: "Price is unavailable" — `/status` must show the
    // counters and report cost as unknown rather than assuming a price,
    // unlike the exit report's "no cost line at all".
    let mut totals = std::collections::BTreeMap::new();
    totals.insert(CounterKind::InputUncached, 1_000);
    let usage = SessionUsage {
        turns: 1,
        reported: true,
        totals,
        ..SessionUsage::default()
    };
    let rendered = render_status_cost(&usage, None, ("openai", "gpt-5.3"));
    assert_eq!(rendered, "unknown · no price reference for openai/gpt-5.3");
}

#[test]
fn status_cost_names_the_priced_binding_and_labels_it_exact() {
    // usage-accounting: "A priced model with reported counters" — one
    // USD figure labelled exact, naming the provider and model.
    let mut totals = std::collections::BTreeMap::new();
    totals.insert(CounterKind::InputUncached, 1_000_000);
    let usage = SessionUsage {
        turns: 1,
        reported: true,
        totals,
        ..SessionUsage::default()
    };
    let price = PriceReference {
        provider: "openai".to_owned(),
        model: "gpt-5.3".to_owned(),
        table: PriceTable {
            input: Some(2_000_000),
            output: None,
            cache_read: None,
            cache_write: None,
        },
    };
    let rendered = render_status_cost(&usage, Some(&price), ("openai", "gpt-5.3"));
    assert_eq!(rendered, "$2.000 exact · openai/gpt-5.3");
}

#[test]
fn status_cost_says_nothing_spent_yet_before_any_usage() {
    // A fresh session has no usage to price at all — a different fact
    // from "priced but unknown", and must not be conflated with it.
    let rendered = render_status_cost(&SessionUsage::default(), None, ("openai", "gpt-5.3"));
    assert_eq!(rendered, "nothing spent yet");
}

#[test]
fn cache_controller_status_projects_idle_compaction_metadata_and_usage() {
    let mut controller = smith_runtime::cache_controller::CacheControllerSnapshot::default();
    controller.idle_compaction.attempted = true;
    controller.idle_compaction_outcome =
        Some(smith_runtime::cache_controller::IdleCompactionOutcome::Completed);
    controller.idle_compaction_reason = Some("artifact_projection_unavailable".to_owned());
    controller.idle_compaction_latency_ms = Some(17);
    controller.idle_compaction_provider = Some("summary-provider".to_owned());
    controller.idle_compaction_model = Some("summary-model".to_owned());
    controller.idle_compaction_revision =
        Some(agent_runtime::registry::RegistryRevision::new("summary-r1"));
    controller.idle_compaction_usage.input_uncached = 120;
    controller.idle_compaction_usage.input_cached = 800;
    controller.idle_compaction_usage.output = 20;
    controller.synthetic_attempts.push(
        smith_runtime::cache_controller::SyntheticCacheAttemptProjection {
            operation: None,
            attempt: None,
            purpose: agent_runtime_core::provider::ProviderAttemptPurpose::IdleCompaction,
            provider: "summary-provider".to_owned(),
            model: "summary-model".to_owned(),
            cache_identity: None,
            usage: controller.idle_compaction_usage.clone(),
            counter_provenance: Default::default(),
            cost_micro_usd: None,
            cost_provenance: Default::default(),
            latency_ms: 17,
            status: "completed".to_owned(),
        },
    );

    let section = DiagnosticsSection {
        heading: "Cache".to_owned(),
        rows: cache_controller_rows(&controller),
    };
    for row in &section.rows {
        if let DiagnosticsRow::Field { label, .. } = row {
            assert!(
                ratatui::text::Line::from(label.as_str()).width() <= 18,
                "{label}"
            );
        }
    }
    let rendered = diagnostics_section_plain(section);
    let (_, maintenance) = rendered.split_once("\nmaintenance: ").unwrap();
    let (maintenance, _) = maintenance.split_once("\nidle compaction: ").unwrap();
    assert!(
        maintenance
            .lines()
            .skip(1)
            .all(|line| line.starts_with("  "))
    );
    let (_, idle) = rendered
        .split_once("\nidle compaction: attempted yes\n")
        .expect("idle facts follow their parent row");
    let (idle, synthetic) = idle.split_once("\nsynthetic attempts: 1\n").unwrap();
    assert!(idle.lines().all(|line| line.starts_with("  ")));
    assert!(synthetic.lines().all(|line| line.starts_with("  ")));
    assert!(idle.contains("  decision: none"), "{rendered}");
    assert!(idle.contains("  outcome: completed"), "{rendered}");
    assert!(
        idle.contains("  reason: artifact_projection_unavailable"),
        "{rendered}"
    );
    assert!(idle.contains("  latency: 17ms"), "{rendered}");
    assert!(
        idle.contains("  route: summary-provider/summary-model/summary-r1"),
        "{rendered}"
    );
    assert!(
        idle.contains("  usage: input 120 · cached 800 · writes 0 · output 20 · reasoning 0"),
        "{rendered}"
    );
    for fact in [
        "  purpose: idle compaction",
        "  route: summary-provider/summary-model",
        "  identity: none",
        "  usage: input 120 · cached 800 · writes 0 · output 20 · reasoning 0",
        "  cost: unknown (unknown)",
        "  latency: 17ms",
        "  status: completed",
    ] {
        assert!(synthetic.lines().any(|line| line == fact), "{rendered}");
    }
}

#[test]
fn context_categories_name_tool_results_separately_from_user_input() {
    let tool = context_display_category("tool_result", 4_200);
    let user = context_display_category("user_input", 58);
    assert_eq!(tool.label, "tool results");
    assert_eq!(tool.kind, ContextCategoryKind::Tool);
    assert_eq!(tool.tokens, 4_200);
    assert_eq!(user.label, "user input");
    assert_eq!(user.kind, ContextCategoryKind::Input);
    assert_eq!(user.tokens, 58);
    assert!(tool.rank < user.rank);

    let unknown = context_display_category("future_context_kind", 1);
    assert_eq!(unknown.label, "future context kind");
    assert_eq!(unknown.kind, ContextCategoryKind::Other);
}

#[test]
fn context_categories_keep_system_and_tools_visible_and_aggregate_instructions() {
    let totals = std::collections::BTreeMap::from([
        ("system_instruction".to_owned(), 100),
        ("developer_instruction".to_owned(), 40),
        ("ability_instruction".to_owned(), 60),
        ("history".to_owned(), 300),
    ]);

    let categories = context_display_categories(&totals);
    assert_eq!(categories[0].label, "system instructions");
    assert_eq!(categories[0].tokens, 200);
    assert_eq!(categories[1].label, "tool schemas");
    assert_eq!(categories[1].tokens, 0);
    assert_eq!(categories[2].label, "history");
    assert!(
        categories
            .iter()
            .all(|category| category.label != "developer instructions")
    );
}

#[test]
fn harness_status_names_registry_view_activation_and_context_provenance() {
    let mut status = Status::new("example-model", "/project");
    status.record_registry("registry-fingerprint", 6);
    status.record_scoped_view("view-fingerprint", 4);
    status.record_retrieval("resolver-1", vec!["tool:read".into(), "tool:search".into()]);
    status.record_activation(1, vec!["tool:read".into()]);
    let totals = std::collections::BTreeMap::new();
    status.record_context_plan(ContextPlanUpdate {
        fingerprint: "context-fingerprint",
        cache_fingerprint: "cache-fingerprint",
        input_tokens: 100,
        input_budget_tokens: 1_000,
        reserved_tokens: 100,
        segment_count: 1,
        totals: &totals,
        confidence: EstimationConfidence::Exact,
    });
    status.record_compaction(250);

    let rendered = diagnostics_section_plain(harness_section(&status));
    assert!(
        rendered.contains("registry snapshot: registry-fingerprint\n  entries: 6"),
        "{rendered}"
    );
    assert!(
        rendered.contains("capability view: view-fingerprint\n  visible: 4"),
        "{rendered}"
    );
    assert!(
        rendered.contains("retrieval: resolver-1\n  candidates: tool:read, tool:search"),
        "{rendered}"
    );
    assert!(
        rendered.contains("activation epoch: 1\n  active: tool:read"),
        "{rendered}"
    );
    assert!(
        rendered.contains("context provenance: context-fingerprint\n  cache: cache-fingerprint"),
        "{rendered}"
    );
    assert!(
        rendered.contains("compaction runs: 1\ntokens reclaimed: 250"),
        "{rendered}"
    );
}

#[tokio::test]
async fn informational_commands_append_inline_without_provider_history() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
    let config_with_windows = LOCAL_COMMAND_CONFIG.replace(
                "context_tokens = 128000\nmax_input_tokens = 124000\nmax_output_tokens = 4096",
                "default_context_window = \"128k\"\nmax_output_tokens = 4096\n\n[models.\"local/example-model\".context_windows.\"128k\"]\ncontext_tokens = 128000\nmax_input_tokens = 124000\n\n[models.\"local/example-model\".context_windows.\"256k\"]\ncontext_tokens = 256000\nmax_input_tokens = 252000",
            );
    assert_ne!(config_with_windows, LOCAL_COMMAND_CONFIG);
    std::fs::write(
        project.path().join(".smith/config.toml"),
        config_with_windows,
    )
    .expect("config");
    let config = resolve(
        &ResolveRequest::new(project.path())
            .with_known_modules(crate::modules::known_modules())
            .with_home_dir(home.path()),
    )
    .expect("resolution")
    .config;
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("workspace"),
        )),
        approval: Some(Arc::new(agent_runtime_core::approval::DenyAll)),
        ..RuntimeRequest::new(config, HostSurface::Terminal)
    };
    let host = smith_runtime::host::start(
        HostSessionRequest::new(runtime, project.path())
            .checkpoint_keys(Arc::new(TestCheckpointKeys)),
    )
    .await
    .expect("host");
    let history_before = host.session().history().len();
    let mut app = App::new("example-model", project.path().display().to_string());

    let before_plan =
        diagnostics_section_plain(context_section(&app.status, host.runtime().policy()));
    assert!(before_plan.contains("not planned yet"), "{before_plan}");
    assert!(
        before_plan.contains("128k total · 4k reserved"),
        "{before_plan}"
    );
    assert!(
        before_plan.contains("compaction: enabled on overflow\nrecovery target: 74.3k"),
        "{before_plan}"
    );
    let before_context =
        render_context_plain(&context_report(&app.status, host.runtime().policy()));
    assert!(
        before_context.contains("available context windows: 128k (active), 256k"),
        "{before_context}"
    );
    assert!(
        before_context.contains("usage unavailable until the first turn"),
        "{before_context}"
    );
    assert!(
        before_context.contains("· free input: 123.9k"),
        "{before_context}"
    );
    assert!(
        before_context.contains("□ output/reasoning reserve: 4k"),
        "{before_context}"
    );
    assert!(
        before_context.contains("■ system instructions: ? (not counted yet)"),
        "{before_context}"
    );
    assert!(
        before_context.contains("◆ tool schemas: ? (not counted yet)"),
        "{before_context}"
    );
    assert_eq!(
        before_context
            .lines()
            .filter(|line| {
                !line.is_empty()
                    && line
                        .chars()
                        .all(|character| matches!(character, '·' | '□' | ' '))
            })
            .count(),
        5,
        "{before_context}"
    );

    let totals = std::collections::BTreeMap::from([
        (
            agent_runtime_core::manifest::SegmentKind::new("history"),
            1_500,
        ),
        (
            agent_runtime_core::manifest::SegmentKind::new("tool_schema"),
            500,
        ),
    ]);
    app.status.record_context_plan(ContextPlanUpdate {
        fingerprint: "context-test",
        cache_fingerprint: "cache-test",
        input_tokens: 2_000,
        input_budget_tokens: 123_904,
        reserved_tokens: 4_096,
        segment_count: 2,
        totals: &totals,
        confidence: EstimationConfidence::Estimated,
    });
    let planned = diagnostics_section_plain(context_section(&app.status, host.runtime().policy()));
    assert!(planned.contains("~98% input left"), "{planned}");
    assert!(planned.contains("~2k used / 123.9k budget"), "{planned}");
    assert!(planned.contains("provider input: unknown"), "{planned}");
    assert!(planned.contains("tool schema: ~500"), "{planned}");
    assert!(
        planned.contains("compaction: enabled on overflow\nrecovery target: 74.3k"),
        "{planned}"
    );
    let context = render_context_plain(&context_report(&app.status, host.runtime().policy()));
    assert!(
        context.contains("example-model · ~2k / 123.9k input tokens · ~98% left"),
        "{context}"
    );
    assert!(context.contains("◆ tool schemas: ~500"), "{context}");
    assert!(
        context.contains("■ system instructions: ~0 (0.0%)"),
        "{context}"
    );
    assert!(context.contains("● history: ~1.5k"), "{context}");
    assert!(
        context.contains("counting: estimated · 2 segments"),
        "{context}"
    );

    let diagnostics = crate::local_command::diagnostics::report(&app, &host, project.path());
    for section in &diagnostics.sections {
        for row in &section.rows {
            if let DiagnosticsRow::Field { label, .. } = row {
                assert!(
                    ratatui::text::Line::from(label.as_str()).width() <= 18,
                    "{} label exceeds 18 columns: {label:?}",
                    section.heading
                );
            }
        }
    }
    assert_eq!(
        diagnostics
            .sections
            .iter()
            .map(|section| section.heading.as_str())
            .collect::<Vec<_>>(),
        ["Session", "Context", "Cache", "Recovery"]
    );
    let plain = smith_client::diagnostics_report::render_plain(&diagnostics);
    assert_eq!(
        plain
            .lines()
            .filter(|line| line.starts_with("reasoning:"))
            .count(),
        1
    );
    assert_eq!(
        plain
            .lines()
            .filter(|line| line.starts_with("reasoning controls:"))
            .count(),
        1
    );
    assert!(plain.contains("idle compaction: attempted no"), "{plain}");
    assert!(plain.contains("  route: unknown"), "{plain}");
    assert!(plain.contains("session read: unknown"), "{plain}");
    assert!(plain.contains("cache hit: unknown"), "{plain}");
    assert!(!plain.contains('?'), "{plain}");

    let (_, skills) = crate::skills::SkillContext::compose(
        smith_runtime::skills::SmithSkillSources::new(),
        home.path(),
        project.path(),
    )
    .expect("a skill context");
    let commands = [
        HostCommand::Status,
        HostCommand::Diagnostics,
        HostCommand::Context,
        HostCommand::Agent(AgentAction::List),
        HostCommand::Diff(DiffScope::Git(None)),
        HostCommand::Skills(smith_client::commands::SkillsAction::List),
    ];
    for command in commands {
        handle_local_command(&mut app, &host, project.path(), None, &skills, command).await;
    }

    git(project.path(), &["init"]);
    git(
        project.path(),
        &["config", "user.email", "smith@example.invalid"],
    );
    git(project.path(), &["config", "user.name", "Smith Test"]);
    std::fs::write(project.path().join("tracked.txt"), "before\n").expect("tracked");
    git(project.path(), &["add", "tracked.txt"]);
    git(project.path(), &["commit", "-m", "initial"]);
    std::fs::write(project.path().join("tracked.txt"), "after\n").expect("changed");
    handle_local_command(
        &mut app,
        &host,
        project.path(),
        None,
        &skills,
        HostCommand::Diff(DiffScope::Git(Some("unstaged".to_owned()))),
    )
    .await;

    for character in "/help".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    assert_eq!(
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        None
    );

    let results = app
        .transcript
        .blocks()
        .iter()
        .filter_map(|block| match block {
            Block::Local(result) => Some((result.title(), result.state())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        results,
        [
            ("status", LocalResultState::Info),
            ("diagnostics", LocalResultState::Info),
            ("context", LocalResultState::Info),
            ("agent", LocalResultState::Empty),
            ("diff", LocalResultState::Error),
            ("skills", LocalResultState::Info),
            ("diff · unstaged", LocalResultState::Info),
            ("help", LocalResultState::Info),
        ]
    );
    assert!(app.overlay.is_none(), "local output must not open a viewer");
    assert_eq!(
        host.session().history().len(),
        history_before,
        "local output became provider conversation history"
    );
    let status_report = app
        .transcript
        .blocks()
        .iter()
        .find_map(|block| match block {
            Block::Local(LocalResult::Status(report)) => Some(report),
            _ => None,
        })
        .expect("status output");
    assert_eq!(status_report.profile, "dev");
    assert_eq!(status_report.cost, "nothing spent yet");
    let status_content = smith_client::status_report::render_plain(status_report);
    assert!(status_content.contains("profile: dev"), "{status_content}");
    assert!(status_content.contains("/diagnostics"), "{status_content}");
    assert!(
        !status_content.contains("posture build"),
        "{status_content}"
    );
    assert!(!status_content.contains("cache: state"), "{status_content}");
    assert!(
        !status_content.contains("resume capsule:"),
        "{status_content}"
    );
    let diagnostics_report = app
        .transcript
        .blocks()
        .iter()
        .find_map(|block| match block {
            Block::Local(LocalResult::Diagnostics(report)) => Some(report),
            _ => None,
        })
        .expect("diagnostic output");
    let diagnostics_content = smith_client::diagnostics_report::render_plain(diagnostics_report);
    assert!(
        diagnostics_content.contains("~98% input left"),
        "{diagnostics_content}"
    );
    for fact in ["profile: dev", "posture: build", "profile uses: main"] {
        assert!(
            diagnostics_content.lines().any(|line| line == fact),
            "{diagnostics_content}"
        );
    }
    for label in ["profile revision: ", "profile source: "] {
        assert!(
            diagnostics_content
                .lines()
                .any(|line| line.starts_with(label) && line.len() > label.len()),
            "{diagnostics_content}"
        );
    }
    // No usage has been recorded yet, so `/status` reports "nothing
    // spent yet" rather than either a zero dollar figure or "unknown" —
    // there is nothing to price, which is a different fact from a
    // priced session's cost being unpriceable.
    assert!(
        status_content.contains("cost: nothing spent yet"),
        "{status_content}"
    );
    let context_report = app
        .transcript
        .blocks()
        .iter()
        .find_map(|block| match block {
            Block::Local(LocalResult::Context(report)) => Some(report),
            _ => None,
        })
        .expect("context output");
    assert_eq!(
        context_report.usage,
        smith_client::context_report::ContextUsage::Estimated,
    );
    let context_content = render_context_plain(context_report);
    assert!(
        context_content.contains("Estimated usage by category"),
        "{context_content}"
    );
    assert!(
        context_content.contains("available context windows: 128k (active), 256k"),
        "{context_content}"
    );

    let totals = std::collections::BTreeMap::from([
        (
            agent_runtime_core::manifest::SegmentKind::new("summary"),
            600,
        ),
        (
            agent_runtime_core::manifest::SegmentKind::new("user_input"),
            600,
        ),
    ]);
    app.status.record_context_plan(ContextPlanUpdate {
        fingerprint: "context-summary",
        cache_fingerprint: "cache-summary",
        input_tokens: 1_200,
        input_budget_tokens: 123_904,
        reserved_tokens: 4_096,
        segment_count: 2,
        totals: &totals,
        confidence: EstimationConfidence::Estimated,
    });
    let compacted =
        diagnostics_section_plain(context_section(&app.status, host.runtime().policy()));
    assert!(
        compacted.contains("compaction: applied\nrecovery target: 74.3k"),
        "{compacted}"
    );
    host.shutdown().await.expect("shutdown");
}

#[test]
fn disabled_maintenance_is_not_rendered_as_an_authority_failure() {
    let controller = smith_runtime::cache_controller::CacheControllerSnapshot {
        requested_maintenance: smith_runtime::cache_lifecycle::CacheMaintenanceMode::Off,
        effective_maintenance: smith_runtime::cache_lifecycle::CacheMaintenanceMode::Off,
        ..Default::default()
    };
    let rendered = crate::local_command::render_cache_controller_summary(&controller);
    assert_eq!(rendered, "cache maintenance: off");
    assert!(!rendered.contains("denied"));
    assert!(!rendered.contains("lease"));
    assert!(!rendered.contains("idle compaction"));
}

#[test]
fn observe_only_maintenance_names_its_no_spend_behavior() {
    let controller = smith_runtime::cache_controller::CacheControllerSnapshot::default();
    let rendered = crate::local_command::render_cache_controller_summary(&controller);
    assert_eq!(
        rendered,
        "cache maintenance: observe only (no background requests)"
    );
    assert!(!rendered.contains("denied"));
}
