use super::*;

pub(super) fn fixture_raw_and_view(
    app: &App,
    normalizer: &mut fixture_support::Normalizer,
) -> (String, App) {
    let mut raw = String::new();
    let mut view = App::new("example-model", "<PROJECT>");
    view.children = app.children.clone();
    if let Some(child) = &app.inspected_child {
        let detail = app.inspected_detail().expect("child status card");
        let content = smith_client::agent_report::render_plain(
            &smith_client::agent_report::AgentReport::Inspector(detail.clone()),
        );
        raw.push_str(&format!(
            "title: agent {child}\nstate: Inspector\nbody:\n{content}\n"
        ));
        view.inspect_child(child.clone());
        view.set_inspected_detail(child, Some(fixture_agent_snapshot_view(detail, normalizer)));
    }
    for block in app.transcript.blocks() {
        match block {
            Block::Local(result) => {
                let title = result.title();
                let state = match result {
                    LocalResult::Agent(report)
                        if matches!(
                            report.as_ref(),
                            smith_client::agent_report::AgentReport::Resume(
                                smith_client::agent_report::AgentResumeReport::RequiresIdle
                                    | smith_client::agent_report::AgentResumeReport::Started { .. }
                            )
                        ) =>
                    {
                        "Notice".to_owned()
                    }
                    LocalResult::Review(report)
                        if matches!(
                            report.as_ref(),
                            smith_client::review_report::ReviewReport::Empty
                                | smith_client::review_report::ReviewReport::Start(
                                    smith_client::review_report::ReviewStartReport::Started { .. }
                                        | smith_client::review_report::ReviewStartReport::Queued { .. }
                                )
                        ) =>
                    {
                        "Notice".to_owned()
                    }
                    LocalResult::Recovery(report) if report.is_notice() => "Notice".to_owned(),
                    _ => format!("{:?}", result.state()),
                };
                let content = match result {
                    LocalResult::Status(report) => {
                        smith_client::status_report::render_plain(report)
                    }
                    LocalResult::Diagnostics(report) => {
                        smith_client::diagnostics_report::render_plain(report)
                    }
                    LocalResult::Context(report) => {
                        smith_client::context_report::render_plain(report)
                    }
                    LocalResult::Help(report) => smith_client::help_report::render_plain(report),
                    LocalResult::Timeline(report) => {
                        smith_client::timeline_report::render_plain(report)
                    }
                    LocalResult::Goal(report) => smith_client::goal_report::render_plain(report),
                    LocalResult::Agent(report) => smith_client::agent_report::render_plain(report),
                    LocalResult::Mcp(report) => smith_client::mcp_report::render_plain(report),
                    LocalResult::Skills(report) => {
                        smith_client::skills_report::render_plain(report)
                    }
                    LocalResult::Diff(report) => smith_client::diff_report::render_plain(report),
                    LocalResult::Review(report) => {
                        smith_client::review_report::render_plain(report)
                    }
                    LocalResult::Recovery(report) => {
                        smith_client::recovery_report::render_plain(report)
                    }
                    LocalResult::Shell(report) => smith_client::shell_report::render_plain(report),
                    LocalResult::Message(report) => {
                        smith_client::message_report::render_plain(report)
                    }
                };
                raw.push_str(&format!(
                    "title: {title}\nstate: {state}\nbody:\n{content}\n"
                ));
                // Draw a normalized typed report, never a prose round-trip.
                let normalized = match result {
                    LocalResult::Status(report) => {
                        LocalResult::Status(Box::new(fixture_status_view(report, normalizer)))
                    }
                    LocalResult::Diagnostics(report) => LocalResult::Diagnostics(Box::new(
                        fixture_diagnostics_view(report, normalizer),
                    )),
                    LocalResult::Context(report) => {
                        LocalResult::Context(Box::new(fixture_context_view(report, normalizer)))
                    }
                    LocalResult::Help(report) => {
                        LocalResult::Help(Box::new(fixture_help_view(report, normalizer)))
                    }
                    LocalResult::Timeline(report) => {
                        LocalResult::Timeline(Box::new(fixture_timeline_view(report, normalizer)))
                    }
                    LocalResult::Goal(report) => {
                        LocalResult::Goal(Box::new(fixture_goal_view(report, normalizer)))
                    }
                    LocalResult::Agent(report) => {
                        LocalResult::Agent(Box::new(fixture_agent_view(report, normalizer)))
                    }
                    LocalResult::Mcp(report) => {
                        LocalResult::Mcp(Box::new(fixture_mcp_view(report, normalizer)))
                    }
                    LocalResult::Skills(report) => {
                        LocalResult::Skills(Box::new(fixture_skills_view(report, normalizer)))
                    }
                    LocalResult::Diff(report) => {
                        LocalResult::Diff(Box::new(fixture_diff_view(report, normalizer)))
                    }
                    LocalResult::Review(report) => {
                        LocalResult::Review(Box::new(fixture_review_view(report, normalizer)))
                    }
                    LocalResult::Recovery(report) => {
                        LocalResult::Recovery(Box::new(fixture_recovery_view(report, normalizer)))
                    }
                    LocalResult::Shell(report) => {
                        LocalResult::Shell(Box::new(fixture_shell_view(report, normalizer)))
                    }
                    LocalResult::Message(report) => {
                        LocalResult::Message(Box::new(fixture_message_view(report, normalizer)))
                    }
                };
                view.transcript.push_local(normalized);
            }
            Block::Error { message } => {
                raw.push_str(&format!("title: error\nstate: Error\nbody:\n{message}\n"));
                view.transcript.push_error(normalizer.normalize(message));
            }
            Block::Notice { kind: source, text } => {
                raw.push_str(&format!(
                    "title: {}\nstate: Notice\nbody:\n{text}\n",
                    source.label()
                ));
                view.transcript
                    .push_notice(source.clone(), normalizer.normalize(text));
            }
            _ => panic!("unexpected local fixture block: {block:?}"),
        }
    }
    use smith_tui::Overlay;
    match &app.overlay {
        Some(Overlay::Confirm(dialog)) => {
            let mut lines = Vec::new();
            if let Some((warning, _)) = &dialog.warning {
                lines.push(warning.clone());
            }
            lines.extend(dialog.body.clone());
            raw.push_str(&format!(
                "title: {}\nstate: Confirmation\nbody:\n{}\n",
                dialog.title,
                lines.join("\n"),
            ));
            let mut normalized = smith_tui::app::ConfirmDialog::new(
                &normalizer.normalize(&dialog.title),
                dialog.tone,
                dialog
                    .body
                    .iter()
                    .map(|line| normalizer.normalize(line))
                    .collect(),
                &dialog.accept_label,
                (*dialog.accept).clone(),
                (*dialog.cancel).clone(),
            );
            normalized.warning = dialog
                .warning
                .as_ref()
                .map(|(line, tone)| (normalizer.normalize(line), *tone));
            normalized.accept_tone = dialog.accept_tone;
            normalized.cancel_key = dialog.cancel_key;
            normalized.cancel_label = dialog.cancel_label.clone();
            normalized.hint = dialog.hint.clone();
            normalized.scroll = dialog.scroll;
            view.open_overlay(Overlay::Confirm(normalized));
        }
        Some(Overlay::ResourcePicker {
            picker,
            target,
            restore_on_escape,
        }) => {
            raw.push_str(&format!(
                "title: {}\nstate: Picker\nbody:\n{}\n",
                picker.title, picker.empty_guidance
            ));
            for entry in &picker.entries {
                raw.push_str(&format!(
                    "{} · {} · {} · active {} · disabled {:?}\n",
                    entry.id, entry.label, entry.detail, entry.active, entry.disabled_reason
                ));
            }
            view.open_overlay(Overlay::ResourcePicker {
                picker: picker.clone(),
                target: *target,
                restore_on_escape: restore_on_escape.clone(),
            });
        }
        None => {}
        other => panic!("unexpected local fixture overlay: {other:?}"),
    }
    if let Some(notice) = app.feedback_notice() {
        raw.push_str(&format!(
            "title: {}\nstate: Feedback\nbody:\n{}\n",
            notice.kind.label(),
            notice.text,
        ));
        view.push_notice(notice.kind.clone(), normalizer.normalize(&notice.text));
    }
    assert!(!raw.is_empty(), "command produced no captured local result");
    (normalizer.normalize(&raw), view)
}

