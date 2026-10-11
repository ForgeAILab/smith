use super::usage::resolve_catalog_price;
use super::{
    App, AppBinding, CounterKind, GitChanges, HostSession, InteractiveResources, NoticeKind,
    PresentationOptions, RestoreReport, abbreviate_home, account_status, child_summary_projection,
    last_session_usage, resolve_price, restore_transcript, restore_usage_with_bindings,
    runtime_resources,
};

pub(super) async fn seed_app(
    host: &HostSession,
    project: &std::path::Path,
    resources: &InteractiveResources,
    presentation: &PresentationOptions,
) -> App {
    let policy = host.runtime().policy();
    let snapshot = host.snapshot();
    let mut app = App::new(policy.model.as_str(), project_label(project));
    restore_transcript(host, &mut app, &snapshot.history);
    seed_host_state(host, &mut app, project, resources, presentation).await;
    if policy.reasoning.has_override() {
        app.transcript.push_notice(
            NoticeKind::Reasoning,
            format!(
                "thinking {} · effort {} · {} · applies to the next turn",
                policy.reasoning.effective_state(),
                policy.reasoning.effective_effort(),
                policy.reasoning.selection_source,
            ),
        );
    }
    if let Some(notice) = presentation.reasoning_notice.as_ref() {
        app.transcript.push_notice(NoticeKind::Reasoning, notice);
    }
    if let Some(notice) = presentation.host_notice.as_ref() {
        app.transcript.push_notice(NoticeKind::Provider, notice);
    }
    if let Some(previous) = snapshot.manifests.last().map(|entry| &entry.manifest.model)
        && (previous.provider != policy.provider_name || previous.model != policy.model)
    {
        app.transcript.push_notice(
            NoticeKind::Provider,
            format!(
                "changed · {}/{} → {}/{} · prior cache not transferable",
                previous.provider, previous.model, policy.provider_name, policy.model
            ),
        );
    }
    // The shared seed replayed status; only a new app presents its cache notice.
    app.restore_cache_events(std::iter::empty());
    app
}

pub(super) async fn rebind_app(
    host: &HostSession,
    app: &mut App,
    previous: &AppBinding,
    project: &std::path::Path,
    resources: &InteractiveResources,
    presentation: &PresentationOptions,
) {
    app.rebind_host();
    seed_host_state(host, app, project, resources, presentation).await;
    if let Some(notice) = presentation.host_notice.as_ref() {
        app.transcript.push_notice(NoticeKind::Provider, notice);
    }
    let policy = host.runtime().policy();
    if previous.provider != policy.provider_name || previous.model != policy.model.as_str() {
        app.transcript.push_notice(
            NoticeKind::Provider,
            format!(
                "changed · {}/{} → {}/{} · prior cache not transferable",
                previous.provider, previous.model, policy.provider_name, policy.model
            ),
        );
    }
    if let Some(notice) = presentation.reasoning_notice.as_ref() {
        app.transcript.push_notice(NoticeKind::Reasoning, notice);
    } else if previous.reasoning.effective_state() != policy.reasoning.effective_state()
        || previous.reasoning.effective_effort() != policy.reasoning.effective_effort()
        || previous.reasoning.selected_enabled != policy.reasoning.selected_enabled
        || previous.reasoning.selected_effort != policy.reasoning.selected_effort
    {
        app.transcript.push_notice(
            NoticeKind::Reasoning,
            format!(
                "thinking {} · effort {} · {} · applies to the next turn",
                policy.reasoning.effective_state(),
                policy.reasoning.effective_effort(),
                policy.reasoning.selection_source,
            ),
        );
    }
    if previous.context_window != policy.context_window {
        app.transcript.push_notice(
            NoticeKind::Context,
            format!(
                "window changed · {} → {}",
                previous
                    .context_window
                    .as_deref()
                    .unwrap_or("model default"),
                policy.context_window.as_deref().unwrap_or("model default"),
            ),
        );
    }
    if previous.profile != policy.agent_profile {
        app.transcript.push_notice(
            NoticeKind::Profile,
            format!("changed · {} → {}", previous.profile, policy.agent_profile),
        );
    }
}

fn project_label(project: &std::path::Path) -> String {
    GitChanges::discover(project)
        .and_then(|git| git.branch_label())
        .map_or_else(
            |_| abbreviate_home(&project.to_string_lossy()),
            |branch| format!("{}:{branch}", abbreviate_home(&project.to_string_lossy())),
        )
}