fn fixture_shell_view(
    report: &smith_client::shell_report::ShellReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::shell_report::ShellReport {
    let mut report = report.clone();
    if let smith_client::shell_report::ShellOutput::Output(output) = &mut report.output {
        *output = normalizer.normalize(output);
    }
    report
}

fn fixture_message_view(
    report: &smith_client::message_report::MessageReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::message_report::MessageReport {
    use smith_client::message_report::MessageReport;

    let mut report = report.clone();
    let (title, message) = match &mut report {
        MessageReport::Notice { title, message }
        | MessageReport::Empty { title, message }
        | MessageReport::Error { title, message } => (title, message),
    };
    *title = normalizer.normalize(title);
    *message = normalizer.normalize(message);
    report
}

fn fixture_status_view(
    report: &smith_client::status_report::StatusReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::status_report::StatusReport {
    let mut report = report.clone();
    for value in [
        &mut report.session,
        &mut report.profile,
        &mut report.provider,
        &mut report.model,
        &mut report.permission,
        &mut report.reasoning,
        &mut report.reasoning_controls,
        &mut report.advisor,
        &mut report.prompt_cache,
        &mut report.cache_maintenance,
        &mut report.resume_checkpoint,
        &mut report.project,
        &mut report.git,
        &mut report.usage,
        &mut report.cost,
    ] {
        *value = normalizer.normalize(value);
    }
    match &mut report.goal {
        smith_client::status_report::StatusGoal::None => {}
        smith_client::status_report::StatusGoal::Unavailable(error) => {
            *error = normalizer.normalize(error);
        }
        smith_client::status_report::StatusGoal::Active(goal) => {
            for value in [
                &mut goal.objective,
                &mut goal.status,
                &mut goal.tokens,
                &mut goal.budget,
                &mut goal.active_elapsed,
                &mut goal.reason,
                &mut goal.id,
            ] {
                *value = normalizer.normalize(value);
            }
        }
    }
    report
}

fn fixture_diagnostics_view(
    report: &smith_client::diagnostics_report::DiagnosticsReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::diagnostics_report::DiagnosticsReport {
    use smith_client::diagnostics_report::DiagnosticsRow;

    let mut report = report.clone();
    for section in &mut report.sections {
        section.heading = normalizer.normalize(&section.heading);
        for row in &mut section.rows {
            match row {
                DiagnosticsRow::Field { label, value } | DiagnosticsRow::Path { label, value } => {
                    *label = normalizer.normalize(label);
                    *value = normalizer.normalize(value);
                }
                DiagnosticsRow::Line(line) => *line = normalizer.normalize(line),
            }
        }
    }
    report
}

fn fixture_context_view(
    report: &smith_client::context_report::ContextReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::context_report::ContextReport {
    let mut report = report.clone();
    for value in [
        &mut report.summary,
        &mut report.free_input.value,
        &mut report.reserve.value,
        &mut report.model_window,
        &mut report.counting,
        &mut report.tool_context,
        &mut report.provider_input,
        &mut report.cache_read,
        &mut report.cache,
        &mut report.reasoning,
        &mut report.reasoning_controls,
    ] {
        *value = normalizer.normalize(value);
    }
    for window in &mut report.available_windows {
        window.name = normalizer.normalize(&window.name);
    }
    for category in &mut report.categories {
        category.label = normalizer.normalize(&category.label);
        category.value = normalizer.normalize(&category.value);
    }
    match &mut report.compaction {
        smith_client::context_report::ContextCompaction::Enabled { recovery_target } => {
            *recovery_target = normalizer.normalize(recovery_target);
        }
        smith_client::context_report::ContextCompaction::Applied {
            summary,
            recovery_target,
        } => {
            *summary = normalizer.normalize(summary);
            *recovery_target = normalizer.normalize(recovery_target);
        }
    }
    report
}

fn fixture_help_view(
    report: &smith_client::help_report::HelpReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::help_report::HelpReport {
    let mut report = report.clone();
    report.introduction = normalizer.normalize(&report.introduction);
    for command in report
        .getting_started
        .iter_mut()
        .chain(&mut report.primary)
        .chain(&mut report.advanced)
    {
        for value in [
            &mut command.name,
            &mut command.argument_hint,
            &mut command.description,
        ] {
            *value = normalizer.normalize(value);
        }
    }
    for guidance in &mut report.composer {
        *guidance = normalizer.normalize(guidance);
    }
    for key in &mut report.keys {
        key.key = normalizer.normalize(&key.key);
        key.description = normalizer.normalize(&key.description);
    }
    report
}

fn fixture_agent_summary_view(
    summary: &mut smith_client::agent_report::AgentSummary,
    normalizer: &mut fixture_support::Normalizer,
) {
    summary.child = normalizer.normalize(&summary.child);
    if let smith_client::agent_report::ChildState::Stopped { reason } = &mut summary.state {
        *reason = normalizer.normalize(reason);
    }
}

fn fixture_agent_snapshot_view(
    snapshot: &smith_client::agent_report::AgentSnapshot,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::agent_report::AgentSnapshot {
    let mut snapshot = snapshot.clone();
    fixture_agent_summary_view(&mut snapshot.summary, normalizer);
    for value in [&mut snapshot.session, &mut snapshot.workspace] {
        *value = normalizer.normalize(value);
    }
    for value in [&mut snapshot.incompatibility, &mut snapshot.last_result]
        .into_iter()
        .flatten()
    {
        *value = normalizer.normalize(value);
    }
    snapshot
}

fn fixture_agent_view(
    report: &smith_client::agent_report::AgentReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::agent_report::AgentReport {
    use smith_client::agent_report::{AgentReport, AgentResumeReport};

    let mut report = report.clone();
    match &mut report {
        AgentReport::Empty | AgentReport::Unavailable | AgentReport::Parent => {}
        AgentReport::Missing(child) => *child = normalizer.normalize(child),
        AgentReport::List(children) => {
            for child in children {
                fixture_agent_summary_view(child, normalizer);
            }
        }
        AgentReport::Inspector(snapshot) => {
            *snapshot = fixture_agent_snapshot_view(snapshot, normalizer);
        }
        AgentReport::Resume(resume) => match resume {
            AgentResumeReport::RequiresIdle | AgentResumeReport::Unavailable => {}
            AgentResumeReport::Missing { child }
            | AgentResumeReport::Incompatible { child }
            | AgentResumeReport::Started { child } => *child = normalizer.normalize(child),
            AgentResumeReport::Failed { child, error } => {
                *child = normalizer.normalize(child);
                *error = normalizer.normalize(error);
            }
        },
    }
    report
}

fn fixture_mcp_view(
    report: &smith_client::mcp_report::McpReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::mcp_report::McpReport {
    use smith_client::mcp_report::{McpReport, McpServerState};

    let mut report = report.clone();
    match &mut report {
        McpReport::Empty { .. } | McpReport::Unavailable => {}
        McpReport::Error(error) => *error = normalizer.normalize(error),
        McpReport::Trusted { server, digest } => {
            *server = normalizer.normalize(server);
            *digest = normalizer.normalize(digest);
        }
        McpReport::Servers(servers) => {
            for server in servers {
                for value in [&mut server.name, &mut server.transport, &mut server.source] {
                    *value = normalizer.normalize(value);
                }
                if let McpServerState::Failed { reason } = &mut server.state {
                    *reason = normalizer.normalize(reason);
                }
                for rejected in &mut server.rejected {
                    *rejected = normalizer.normalize(rejected);
                }
                for value in &mut server.values {
                    value.name = normalizer.normalize(&value.name);
                    if let Some(credential) = &mut value.credential {
                        *credential = normalizer.normalize(credential);
                    }
                }
            }
        }
    }
    report
}

fn fixture_skills_view(
    report: &smith_client::skills_report::SkillsReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::skills_report::SkillsReport {
    use smith_client::skills_report::SkillsReport;

    let mut report = report.clone();
    match &mut report {
        SkillsReport::Empty => {}
        SkillsReport::Error(error) => *error = normalizer.normalize(error),
        SkillsReport::Trusted { skill, digest } => {
            *skill = normalizer.normalize(skill);
            *digest = normalizer.normalize(digest);
        }
        SkillsReport::Indexed { groups, problems } => {
            for group in groups {
                for entry in &mut group.entries {
                    entry.name = normalizer.normalize(&entry.name);
                    entry.description = normalizer.normalize(&entry.description);
                }
            }
            for problem in problems {
                for value in [&mut problem.name, &mut problem.reason, &mut problem.path] {
                    *value = normalizer.normalize(value);
                }
            }
        }
    }
    report
}

fn fixture_diff_view(
    report: &smith_client::diff_report::DiffReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::diff_report::DiffReport {
    use smith_client::diff_report::DiffOutcome;

    let mut report = report.clone();
    report.title = normalizer.normalize(&report.title);
    match &mut report.outcome {
        DiffOutcome::Empty => {}
        DiffOutcome::Error(message) => *message = normalizer.normalize(message),
        DiffOutcome::Patch(lines) => {
            for line in lines {
                line.text = normalizer.normalize(&line.text);
            }
        }
    }
    report
}

fn fixture_recovery_view(
    report: &smith_client::recovery_report::RecoveryReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::recovery_report::RecoveryReport {
    use smith_client::recovery_report::{RecoveryApplied, RecoveryReport};

    let mut report = report.clone();
    match &mut report {
        RecoveryReport::UndoConfirmation(preview) | RecoveryReport::RedoConfirmation(preview) => {
            for line in &mut preview.patch {
                line.text = normalizer.normalize(&line.text);
            }
        }
        RecoveryReport::RevertConfirmation(preview) => {
            preview.scope = normalizer.normalize(&preview.scope);
            preview.fingerprint = normalizer.normalize(&preview.fingerprint);
            for line in &mut preview.patch {
                line.text = normalizer.normalize(&line.text);
            }
        }
        RecoveryReport::PreviewError { message, .. }
        | RecoveryReport::ApplyError { message, .. } => *message = normalizer.normalize(message),
        RecoveryReport::Applied(RecoveryApplied::Revert { scope }) => {
            *scope = normalizer.normalize(scope);
        }
        RecoveryReport::RevertUsage
        | RecoveryReport::Applied(RecoveryApplied::Undo | RecoveryApplied::Redo)
        | RecoveryReport::Cancelled(_) => {}
    }
    report
}

fn fixture_review_view(
    report: &smith_client::review_report::ReviewReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::review_report::ReviewReport {
    use smith_client::review_report::{ReviewReport, ReviewStartReport};

    let mut report = report.clone();
    match &mut report {
        ReviewReport::Empty => {}
        ReviewReport::Error(message) => *message = normalizer.normalize(message),
        ReviewReport::Confirmation(preview) => {
            preview.scope = normalizer.normalize(&preview.scope);
            preview.title = normalizer.normalize(&preview.title);
            for line in &mut preview.patch {
                line.text = normalizer.normalize(&line.text);
            }
        }
        ReviewReport::Start(start) => match start {
            ReviewStartReport::Started { child } | ReviewStartReport::Queued { child } => {
                *child = normalizer.normalize(child);
            }
            ReviewStartReport::Failed(error) => *error = normalizer.normalize(error),
            ReviewStartReport::Unavailable | ReviewStartReport::AtCapacity { .. } => {}
        },
    }
    report
}

fn fixture_goal_view(
    report: &smith_client::goal_report::GoalReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::goal_report::GoalReport {
    use smith_client::goal_report::GoalReport;

    let mut report = report.clone();
    match &mut report {
        GoalReport::Empty | GoalReport::Cleared => {}
        GoalReport::Unavailable(error) => {
            *error = normalizer.normalize(error);
        }
        GoalReport::Snapshot(goal) => {
            for value in [
                &mut goal.objective,
                &mut goal.status,
                &mut goal.usage_provenance,
                &mut goal.active_elapsed,
                &mut goal.id,
            ] {
                *value = normalizer.normalize(value);
            }
            if let Some(reason) = &mut goal.stopped_reason {
                reason.code = normalizer.normalize(&reason.code);
                if let Some(detail) = &mut reason.detail {
                    *detail = normalizer.normalize(detail);
                }
            }
        }
    }
    report
}

fn fixture_timeline_view(
    report: &smith_client::timeline_report::TimelineReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::timeline_report::TimelineReport {
    use smith_client::timeline_report::{TimelineChildEvent, TimelineEntry, TimelineReport};

    let mut report = report.clone();
    match &mut report {
        TimelineReport::Empty => {}
        TimelineReport::Unavailable(error) => {
            *error = normalizer.normalize(error);
        }
        TimelineReport::Entries(entries) => {
            for entry in entries {
                match entry {
                    TimelineEntry::RootTurn { turn, finish, .. } => {
                        *turn = normalizer.normalize(turn);
                        *finish = normalizer.normalize(finish);
                    }
                    TimelineEntry::RootManifest {
                        turn,
                        provider,
                        model,
                        ..
                    } => {
                        for value in [turn, provider, model] {
                            *value = normalizer.normalize(value);
                        }
                    }
                    TimelineEntry::ChildEvent { child, event } => {
                        *child = normalizer.normalize(child);
                        match event {
                            TimelineChildEvent::Started { workspace, .. } => {
                                *workspace = normalizer.normalize(workspace);
                            }
                            TimelineChildEvent::Stopped { reason } => {
                                *reason = normalizer.normalize(reason);
                            }
                            TimelineChildEvent::NeedsInput
                            | TimelineChildEvent::Completed
                            | TimelineChildEvent::Failed => {}
                        }
                    }
                    TimelineEntry::ChildSnapshot {
                        child,
                        session,
                        durability,
                        state,
                        turns,
                        ..
                    } => {
                        for value in [child, session, durability, state, turns] {
                            *value = normalizer.normalize(value);
                        }
                    }
                    TimelineEntry::Recovery { detail, .. } => {
                        *detail = normalizer.normalize(detail);
                    }
                }
            }
        }
    }
    report
}

pub(super) fn fixture_screen(app: &App, width: u16) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 512))
        .expect("test terminal");
    terminal
        .draw(|frame| smith_tui::draw(frame, app, smith_tui::Theme::new().without_color()))
        .expect("frame");
    let buffer = terminal.backend().buffer();
    let rows = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>();
    format!("{}\n", rows.join("\n").trim_matches('\n'))
}

pub(super) fn fixture_record(name: &str, app: &App, normalizer: &mut fixture_support::Normalizer) {
    // A grouped scenario may need replay to reach one report. Limit writes
    // during re-recording without changing the scenario or other goldens.
    if std::env::var_os("SMITH_UPDATE_FIXTURES").is_some_and(|value| value == "1")
        && let Ok(names) = std::env::var("SMITH_FIXTURE_NAMES")
        && !names.split(',').any(|selected| selected == name)
    {
        return;
    }
    let (raw, view) = fixture_raw_and_view(app, normalizer);
    fixture_support::compare_or_update(&format!("local-commands/{name}.raw.txt"), &raw);
    for width in [100, 44] {
        fixture_support::compare_or_update(
            &format!("local-commands/{name}.w{width}.txt"),
            &fixture_screen(&view, width),
        );
    }
}

// Each command group owns its home, project, and host. Box the host/command
// futures at their call sites so their large runtime state never accumulates