/// Rebuilds every host-backed projection from the same durable sources.
async fn seed_host_state(
    host: &HostSession,
    app: &mut App,
    project: &std::path::Path,
    resources: &InteractiveResources,
    presentation: &PresentationOptions,
) {
    let policy = host.runtime().policy();
    let snapshot = host.snapshot();
    let previous_usage = app.status.session_usage();
    app.status = smith_tui::status::Status::new(policy.model.as_str(), project_label(project));
    app.set_cache_miss_notices(presentation.cache_miss_notices);
    app.set_module_status(smith_client::status::module_status(host.runtime()));
    app.status
        .switch_model(Some(policy.provider_name.clone()), policy.model.as_str());
    app.status
        .set_price(resolve_price(policy, &resources.catalog));
    app.status
        .set_advisor_price(host.runtime().advisor_route().and_then(|advisor| {
            advisor.price.as_ref().map(|price| {
                smith_client::status::PriceReference::from_catalog(
                    &advisor.provider_name,
                    advisor.model.as_str(),
                    price,
                )
            })
        }));
    app.status.set_agent(policy.agent_profile.clone());
    app.status.approval_mode = Some(policy.approval_mode.as_str().to_owned());
    // Labels the turn as executed by an installed CLI, and switches the model
    // picker to that CLI's models rather than the provider catalog.
    app.status.harness = policy
        .harness
        .as_ref()
        .map(|harness| harness.kind.value.clone());
    match host.goal() {
        Ok(goal) => app.status.set_goal(goal),
        Err(error) => app
            .transcript
            .push_error(format!("persistent goal state unavailable: {error}")),
    }
    app.status
        .set_reasoning_hint(policy.reasoning.has_override().then(|| {
            format!(
                "think {} · effort {}",
                policy.reasoning.effective_state(),
                policy.reasoning.effective_effort(),
            )
        }));
    app.status.advisor = Some(crate::resources::advisor_status(&resources.agents));
    let mut runtime_resources = runtime_resources(
        resources.inventory.clone(),
        resources.sessions.clone(),
        host.session().id().as_str(),
        project,
        &resources.agents,
        &policy.reasoning,
        &policy.context_windows,
        policy.context_window.as_deref(),
        resources.credential_pool.as_ref(),
        policy.harness.as_ref(),
    );
    runtime_resources.capability_denials = resources.capability_denials.clone();
    app.set_resources(runtime_resources);
    app.set_command_catalog(resources.commands.catalog());
    app.status.account = account_status(resources.credential_pool.as_ref());
    let children = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .map_or_else(Vec::new, |coordinator| {
            coordinator
                .list()
                .into_iter()
                .map(|child| {
                    let (state, detail) = child_summary_projection(&child);
                    (child.child.as_str().to_owned(), state, Some(detail))
                })
                .collect()
        });
    app.replace_children(children);
    if let Some(turn) = host.session().interrupted_on_resume() {
        let report = RestoreReport::ActivationChanged {
            turn: turn.to_string(),
        };
        if let Some(text) = smith_client::recovery_report::render_restore_plain(&report) {
            app.transcript.push_notice(report.notice_kind(), text);
        }
    }
    if let Some(interruption) = host.recovered_ephemeral_work() {
        app.present_recovered_ephemeral_work(
            interruption.children.len(),
            interruption.monitors.len(),
            interruption.tasks.len(),
        );
    }
    let snapshot_cache_read = if snapshot.usage.records().is_empty() {
        None
    } else {
        let cache_read = snapshot.usage.total().get(CounterKind::InputCached);
        (cache_read > 0).then_some(cache_read)
    };
    // Restore canonical records individually. The aggregate ledger total is
    // intentionally not suitable for seeding the TUI: synthetic cache work
    // (keepalives, handoffs, idle summaries, …) is part of that total but
    // must stay out of ordinary turn/context accounting.
    let logged_usage = host
        .paths()
        .and_then(|paths| last_session_usage(paths, host.session().id(), snapshot.usage.records()));
    app.status.restore_turn_count(snapshot.identity.turn);
    restore_usage_with_bindings(
        &mut app.status,
        snapshot.usage.records(),
        logged_usage.as_ref(),
        snapshot.manifests.iter().map(|entry| {
            let model = &entry.manifest.model;
            (model.provider.as_str(), model.model.as_str())
        }),
        |provider, model| {
            if provider == policy.provider_name && model == policy.model.as_str() {
                resolve_price(policy, &resources.catalog)
            } else {
                let catalog_provider = resources
                    .inventory
                    .models
                    .iter()
                    .find(|entry| entry.provider == provider && entry.model == model)
                    .and_then(|entry| entry.catalog_provider.as_deref());
                resolve_catalog_price(provider, model, catalog_provider, &resources.catalog)
            }
        },
    );
    // In-process rebinds still know each turn's binding. A process resume has
    // only identity-free records, so the log or manifest supplies attribution.
    app.status.retain_usage_bindings(&previous_usage);
    if let Ok(events) = host.client_timeline_events().await {
        app.status.replay_cache_events(events);
    }
    // Old snapshots only carry an aggregate positive cached-input counter.
    // Keep it as a legacy fallback after replay, and never let it override or
    // double-count any attributed canonical projection recovered above.
    if let Some(cache_read) = snapshot_cache_read
        && app
            .status
            .cache_projection
            .session_observed_read()
            .is_none()
    {
        app.status.record_cache(cache_read);
    }
}
