//! Interactive terminal event loop and TUI action routing.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::ids::ChildId;
use agent_runtime_core::usage::{CounterKind, UsageRecord};
use anyhow::{Context, Result};
use crossterm::event::{Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use smith_client::NoticeKind;
use smith_client::agent_report::AgentSnapshot;
use smith_client::commands::{SelectionCommand, SessionControl};
use smith_client::local_result::LocalResult;
use smith_client::recovery_report::{RecoveryAction, RecoveryReport, RestoreReport};
use smith_config::inventory::SelectionInventory;
use smith_config::resolve::ResolvedAgent;
use smith_host::{
    ApprovalPrompt, ApprovalRequests, GitChanges, InteractionRequests, RotationPrompt,
    RotationRequests,
};
use smith_runtime::client::{
    ChildPhase, SmithEvent as EventEnvelope, SmithEventKind as RuntimeEvent,
};
use smith_runtime::factory::RuntimePolicy;
use smith_runtime::host::HostSession;
use smith_runtime::pool_state::ActiveAccounts;
use smith_runtime::rotation::SharedPool;
use smith_runtime::session::SessionListing;
use smith_tui::app::{Action, App, MouseOutcome, RunningTaskSummary, SubmissionTarget};
use smith_tui::theme::Theme;

use crate::local_command::{LocalOutcome, handle_local_command, tool_call_for_display};
use crate::resources::{abbreviate_home, account_entries, account_status, runtime_resources};
use crate::submission::{
    LocalShellApprovals, LocalShellIdentity, attach_from_clipboard, child_summary_projection,
    copy_selection_to_clipboard, dispatch_prepared_with_materialization, follow_up_agent,
    resume_agent, start_agent, start_local_shell, start_review,
};
use crate::{FRAME, SPINNER_TICK, interaction, local_command, terminal};

pub(super) enum InteractiveExit {
    /// A declared MCP server finished connecting, so the composed tool set is
    /// stale. The session is rebuilt around the same identity at the next idle
    /// boundary; the connections themselves survive it.
    CapabilitiesChanged,
    /// The session's usage, plus the price reference it was resolved against
    /// (if any) — carried out here because the exit report needs the identical
    /// reference `/status` read from during the session rather than a fresh
    /// catalog lookup of its own.
    Quit(
        Box<smith_client::status::SessionUsage>,
        Option<smith_client::status::PriceReference>,
        Option<Box<smith_client::cache::CacheTurnSummary>>,
    ),
    Reconfigure(SelectionCommand),
    Connect(String),
    Disconnect(String),
}

pub(super) struct PresentationOptions {
    pub(super) no_color: bool,
    pub(super) no_motion: bool,
    pub(super) reasoning_notice: Option<String>,
    pub(super) cache_miss_notices: bool,
}

pub(super) struct InteractiveResources {
    pub(super) inventory: SelectionInventory,
    pub(super) agents: ResolvedAgent,
    pub(super) sessions: Vec<SessionListing>,
    /// Live credential-pool state, when the provider declares a pool.
    pub(super) credential_pool: Option<SharedPool>,
    /// The catalog snapshot backing this session, so the active model's
    /// price can be resolved with the exact binding the runtime factory
    /// used. See `resolve_price`.
    pub(super) catalog: Arc<smith_config::catalog::CatalogSnapshot>,
    /// Declared MCP servers and their connections, when any are declared.
    pub(super) mcp: Option<Arc<crate::mcp::McpContext>>,
    /// The skills this composition indexed, and the files it refused.
    pub(super) skills: Arc<crate::skills::SkillContext>,
}

/// The runtime's out-of-band request streams, plus the accounts they rotate
/// between. Each stream is absent when the surface was started without that
/// capability.
pub(super) struct InteractiveRequests<'a> {
    /// Borrow process input so shutdown and composition cannot replace its reader.
    pub(super) keys: &'a mut crate::screen_runner::TerminalEvents,
    pub(super) approvals: Option<ApprovalRequests>,
    pub(super) interactions: Option<InteractionRequests>,
    pub(super) rotations: Option<RotationRequests>,
    pub(super) accounts: ActiveAccounts,
}

#[derive(Default)]
pub(super) struct ShellShortcuts {
    anchors: BTreeMap<u64, usize>,
}

impl ShellShortcuts {
    pub(super) fn dispatched(&mut self, host: &HostSession, echo: u64) {
        let anchor = host.session().with_history(|history| history.len());
        self.anchors.insert(echo, anchor);
    }

    pub(super) fn finish(
        &mut self,
        host: &HostSession,
        app: &mut App,
        echo: u64,
        call: Option<&str>,
        content: &str,
        is_error: bool,
    ) {
        let result = app
            .transcript
            .finish_shell_shortcut(echo, call, is_error, content);
        if let Some(anchor) = self.anchors.remove(&echo)
            && let Some(command) = app.transcript.shell_shortcut_command(echo)
        {
            host.record_shell_shortcut(anchor, call, command, is_error, result.as_deref());
        }
    }
}

pub(super) fn restore_transcript(
    host: &HostSession,
    app: &mut App,
    history: &[agent_runtime_core::content::Message],
) {
    let shortcuts = host
        .saved_shell_shortcuts()
        .into_iter()
        .map(|shortcut| smith_tui::transcript::RestoredShellShortcut {
            anchor: shortcut.anchor,
            call: shortcut.call,
            command: shortcut.command,
            is_error: shortcut.is_error,
            result: shortcut.result,
        })
        .collect::<Vec<_>>();
    app.transcript
        .replace_from_history_with_shell_shortcuts(history, &shortcuts);
    for (call, display) in host.tool_call_displays() {
        app.set_tool_display(call.as_str(), display);
    }
    for (call, text) in host.tool_result_texts() {
        app.set_tool_result_preview(call.as_str(), text);
    }
}

/// The application and the binding it last displayed, retained between hosts.
pub(super) struct InteractiveApp {
    pub(super) app: App,
    session: String,
    binding: AppBinding,
}

struct AppBinding {
    provider: String,
    model: String,
    profile: String,
    reasoning: smith_runtime::reasoning::ReasoningRuntimePolicy,
    context_window: Option<String>,
}

impl AppBinding {
    fn from_host(host: &HostSession) -> Self {
        let policy = host.runtime().policy();
        Self {
            provider: policy.provider_name.clone(),
            model: policy.model.as_str().to_owned(),
            profile: policy.agent_profile.clone(),
            reasoning: policy.reasoning.clone(),
            context_window: policy.context_window.clone(),
        }
    }
}

/// Selects a fresh seed or a same-session rebind without touching the terminal.
pub(super) async fn prepare_interactive_app(
    host: &HostSession,
    project: &std::path::Path,
    resources: &InteractiveResources,
    presentation: &PresentationOptions,
    previous: Option<InteractiveApp>,
) -> InteractiveApp {
    let session = host.session().id().as_str().to_owned();
    let binding = AppBinding::from_host(host);
    let app = match previous {
        Some(mut previous) if previous.session == session => {
            rebind_app(
                host,
                &mut previous.app,
                &previous.binding,
                project,
                resources,
                presentation,
            )
            .await;
            previous.app
        }
        previous => {
            let mut app = seed_app(host, project, resources, presentation).await;
            if let Some(mut previous) = previous {
                app.inherit_composer_history(&mut previous.app);
            }
            app
        }
    };
    InteractiveApp {
        app,
        session,
        binding,
    }
}

async fn seed_app(
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

async fn rebind_app(
    host: &HostSession,
    app: &mut App,
    previous: &AppBinding,
    project: &std::path::Path,
    resources: &InteractiveResources,
    presentation: &PresentationOptions,
) {
    app.rebind_host();
    seed_host_state(host, app, project, resources, presentation).await;
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
    app.set_resources(runtime_resources(
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
    ));
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

pub(super) async fn run_interactive(
    terminal: &mut terminal::Terminal,
    previous: Option<InteractiveApp>,
    host: &HostSession,
    requests: InteractiveRequests<'_>,
    project: &std::path::Path,
    resources: InteractiveResources,
    presentation: PresentationOptions,
) -> Result<(InteractiveExit, InteractiveApp)> {
    let InteractiveApp {
        app,
        session,
        binding,
    } = prepare_interactive_app(host, project, &resources, &presentation, previous).await;
    let InteractiveRequests {
        keys,
        approvals,
        interactions,
        rotations,
        accounts,
    } = requests;
    let InteractiveResources {
        agents,
        catalog,
        inventory,
        credential_pool,
        mcp,
        skills,
        ..
    } = resources;
    let theme =
        crate::screen_runner::theme_from_flags(presentation.no_color, presentation.no_motion);
    let mut run_result = run_tui(
        terminal,
        app,
        TuiRunInputs {
            keys,
            host,
            project,
            approvals,
            interactions,
            rotations,
            accounts,
            credential_pool,
            agents: &agents,
            catalog: &catalog,
            inventory: &inventory,
            theme,
            mcp,
            skills,
        },
    )
    .await;
    let shutdown_result = host
        .shutdown()
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("shutting the session down");

    if let Ok((InteractiveExit::Quit(usage, ..), _)) = &mut run_result {
        let snapshot = host.snapshot();
        usage.reconcile_synthetic_records(snapshot.usage.records());
        usage.reconcile_advisor_records(snapshot.usage.records());
    }

    shutdown_result?;
    run_result.map(|(exit, app)| {
        (
            exit,
            InteractiveApp {
                app,
                session,
                binding,
            },
        )
    })
}

/// A later unattributed record must not hide matching per-model counters.
fn last_session_usage(
    paths: &smith_runtime::session::SessionPaths,
    session: &agent_runtime_core::ids::SessionId,
    records: &[UsageRecord],
) -> Option<smith_client::usage_log::SessionUsageRecord> {
    let mut restored = smith_client::status::Status::new("", "");
    restore_usage_records(&mut restored, records);
    let totals = restored
        .session_usage()
        .totals
        .iter()
        .map(|(kind, value)| {
            (
                smith_client::status::counter_label(*kind).to_owned(),
                *value,
            )
        })
        .collect::<BTreeMap<_, _>>();
    smith_client::usage_log::read_all(&smith_client::usage_log::default_path(paths.directory()))
        .into_iter()
        .rev()
        .find(|record| {
            record.session == session.as_str()
                && record.totals == totals
                && usage_bindings_are_attributed(record)
        })
}

fn usage_bindings_are_attributed(record: &smith_client::usage_log::SessionUsageRecord) -> bool {
    record.schema_version == 5
        && !record.bindings.is_empty()
        && record.bindings.iter().all(|binding| {
            binding
                .provider
                .as_deref()
                .is_some_and(|provider| !provider.trim().is_empty())
                && !binding.model.trim().is_empty()
                && binding.model != "earlier models"
        })
}

/// Seeds the status projection from durable Runtime records without losing
/// their typed provenance. In particular, synthetic cache attempts count
/// toward session spend but never become an ordinary user turn.
fn restore_usage_records(status: &mut smith_client::status::Status, records: &[UsageRecord]) {
    for record in records {
        status.record_usage_record(record);
    }
}

/// A matching v5 log preserves model switches that identity-free runtime
/// records cannot recover. Stale or older logs use the single-manifest rule
/// rather than assigning all earlier usage to the last model.
fn restore_usage_with_bindings<'a>(
    status: &mut smith_client::status::Status,
    records: &[UsageRecord],
    logged: Option<&smith_client::usage_log::SessionUsageRecord>,
    manifests: impl IntoIterator<Item = (&'a str, &'a str)>,
    mut price_for: impl FnMut(&str, &str) -> Option<smith_client::status::PriceReference>,
) {
    if records.is_empty() {
        return;
    }
    let provider = status.provider.clone();
    let model = status.model.clone();
    let price = status.price().cloned();
    let bindings = manifests
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if bindings.len() == 1
        && let Some((provider, model)) = bindings.iter().next()
    {
        status.switch_model(Some((*provider).to_owned()), *model);
        status.set_price(price_for(provider, model));
    } else {
        status.switch_model(None, "earlier models");
    }
    restore_usage_records(status, records);
    if let Some(logged) = logged {
        restore_logged_bindings(status, logged, &mut price_for);
    }
    if status.provider != provider || status.model != model {
        status.switch_model(provider, model);
        status.set_price(price);
    }
}

/// Checks the root rollup and the binding partition before using the log:
/// v4's synthesized last-model bucket is not evidence of earlier attribution.
fn restore_logged_bindings(
    status: &mut smith_client::status::Status,
    logged: &smith_client::usage_log::SessionUsageRecord,
    price_for: &mut impl FnMut(&str, &str) -> Option<smith_client::status::PriceReference>,
) {
    use smith_client::status::{BindingUsage, counter_label};
    let mut restored = status.session_usage();
    let totals = restored
        .totals
        .iter()
        .map(|(kind, value)| (counter_label(*kind).to_owned(), *value))
        .collect::<std::collections::BTreeMap<_, _>>();
    if logged.totals != totals || !usage_bindings_are_attributed(logged) {
        return;
    }
    let mut partition = std::collections::BTreeMap::<String, u64>::new();
    for binding in &logged.bindings {
        for (kind, value) in &binding.totals {
            let total = partition.entry(kind.clone()).or_default();
            let Some(sum) = total.checked_add(*value) else {
                return;
            };
            *total = sum;
        }
    }
    let mut expected = totals;
    for (kind, value) in &logged.delegated_totals {
        let total = expected.entry(kind.clone()).or_default();
        let Some(sum) = total.checked_add(*value) else {
            return;
        };
        *total = sum;
    }
    if partition != expected {
        return;
    }
    let kinds = [
        CounterKind::InputUncached,
        CounterKind::InputCached,
        CounterKind::CacheWrite,
        CounterKind::Output,
        CounterKind::Reasoning,
    ];
    let mut bindings = Vec::new();
    for entry in &logged.bindings {
        let mut binding = BindingUsage::new(entry.provider.clone(), &entry.model, None);
        for (label, value) in &entry.totals {
            let Some(kind) = kinds.iter().find(|kind| counter_label(**kind) == label) else {
                return;
            };
            binding.totals.insert(*kind, *value);
        }
        binding.reported = logged.reported && restored.bindings.iter().all(|entry| entry.reported);
        bindings.push(binding);
    }
    for binding in &mut bindings {
        binding.price = binding
            .provider
            .as_deref()
            .and_then(|provider| price_for(provider, &binding.model));
    }
    restored.bindings = bindings;
    status.retain_usage_bindings(&restored);
}

/// Resolves the active model's catalog price, using **exactly** the binding
/// the runtime factory itself resolves models against — this mirrors
/// `crates/smith-runtime/src/factory.rs`'s `prepare_factory_inputs` catalog
/// lookup line for line, rather than inventing a second resolution that
/// could disagree with it and price the wrong model.
///
/// Returns `None` when the catalog carries no price entry for this binding.
/// That is never treated as "assume some other price" anywhere downstream —
/// `usage-accounting`'s "Labelled cost calculation" forbids substituting a
/// price from another model, provider, or a hard-coded default, and a
/// `None` here is exactly how that absence is represented.
pub(super) fn resolve_price(
    policy: &RuntimePolicy,
    catalog: &smith_config::catalog::CatalogSnapshot,
) -> Option<smith_client::status::PriceReference> {
    let catalog_provider = smith_config::catalog::catalog_provider_for(
        &policy.provider_kind,
        policy.endpoint.as_deref(),
    );
    resolve_catalog_price(
        &policy.provider_name,
        policy.model.as_str(),
        catalog_provider,
        catalog,
    )
}

fn resolve_catalog_price(
    provider: &str,
    model: &str,
    catalog_provider: Option<&str>,
    catalog: &smith_config::catalog::CatalogSnapshot,
) -> Option<smith_client::status::PriceReference> {
    let cost = catalog_provider
        .and_then(|provider| catalog.provider(provider))
        .and_then(|provider| provider.models.get(model))
        .and_then(|model| model.cost.as_ref())?;
    Some(smith_client::status::PriceReference::from_catalog(
        provider, model, cost,
    ))
}

fn resolve_child_usage_binding(
    profile: &str,
    inventory: &SelectionInventory,
    catalog: &smith_config::catalog::CatalogSnapshot,
) -> Option<smith_client::status::BindingUsage> {
    // The inventory is frozen with this host's config/catalog. Its catalog
    // provider already encodes the exact adapter/endpoint pairing, including
    // local aliases; rereading config at spawn could disagree with the runtime.
    let profile = inventory.profiles.iter().find(|entry| {
        entry.name == profile && entry.uses.contains(&smith_config::model::ProfileUse::Child)
    })?;
    let provider = profile.provider.as_deref()?;
    let model = profile.model.as_deref()?;
    let catalog_provider = inventory
        .models
        .iter()
        .find(|entry| entry.provider == provider && entry.model == model)
        .and_then(|entry| entry.catalog_provider.as_deref());
    let price = resolve_catalog_price(provider, model, catalog_provider, catalog);
    Some(smith_client::status::BindingUsage::new(
        Some(provider.to_owned()),
        model,
        price,
    ))
}

/// Forwards one live child's own event stream into the client's single event
/// loop, tagged with the child it belongs to.
///
/// A child is a full runtime session, and this is how the client watches one:
/// the same events, the same fold, a different transcript. Presentation only —
/// the child's lifecycle, budget, and result delivery stay with the runtime's
/// coordinator, which is the authority for them.
///
/// A dormant child has no live stream, so this is a no-op for one recovered
/// from a durable record; its canonical history is what the inspector shows
/// instead. The forwarding task ends with the child's stream.
fn subscribe_to_child(
    host: &HostSession,
    child: &ChildId,
    events: tokio::sync::mpsc::UnboundedSender<(ChildId, EventEnvelope)>,
) {
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
    else {
        return;
    };
    let Some(mut stream) = coordinator.child_events(child) else {
        return;
    };
    let child = child.clone();
    tokio::spawn(async move {
        while let Some(envelope) = stream.next().await {
            if events
                .send((
                    child.clone(),
                    smith_runtime::client::SmithEvent::project_or_unknown(&envelope),
                ))
                .is_err()
            {
                break;
            }
        }
    });
}

pub(super) struct TuiRunInputs<'a> {
    keys: &'a mut crate::screen_runner::TerminalEvents,
    host: &'a HostSession,
    project: &'a std::path::Path,
    approvals: Option<ApprovalRequests>,
    interactions: Option<InteractionRequests>,
    rotations: Option<RotationRequests>,
    accounts: ActiveAccounts,
    credential_pool: Option<SharedPool>,
    agents: &'a ResolvedAgent,
    catalog: &'a smith_config::catalog::CatalogSnapshot,
    inventory: &'a SelectionInventory,
    theme: Theme,
    mcp: Option<Arc<crate::mcp::McpContext>>,
    skills: Arc<crate::skills::SkillContext>,
}

struct TuiLoop<'a> {
    app: App,
    host: &'a HostSession,
    project: &'a std::path::Path,
    approvals: Option<ApprovalRequests>,
    rotations: Option<RotationRequests>,
    accounts: ActiveAccounts,
    credential_pool: Option<SharedPool>,
    agents: &'a ResolvedAgent,
    catalog: &'a smith_config::catalog::CatalogSnapshot,
    inventory: &'a SelectionInventory,
    theme: Theme,
    mcp: Option<Arc<crate::mcp::McpContext>>,
    skills: Arc<crate::skills::SkillContext>,
    session: &'a smith_runtime::SessionHandle,
    events: smith_runtime::client::SmithEventStream,
    keys: &'a mut crate::screen_runner::TerminalEvents,
    spinner: tokio::time::Interval,
    frame: tokio::time::Interval,
    local_tx: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
    local_rx: tokio::sync::mpsc::UnboundedReceiver<LocalOutcome>,
    local_shell_approvals: LocalShellApprovals,
    shell_shortcuts: ShellShortcuts,
    child_tx: tokio::sync::mpsc::UnboundedSender<(ChildId, EventEnvelope)>,
    child_rx: tokio::sync::mpsc::UnboundedReceiver<(ChildId, EventEnvelope)>,
    mcp_changes: Option<tokio::sync::watch::Receiver<u64>>,
    composed_remote_tools: usize,
    remote_tools_pending: bool,
    trusted_skill_pending: bool,
    last_change_turn: Option<u64>,
    interactions: interaction::InteractionSurface,
    dirty: bool,
    window_title: smith_tui::terminal_title::TerminalTitleState,
}

pub(super) async fn run_tui(
    terminal: &mut terminal::Terminal,
    app: App,
    inputs: TuiRunInputs<'_>,
) -> Result<(InteractiveExit, App)> {
    run_tui_on(terminal.inner_mut(), TuiLoop::new(app, inputs)).await
}

async fn run_tui_on<B>(
    terminal: &mut ratatui::Terminal<B>,
    mut tui: TuiLoop<'_>,
) -> Result<(InteractiveExit, App)>
where
    B: ratatui::backend::Backend,
    B::Error: Send + Sync + 'static,
{
    let exit = loop {
        tokio::select! {
            // Keyboard first: a provider flood must not starve cancellation.
            biased;

            Some(key) = tui.keys.next() => {
                if let Some(exit) = tui.on_terminal_event(terminal, key).await? {
                    break exit;
                }
            }

            prompt = next_approval(&mut tui.approvals) => {
                tui.on_approval(prompt);
            }

            offer = next_rotation(&mut tui.rotations) => {
                tui.on_rotation(offer);
            }

            notice = tui.interactions.next_notice() => {
                tui.on_interaction_notice(notice);
            }

            envelope = tui.events.next() => {
                if let Some(exit) = tui.on_runtime_event(envelope).await {
                    break exit;
                }
            }

            child_event = tui.child_rx.recv() => {
                tui.on_child_event(child_event);
            }

            outcome = tui.local_rx.recv() => {
                tui.on_local_result(outcome);
            }

            () = async {
                match &mut tui.mcp_changes {
                    Some(receiver) => {
                        let _ = receiver.changed().await;
                    }
                    // No declared server: this arm never completes, and the
                    // loop behaves exactly as it did before MCP existed.
                    None => std::future::pending().await,
                }
            } => {
                tui.on_mcp_change();
            }
            _ = tui.spinner.tick() => {
                tui.on_spinner();
            }

            _ = tui.frame.tick(), if tui.dirty || tui.remote_tools_pending || tui.trusted_skill_pending => {
                if let Some(exit) = tui.on_frame(terminal).await? {
                    break exit;
                }
            }
        }

        if let Some(exit) = tui.after_event() {
            break exit;
        }
    };
    // A normal exit clears the title exactly once, ahead of a new host's
    // tracker or the caller's terminal restore. An I/O error through `?` skips
    // this and leaves the last title standing until the shell's own prompt hook
    // reasserts its title on the next prompt -- that same hook is why
    // restoring a remembered pre-session title is deliberately not attempted.
    let _ = tui.window_title.clear();
    Ok((exit, tui.app))
}

impl<'a> TuiLoop<'a> {
    fn new(app: App, inputs: TuiRunInputs<'a>) -> Self {
        let TuiRunInputs {
            keys,
            host,
            project,
            approvals,
            interactions,
            rotations,
            accounts,
            credential_pool,
            agents,
            catalog,
            inventory,
            theme,
            mcp,
            skills,
        } = inputs;
        let session = host.session();
        let events = host.client().events();
        let spinner = tokio::time::interval(SPINNER_TICK);
        let frame = tokio::time::interval(FRAME);
        let (local_tx, local_rx) = tokio::sync::mpsc::unbounded_channel();
        let local_shell_approvals = LocalShellApprovals::default();
        let shell_shortcuts = ShellShortcuts::default();
        // One forwarding task per live child funnels every child's own stream into
        // this loop, so a child's events are folded by the same single-threaded
        // reducer the root's are and can never interleave mid-fold.
        let (child_tx, child_rx) =
            tokio::sync::mpsc::unbounded_channel::<(ChildId, EventEnvelope)>();
        let mcp_changes = mcp.as_ref().map(|context| context.supervisor().subscribe());
        // What the runtime was composed with. A rebuild is worth its cost only
        // when a server has actually contributed something new since.
        let composed_remote_tools = mcp
            .as_ref()
            .map_or(0, |context| context.supervisor().tools().len());
        let remote_tools_pending = false;
        // A newly trusted project skill is only in the trust file until the
        // catalog is resolved again, and the catalog is resolved at composition.
        let trusted_skill_pending = false;
        let last_change_turn = host.changes().latest().map(|set| set.turn);
        let interactions = interaction::InteractionSurface::new(
            interactions,
            host.restored_interaction()
                .map(|restored| restored.request_id().as_str().to_owned()),
        );
        let dirty = true;
        // The terminal window title follows the same state the header does, but
        // is written outside the frame: it is one OSC sequence per *change*, not
        // per draw. Guarded and deduped inside the tracker, so a non-terminal
        // stdout (or an unchanged title) costs nothing.
        let mut window_title = smith_tui::terminal_title::TerminalTitleState::new();
        // One failed write costs a stale title, not the session.
        let _ = window_title.refresh(&app.status);
        Self {
            app,
            host,
            project,
            approvals,
            rotations,
            accounts,
            credential_pool,
            agents,
            catalog,
            inventory,
            theme,
            mcp,
            skills,
            session,
            events,
            keys,
            spinner,
            frame,
            local_tx,
            local_rx,
            local_shell_approvals,
            shell_shortcuts,
            child_tx,
            child_rx,
            mcp_changes,
            composed_remote_tools,
            remote_tools_pending,
            trusted_skill_pending,
            last_change_turn,
            interactions,
            dirty,
            window_title,
        }
    }

    async fn on_terminal_event<B>(
        &mut self,
        terminal: &mut ratatui::Terminal<B>,
        key: std::io::Result<TermEvent>,
    ) -> Result<Option<InteractiveExit>>
    where
        B: ratatui::backend::Backend,
        B::Error: Send + Sync + 'static,
    {
        match key.context("reading a terminal event")? {
            // `Ctrl+V` is the explicit "attach from clipboard" chord:
            // terminals deliver ordinary pastes as bracketed text, but
            // an image on the clipboard can only be fetched by asking
            // the platform directly.
            TermEvent::Key(key)
                if key.kind != KeyEventKind::Release
                    && key.code == KeyCode::Char('v')
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                attach_from_clipboard(&mut self.app);
                self.dirty = true;
            }
            TermEvent::Key(key) => {
                if let Some(action) = self.app.on_key(key)
                    && let Some(exit) = self.on_action(action).await
                {
                    return Ok(Some(exit));
                }
                self.dirty = true;
            }
            TermEvent::Paste(text) => {
                self.app.on_paste(&text);
                self.dirty = true;
            }
            TermEvent::Mouse(mouse) => match self.app.on_mouse(mouse) {
                MouseOutcome::Ignored => {}
                MouseOutcome::Redraw => self.dirty = true,
                MouseOutcome::CopySelection => {
                    // Drawn here rather than deferred to the frame
                    // tick: the selected text exists only in the frame
                    // buffer, and a runtime event arriving in between
                    // would clear the selection before it could be
                    // read — a release that silently copied nothing.
                    let mut selected = None;
                    terminal.draw(|frame| {
                        smith_tui::render::layout(frame.area(), &self.app, self.theme)
                            .apply(&mut self.app);
                        smith_tui::render::draw(frame, &self.app, self.theme);
                        selected = smith_tui::selected_text(frame, &self.app);
                    })?;
                    self.dirty = false;
                    // A drag across blank space yields nothing, and
                    // clobbering the clipboard with an empty string
                    // would lose whatever the user had there.
                    if let Some(text) = selected {
                        copy_selection_to_clipboard(&mut self.app, &text);
                    }
                }
            },
            TermEvent::Resize(_, _) => self.dirty = true,
            _ => {}
        }
        Ok(None)
    }

    async fn on_action(&mut self, action: Action) -> Option<InteractiveExit> {
        match action {
            Action::Submit { submission, target } => self.on_submit(submission, target).await,
            Action::RunShell { command } => self.on_run_shell(command).await,
            Action::Interrupt => self.on_interrupt(),
            Action::BackgroundShell => self.on_background_shell(),
            Action::Quit => return Some(self.on_quit()),
            // An account switch is live pool state, so it is
            // applied here rather than by tearing the session
            // down and rebuilding it around a new selection.
            Action::Reconfigure(command) => return self.on_reconfigure(command).await,
            Action::Command(command) => self.on_command(command).await,
            Action::TrustMcpServer { server } => self.on_trust_mcp_server(server),
            Action::TrustSkill { skill: name } => self.on_trust_skill(name),
            Action::ApplyUndo => self.on_apply_undo(),
            Action::CancelUndo => self.on_cancel_undo(),
            Action::ApplyRedo => self.on_apply_redo(),
            Action::CancelRedo => self.on_cancel_redo(),
            Action::ApplyRevert { scope, fingerprint } => self.on_apply_revert(scope, fingerprint),
            Action::CancelRevert { scope, fingerprint } => {
                self.on_cancel_revert(scope, fingerprint)
            }
            Action::StartReview { scope } => self.on_start_review(scope),
            Action::StartAgent { preset, task } => self.on_start_agent(preset, task),
            Action::FollowUpAgent { child_id, task } => self.on_follow_up_agent(child_id, task),
            Action::ResumeAgent { child_id } => self.on_resume_agent(child_id),
        }
        None
    }

    async fn on_submit(
        &mut self,
        submission: smith_tui::app::PreparedSubmission,
        target: SubmissionTarget,
    ) {
        self.host.set_goal_continuation_enabled(false);
        dispatch_prepared_with_materialization(
            &mut self.app,
            self.session,
            self.project,
            submission,
            target,
        )
        .await;
    }

    async fn on_run_shell(&mut self, command: String) {
        let echo = self
            .app
            .transcript
            .latest_shell_echo()
            .expect("submitted shell echo");
        self.shell_shortcuts.dispatched(self.host, echo);
        let identity = start_local_shell(
            echo,
            self.session.clone(),
            command,
            self.host
                .runtime()
                .policy()
                .turn_time_limit_ms
                .unwrap_or(600_000),
            self.local_shell_approvals.clone(),
            self.local_tx.clone(),
        )
        .await;
        match identity {
            Some(LocalShellIdentity::Turn(turn)) => self.app.track_shell_shortcut(turn, echo),
            Some(LocalShellIdentity::Call(call)) => {
                self.app.transcript.bind_shell_shortcut(echo, call.as_str())
            }
            None => {}
        }
    }

    fn on_interrupt(&mut self) {
        if let Err(error) = self
            .session
            .interrupt_current_turn(CancelReason::UserRequested)
        {
            self.app
                .transcript
                .push_error(format!("turn interruption failed: {error}"));
        }
    }

    fn on_background_shell(&mut self) {
        // Kept distinct from `Action::Interrupt`: this
        // never kills the group, it only asks the
        // registry to adopt whatever foreground call
        // is currently running, if any.
        if self
            .host
            .background_tasks()
            .trigger_manual_backgrounding(self.session.id())
        {
            self.app
                .transcript
                .push_notice(NoticeKind::Background, "command moved to the background");
        } else {
            self.app.push_notice(
                NoticeKind::BackgroundUnavailable,
                "no foreground shell command is running",
            );
        }
    }

    fn on_quit(&self) -> InteractiveExit {
        InteractiveExit::Quit(
            Box::new(self.app.session_usage()),
            self.app.status.price().cloned(),
            self.app.status.cache_summary().map(Box::new),
        )
    }

    async fn on_reconfigure(&mut self, command: SessionControl) -> Option<InteractiveExit> {
        match command {
            SessionControl::Account(position) => {
                match switch_account(self.credential_pool.as_ref(), &mut self.accounts, position)
                    .await
                {
                    Some(notice) => {
                        self.app.transcript.push_notice(NoticeKind::Account, notice);
                        self.app
                            .set_accounts(account_entries(self.credential_pool.as_ref()));
                        self.app.status.account = account_status(self.credential_pool.as_ref());
                    }
                    None => self
                        .app
                        .push_notice(NoticeKind::AccountUnchanged, "already using that account"),
                }
            }
            command => {
                if let Some(exit) = reconfigure_exit(&mut self.app, command) {
                    return Some(exit);
                }
            }
        }
        None
    }

    async fn on_command(&mut self, command: smith_client::commands::HostCommand) {
        handle_local_command(
            &mut self.app,
            self.host,
            self.project,
            self.mcp.as_deref(),
            &self.skills,
            command,
        )
        .await;
    }

    fn on_trust_mcp_server(&mut self, server: String) {
        self.app
            .show_local_report(smith_client::local_result::LocalResult::Mcp(Box::new(
                local_command::mcp::trust(self.mcp.as_deref(), &server),
            )));
    }

    fn on_trust_skill(&mut self, name: String) {
        let report = local_command::skills::trust(&self.skills, &name);
        if matches!(
            &report,
            smith_client::skills_report::SkillsReport::Trusted { .. }
        ) {
            self.trusted_skill_pending = true;
        }
        self.app
            .show_local_report(smith_client::local_result::LocalResult::Skills(Box::new(
                report,
            )));
    }

    fn on_apply_undo(&mut self) {
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(
                local_command::recovery::undo(self.host),
            )));
    }

    fn on_cancel_undo(&mut self) {
        self.host.changes().record_undo_cancelled();
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(RecoveryReport::Cancelled(
                RecoveryAction::Undo,
            ))));
    }

    fn on_apply_redo(&mut self) {
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(
                local_command::recovery::redo(self.host),
            )));
    }

    fn on_cancel_redo(&mut self) {
        self.host.changes().record_redo_cancelled();
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(RecoveryReport::Cancelled(
                RecoveryAction::Redo,
            ))));
    }

    fn on_apply_revert(&mut self, scope: String, fingerprint: String) {
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(
                local_command::recovery::revert(self.host, self.project, scope, &fingerprint),
            )));
    }

    fn on_cancel_revert(&mut self, scope: String, fingerprint: String) {
        self.host
            .changes()
            .record_revert_event(&scope, &fingerprint, "cancelled");
        self.app
            .transcript
            .push_local(LocalResult::Recovery(Box::new(RecoveryReport::Cancelled(
                RecoveryAction::Revert,
            ))));
    }

    fn on_start_review(&mut self, scope: String) {
        start_review(self.host, self.project, scope, self.local_tx.clone());
    }

    fn on_start_agent(&mut self, preset: String, task: String) {
        start_agent(self.host, self.agents, preset, task, self.local_tx.clone());
    }

    fn on_follow_up_agent(&mut self, child_id: String, task: String) {
        follow_up_agent(self.host, child_id, task, self.local_tx.clone());
    }

    fn on_resume_agent(&mut self, child_id: String) {
        resume_agent(self.host, child_id, self.local_tx.clone());
    }

    fn on_approval(&mut self, prompt: Option<ApprovalPrompt>) {
        match prompt {
            Some(prompt) => {
                if let Some(prompt) = self.local_shell_approvals.resolve(prompt) {
                    self.app.present_approval(prompt);
                    self.dirty = true;
                }
            }
            None => self.approvals = None,
        }
    }

    fn on_rotation(&mut self, offer: Option<RotationPrompt>) {
        match offer {
            Some(prompt) => {
                self.app.present_rotation(prompt);
                self.dirty = true;
            }
            None => self.rotations = None,
        }
    }

    fn on_interaction_notice(&mut self, notice: Option<smith_host::InteractionNotice>) {
        match notice {
            Some(notice) => {
                self.interactions.apply_notice(&mut self.app, notice);
                self.dirty = true;
            }
            None => self.interactions.close_receiver(),
        }
    }

    async fn on_runtime_event(
        &mut self,
        envelope: Option<EventEnvelope>,
    ) -> Option<InteractiveExit> {
        match envelope {
            Some(envelope) => {
                // The live queue is normally this one envelope. When
                // applying it reveals a broadcast lag gap, the missing
                // range is replayed out of the canonical journal ahead
                // of it, so control events (turn terminals, queued-
                // input releases) still fold in order instead of
                // wedging the UI on a state change it never saw.
                let mut pending = VecDeque::from([(envelope, false)]);
                while let Some((envelope, recovered)) = pending.pop_front() {
                    let tool_call = tool_call_for_display(&envelope.payload);
                    let completed_tool =
                        matches!(envelope.payload, RuntimeEvent::ToolCallCompleted { .. });
                    let turn_completed =
                        matches!(envelope.payload, RuntimeEvent::TurnCompleted { .. });
                    if recovered {
                        self.app.apply_recovered(&envelope);
                    } else {
                        self.app.apply(&envelope);
                        if let Some(gap) = self.app.take_stream_gap() {
                            // The envelope was parked, not applied.
                            // Queue the journal's copy of the missing
                            // range first, then retry the parked
                            // envelope on the honest replay path.
                            self.recover_stream_gap(gap, &mut pending).await;
                            continue;
                        }
                    }
                    if turn_completed && self.host.runtime().advisor_route().is_some() {
                        self.app
                            .status
                            .reconcile_advisor_records(self.host.snapshot().usage.records());
                    }
                    if turn_completed
                        && let Some(set) = self.host.changes().latest()
                        && self.last_change_turn != Some(set.turn)
                        && !set.undone
                        && let Some(notice) = change_notice(&set)
                    {
                        self.last_change_turn = Some(set.turn);
                        self.app.transcript.push_notice(NoticeKind::Changes, notice);
                    }
                    if let Some(submission) = self.app.take_ready_submission() {
                        dispatch_prepared_with_materialization(
                            &mut self.app,
                            self.session,
                            self.project,
                            submission,
                            SubmissionTarget::WholeTurn,
                        )
                        .await;
                    }
                    if let Some(call) = tool_call {
                        if let Some(display) = self.host.tool_call_display(&call) {
                            // Only at request time: the same call id
                            // is resolved again at completion, and
                            // this queue must see a spawn exactly
                            // once or `ChildSpawned` would enrich the
                            // wrong row. This is the root's own
                            // event stream — the child-events branch
                            // below never reaches this call, which is
                            // how the pending-spawn queue stays
                            // root-only.
                            if !completed_tool {
                                self.app.note_pending_spawn(call.as_str(), &display);
                            }
                            self.app.set_tool_display(call.as_str(), display);
                        }
                        if completed_tool && let Some(text) = self.host.tool_result_text(&call) {
                            self.app.set_tool_result_preview(call.as_str(), text);
                        }
                    }
                    // A child is a full runtime session. The parent
                    // stream says one started; its own stream says
                    // what it is doing, and that is what the
                    // inspector draws.
                    //
                    // A resume is a second start: the durable record
                    // is bound to a new execution with a new stream,
                    // and the task watching the old one ended with it.
                    match &envelope.payload {
                        RuntimeEvent::ChildSpawned { child, .. } => {
                            let binding =
                                self.app.child_profile(child.as_str()).and_then(|profile| {
                                    resolve_child_usage_binding(
                                        profile,
                                        self.inventory,
                                        self.catalog,
                                    )
                                });
                            self.app.set_child_usage_binding(child.as_str(), binding);
                            subscribe_to_child(self.host, child, self.child_tx.clone());
                        }
                        RuntimeEvent::ChildProgress {
                            child,
                            phase: ChildPhase::ResumeStarted { .. },
                        } => subscribe_to_child(self.host, child, self.child_tx.clone()),
                        _ => {}
                    }
                }
                self.dirty = true;
            }
            None => {
                return Some(InteractiveExit::Quit(
                    Box::new(self.app.session_usage()),
                    self.app.status.price().cloned(),
                    self.app.status.cache_summary().map(Box::new),
                ));
            }
        }
        None
    }

    async fn recover_stream_gap(
        &mut self,
        gap: smith_tui::app::StreamGap,
        pending: &mut VecDeque<(EventEnvelope, bool)>,
    ) {
        match self
            .host
            .client_events_between(gap.first_missing, gap.last_missing)
            .await
        {
            Ok(events) => {
                if !events.is_empty() {
                    // Accumulated, not shown yet: a
                    // broadcast overrun produces a
                    // run of these gaps back to
                    // back, and `App` collapses the
                    // whole run into one line once
                    // it sees a contiguous event
                    // again.
                    self.app.note_recovered_events(events.len());
                }
                pending.push_front((gap.deferred, true));
                for event in events.into_iter().rev() {
                    pending.push_front((event, true));
                }
            }
            Err(error) => {
                self.app.transcript.push_error(format!(
                    "replaying skipped events {}–{} from the \
                                                 session journal failed: {error}",
                    gap.first_missing, gap.last_missing
                ));
                pending.push_front((gap.deferred, true));
            }
        }
    }

    fn on_child_event(&mut self, child_event: Option<(ChildId, EventEnvelope)>) {
        if let Some((child, envelope)) = child_event {
            self.app.apply_child(child.as_str(), &envelope);
            // The child's events withhold argument values and result
            // text exactly as the root's do. Both are resolved the
            // same way: by call id, against that agent's canonical
            // history, redacted by the host.
            if let Some(call) = tool_call_for_display(&envelope.payload) {
                if let Some(display) = self.host.child_tool_call_display(&child, &call) {
                    self.app
                        .set_child_tool_display(child.as_str(), call.as_str(), display);
                }
                if matches!(envelope.payload, RuntimeEvent::ToolCallCompleted { .. })
                    && let Some(text) = self.host.child_tool_result_text(&child, &call)
                {
                    self.app
                        .set_child_tool_result_preview(child.as_str(), call.as_str(), text);
                }
            }
            self.dirty = true;
        }
    }

    fn on_local_result(&mut self, outcome: Option<LocalOutcome>) {
        if let Some(outcome) = outcome {
            match outcome {
                LocalOutcome::Agent(report) => {
                    self.app
                        .transcript
                        .push_local(smith_client::local_result::LocalResult::Agent(report));
                }
                LocalOutcome::Review(report) => {
                    self.app
                        .transcript
                        .push_local(smith_client::local_result::LocalResult::Review(report));
                }
                LocalOutcome::Notice { kind, text } => {
                    self.app.transcript.push_notice(kind, text);
                }
                LocalOutcome::Error(text) => self.app.transcript.push_error(text),
                LocalOutcome::Shell {
                    echo,
                    call,
                    content,
                    is_error,
                } => {
                    self.shell_shortcuts.finish(
                        self.host,
                        &mut self.app,
                        echo,
                        call.as_ref().map(|call| call.as_str()),
                        &content,
                        is_error,
                    );
                }
            }
            self.dirty = true;
        }
    }

    fn on_mcp_change(&mut self) {
        if let Some(context) = &self.mcp {
            let supervisor = context.supervisor();
            let reports = supervisor.reports();
            self.app.status.mcp = smith_tui::McpStatus {
                connecting: reports
                    .iter()
                    .filter(|report| !report.state.is_settled())
                    .count(),
                failed: reports
                    .iter()
                    .filter(|report| {
                        matches!(report.state, smith_runtime::mcp::McpState::Failed { .. })
                    })
                    .count(),
            };
            self.remote_tools_pending = supervisor.tools().len() != self.composed_remote_tools;
            self.dirty = true;
        }
    }

    fn on_spinner(&mut self) {
        let exit_hint_expired = self.app.expire_ctrl_c_exit_hint();
        // Rows retire while the session is idle — that is the whole
        // point of them retiring — so this cannot ride on `tick`,
        // which only advances while there is work to animate.
        let rows_retired = self.app.expire_child_rows();
        let busy = self.app.is_busy();
        if busy {
            self.app.tick();
        }
        if exit_hint_expired
            || rows_retired
            || (busy && (self.theme.uses_motion() || self.app.tick.is_multiple_of(10)))
        {
            self.dirty = true;
        }
    }

    async fn on_frame<B>(
        &mut self,
        terminal: &mut ratatui::Terminal<B>,
    ) -> Result<Option<InteractiveExit>>
    where
        B: ratatui::backend::Backend,
        B::Error: Send + Sync + 'static,
    {
        // A newly connected server's tools and a newly trusted skill
        // both join at the next idle boundary, never mid-turn:
        // swapping the ability set underneath a running turn is what
        // the epoch rules exist to prevent.
        if (self.remote_tools_pending || self.trusted_skill_pending)
            && !self.app.is_busy()
            && !self.app.has_pending_input()
            && !self.app.has_pending_prompt()
            && self.app.overlay.is_none()
        {
            return Ok(Some(InteractiveExit::CapabilitiesChanged));
        }
        // Re-read on the way to the screen rather than at each site
        // that could change it: the pool also moves on its own — a
        // rotation the runtime performed, a snapshot that arrived
        // mid-turn — and a footer refreshed only on manual switches
        // would keep naming an account the session had already left.
        if self.credential_pool.is_some() {
            self.app.status.account = account_status(self.credential_pool.as_ref());
            self.app
                .set_accounts(account_entries(self.credential_pool.as_ref()));
            // Rotation happens inside the runtime, which cannot reach
            // user-scope state, so the account it moved to is
            // remembered here. `remember` reports whether anything
            // changed, so this writes on a switch and not on a frame.
            remember_active_account(self.credential_pool.as_ref(), &mut self.accounts).await;
        }
        // Same cadence as the account refresh above: the TUI never
        // reaches the registry itself, so this poll-on-redraw is the
        // only path by which a task's start or terminal state
        // reaches operational status and the exit-confirm gate.
        self.app.set_running_tasks(
            self.host
                .background_tasks()
                .running_tasks(self.session.id())
                .into_iter()
                .map(|task| RunningTaskSummary {
                    task_id: task.task_id,
                    command_hint: compact_command_hint(&task.command),
                })
                .collect(),
        );
        // Same reason, for the open child inspector and the
        // delegated-work panel: turns, tokens, and lifecycle live in
        // the coordinator, which the TUI cannot reach. A child
        // selected by arrow key gets the same card as one opened by
        // `/agent <id>`, and it stays current while the child works
        // — and every visible child's panel row gets the
        // coordinator's own turn/token counts on the same cadence,
        // per `usage-accounting`'s "Counts come from the
        // coordinator": Smith computes none of this itself.
        if let Some(coordinator) = self
            .host
            .runtime()
            .delegation()
            .and_then(|delegation| delegation.coordinator())
        {
            let statuses = coordinator.list();
            if let Some(inspected) = self.app.inspected_child.clone() {
                let card = statuses
                    .iter()
                    .find(|status| status.child.as_str() == inspected)
                    .map(AgentSnapshot::from);
                self.app.set_inspected_detail(&inspected, card);
            }
            self.app.set_child_counts(
                statuses
                    .iter()
                    .map(|status| {
                        (
                            status.child.to_string(),
                            smith_tui::app::ChildCounts {
                                turns_used: status.turns_used,
                                max_turns: status.max_turns,
                                tokens_used: status.tokens_used,
                            },
                        )
                    })
                    .collect(),
            );
        }
        // The title rides the redraw cadence: every input that can
        // change it (model switch, project label, activity
        // transition) marks the frame dirty on its way in, and the
        // tracker turns that into at most one OSC write per change.
        let _ = self.window_title.refresh(&self.app.status);
        terminal.draw(|frame| {
            smith_tui::render::layout(frame.area(), &self.app, self.theme).apply(&mut self.app);
            smith_tui::render::draw(frame, &self.app, self.theme);
        })?;
        self.dirty = false;
        Ok(None)
    }

    fn after_event(&mut self) -> Option<InteractiveExit> {
        self.interactions.drain_answers(&mut self.app);
        self.host
            .set_goal_continuation_enabled(!self.app.should_defer_goal_continuation());
        if self.app.should_quit {
            return Some(InteractiveExit::Quit(
                Box::new(self.app.session_usage()),
                self.app.status.price().cloned(),
                self.app.status.cache_summary().map(Box::new),
            ));
        }
        None
    }
}

/// Defence behind command validation: no host-owned work may cross a rebuild.
pub(super) fn reconfigure_exit(app: &mut App, command: SessionControl) -> Option<InteractiveExit> {
    let name = match &command {
        SessionControl::Reconfigure(selection) => match selection {
            SelectionCommand::NewSession => "new",
            SelectionCommand::Resume(_) => "resume",
            SelectionCommand::Profile(_) => "profile",
            SelectionCommand::Model { .. } => "model",
            SelectionCommand::Agent(_) => "agent",
            SelectionCommand::Think(_) => "think",
            SelectionCommand::Effort(_) => "effort",
            SelectionCommand::ContextWindow(_) => "context",
        },
        SessionControl::Connect(_) => "connect",
        SessionControl::Disconnect(_) => "disconnect",
        SessionControl::Account(_) => return None,
    };
    if app.is_busy() || app.has_pending_input() || app.has_pending_prompt() {
        app.push_notice(
            NoticeKind::CommandRefused,
            format!("/{name} requires an idle turn; draft preserved"),
        );
        return None;
    }
    match command {
        SessionControl::Reconfigure(selection) => Some(InteractiveExit::Reconfigure(selection)),
        SessionControl::Connect(provider) => Some(InteractiveExit::Connect(provider)),
        SessionControl::Disconnect(provider) => Some(InteractiveExit::Disconnect(provider)),
        SessionControl::Account(_) => None,
    }
}

/// Bounds a background task's command for compact, single-line display.
///
/// The registry keeps the exact command for its own purposes; the footer and
/// exit-confirm modal only need enough to recognize which task is which.
fn compact_command_hint(command: &str) -> String {
    const MAX_CHARS: usize = 60;
    let collapsed = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_CHARS {
        format!("{}…", collapsed.chars().take(MAX_CHARS).collect::<String>())
    } else {
        collapsed
    }
}

pub(super) async fn next_approval(
    approvals: &mut Option<ApprovalRequests>,
) -> Option<ApprovalPrompt> {
    match approvals {
        Some(approvals) => approvals.recv().await,
        None => std::future::pending().await,
    }
}

/// Waits for the next rotation offer, or never when the provider has no pool.
pub(super) async fn next_rotation(
    rotations: &mut Option<RotationRequests>,
) -> Option<RotationPrompt> {
    match rotations {
        Some(rotations) => rotations.recv().await,
        None => std::future::pending().await,
    }
}

/// Persists the active account when it differs from what is remembered.
///
/// Covers rotations the runtime performed on its own; a manual switch already
/// persists at the point of the switch. A failed write costs stickiness, not
/// the session, so it is swallowed rather than escalated.
pub(super) async fn remember_active_account(
    credential_pool: Option<&SharedPool>,
    accounts: &mut ActiveAccounts,
) {
    let Some(pool) = credential_pool else {
        return;
    };
    let Some((provider, active)) = pool.read(|pool| {
        pool.active()
            .map(|member| (pool.provider().to_owned(), member.reference.clone()))
    }) else {
        return;
    };
    if accounts.remember(&provider, &active) {
        let _ = accounts.save().await;
    }
}

/// Applies an account switch to live pool state and remembers it.
///
/// Returns the transcript notice, or `None` when nothing changed. No runtime
/// is rebuilt: the credential source reads the active member on the next
/// acquisition, so the switch takes effect on the very next attempt.
pub(super) async fn switch_account(
    credential_pool: Option<&SharedPool>,
    accounts: &mut ActiveAccounts,
    position: usize,
) -> Option<String> {
    let pool = credential_pool?;
    let outgoing = pool.read(|pool| pool.active().map(|member| member.reference.clone()))?;
    if !pool.write(|pool| pool.set_active(position)) {
        return None;
    }
    let (provider, incoming) = pool.read(|pool| {
        (
            pool.provider().to_owned(),
            pool.active().map(|member| member.reference.clone()),
        )
    });
    let incoming = incoming?;
    if accounts.remember(&provider, &incoming) {
        // A failed write costs stickiness, not the switch: the session is
        // already using the new account either way, so this is reported rather
        // than escalated.
        if let Err(error) = accounts.save().await {
            let _ = error;
        }
    }
    Some(smith_tui::accounts::switch_notice(
        &outgoing, &incoming, true,
    ))
}

/// Exercises the production select loop without raw mode; an idle recomposition
/// ends the run after queued input so tests can inspect an unsubmitted draft.
#[cfg(test)]
pub(super) async fn run_scripted_tui(
    host: &HostSession,
    project: &std::path::Path,
    resources: &InteractiveResources,
    app: App,
    keys: &mut crate::screen_runner::TerminalEvents,
    recompose: bool,
) -> Result<(InteractiveExit, App)> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 32))?;
    let mut tui = TuiLoop::new(
        app,
        TuiRunInputs {
            keys,
            host,
            project,
            approvals: None,
            interactions: None,
            rotations: None,
            accounts: ActiveAccounts::ephemeral(),
            credential_pool: None,
            agents: &resources.agents,
            catalog: &resources.catalog,
            inventory: &resources.inventory,
            theme: Theme::new().without_color().without_motion(),
            mcp: None,
            skills: resources.skills.clone(),
        },
    );
    tui.remote_tools_pending = recompose;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        run_tui_on(&mut terminal, tui),
    )
    .await
    .context("scripted loop did not reach an exit")?
}

#[cfg(test)]
mod tests {
    use agent_runtime_core::provider::ProviderAttemptPurpose;
    use agent_runtime_core::usage::{
        CounterKind, Provenance, UsageDelta, UsageRecord, UsageSource,
    };

    use super::{restore_usage_records, restore_usage_with_bindings};

    fn price(provider: &str, model: &str) -> smith_client::status::PriceReference {
        smith_client::status::PriceReference {
            provider: provider.to_owned(),
            model: model.to_owned(),
            table: smith_client::status::PriceTable {
                input: Some(2_000_000),
                output: None,
                cache_read: None,
                cache_write: None,
            },
        }
    }

    fn root_record(tokens: u64) -> UsageRecord {
        UsageRecord {
            source: UsageSource::ProviderAttempt,
            provenance: Provenance::default(),
            delta: UsageDelta::new().with(CounterKind::InputUncached, tokens),
        }
    }

    #[test]
    fn restored_usage_uses_the_single_manifest_binding_even_after_a_switch() {
        let mut status = smith_client::status::Status::new("current", "project");
        status.switch_model(Some("google".into()), "current");
        status.set_price(Some(price("google", "current")));
        restore_usage_with_bindings(
            &mut status,
            &[root_record(1_000_000)],
            None,
            [("zai", "glm-5.3"), ("zai", "glm-5.3")],
            |provider, model| Some(price(provider, model)),
        );
        let usage = status.session_usage();
        assert_eq!(status.provider.as_deref(), Some("google"));
        assert_eq!(status.model, "current");
        assert_eq!(usage.total_tokens(), 1_000_000);
        assert_eq!(usage.bindings[0].provider.as_deref(), Some("zai"));
        assert_eq!(usage.bindings[0].model, "glm-5.3");
        let retained = usage.cost_price(status.price()).expect("restored price");
        let cost = smith_client::status::SessionCost::compute(&usage, retained);
        assert_eq!(cost.micro_usd, 2_000_000);
        assert_eq!(cost.label, smith_client::status::CostLabel::Exact);
        assert_eq!(retained.render_sources(&usage), "zai/glm-5.3");
    }

    #[test]
    fn restored_usage_with_several_bindings_stays_unpriced_and_named() {
        let mut status = smith_client::status::Status::new("current", "project");
        status.switch_model(Some("google".into()), "current");
        status.set_price(Some(price("google", "current")));
        restore_usage_with_bindings(
            &mut status,
            &[root_record(9_000_000)],
            None,
            [("zai", "glm-5.3"), ("google", "current")],
            |_, _| panic!("ambiguous records must not request a price"),
        );
        assert!(status.session_usage().cost_price(status.price()).is_none());
        status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1_000_000));
        let usage = status.session_usage();
        let current = usage
            .cost_price(status.price())
            .expect("new usage has a price");
        let cost = smith_client::status::SessionCost::compute(&usage, current);
        assert_eq!(cost.micro_usd, 2_000_000);
        assert_eq!(cost.label, smith_client::status::CostLabel::Estimated);
        assert_eq!(
            current.render_sources(&usage),
            "price unknown for earlier models · google/current $2.000"
        );
    }

    #[test]
    fn restored_usage_without_manifests_has_no_invented_binding() {
        let mut status = smith_client::status::Status::new("current", "project");
        status.switch_model(Some("google".into()), "current");
        status.set_price(Some(price("google", "current")));
        restore_usage_with_bindings(&mut status, &[root_record(100)], None, [], |_, _| {
            panic!("missing manifests must not request a price")
        });
        let usage = status.session_usage();
        assert!(usage.cost_price(status.price()).is_none());
        assert_eq!(usage.bindings[0].model, "earlier models");
        assert!(usage.bindings[0].price.is_none());
    }

    fn logged_usage() -> smith_client::usage_log::SessionUsageRecord {
        let mut observed = smith_client::status::Status::new("glm-5.3", "project");
        observed.switch_model(Some("zai".into()), "glm-5.3");
        observed.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 11_000));
        observed.switch_model(Some("google".into()), "gemini-3.8-flash");
        observed.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 12_000));
        smith_client::usage_log::SessionUsageRecord::new(
            "session",
            observed.provider.clone(),
            &observed.model,
            "build",
            &observed.session_usage(),
        )
    }

    #[test]
    fn usage_restore_reads_the_latest_matching_attributed_record_from_the_project_log() {
        use smith_client::usage_log::{append, default_path};
        use smith_runtime::session::{ProjectId, SessionPaths};
        let root = tempfile::tempdir().expect("root");
        let paths = SessionPaths::new(root.path(), &ProjectId::new("project").expect("project"));
        let mut logged = logged_usage();
        append(&default_path(paths.directory()), &logged).expect("first record");
        logged.turns += 1;
        append(&default_path(paths.directory()), &logged).expect("last record");
        let mut mismatched = logged.clone();
        mismatched.totals.insert("input".into(), 24_000);
        append(&default_path(paths.directory()), &mismatched).expect("mismatched record");
        let mut other = logged.clone();
        other.session = "other-session".into();
        append(&default_path(paths.directory()), &other).expect("other record");
        let session = agent_runtime_core::ids::SessionId::new("session");
        let records = [root_record(23_000)];
        assert_eq!(
            super::last_session_usage(&paths, &session, &records),
            Some(logged)
        );
        assert!(
            super::last_session_usage(
                &paths,
                &agent_runtime_core::ids::SessionId::new("missing"),
                &records
            )
            .is_none()
        );
    }

    fn usage_with_unattributed_followup() -> (
        smith_client::usage_log::SessionUsageRecord,
        smith_client::usage_log::SessionUsageRecord,
        UsageRecord,
    ) {
        let mut observed = smith_client::status::Status::new("glm-5.3", "project");
        for (provider, model, input, output) in [
            ("zai", "glm-5.3", 1238, 14),
            ("google", "gemini-3.8-flash", 1163, 1),
        ] {
            observed.switch_model(Some(provider.into()), model);
            observed.record_usage(
                &UsageDelta::new()
                    .with(CounterKind::InputUncached, input)
                    .with(CounterKind::Output, output),
            );
        }
        let attributed = smith_client::usage_log::SessionUsageRecord::new(
            "session",
            observed.provider.clone(),
            &observed.model,
            "build",
            &observed.session_usage(),
        );
        let mut unattributed = attributed.clone();
        unattributed.bindings = vec![smith_client::usage_log::BindingUsageRecord {
            provider: None,
            model: "earlier models".into(),
            totals: attributed.totals.clone(),
        }];
        let mut restored = root_record(2401);
        restored.delta = restored.delta.with(CounterKind::Output, 15);
        (attributed, unattributed, restored)
    }

    #[test]
    fn usage_restore_skips_a_later_unattributed_record_and_restores_both_models() {
        use smith_client::usage_log::{append, default_path};
        use smith_runtime::session::{ProjectId, SessionPaths};
        let root = tempfile::tempdir().expect("root");
        let paths = SessionPaths::new(root.path(), &ProjectId::new("project").expect("project"));
        let (attributed, unattributed, restored) = usage_with_unattributed_followup();
        append(&default_path(paths.directory()), &attributed).expect("attributed record");
        append(&default_path(paths.directory()), &unattributed).expect("unattributed record");
        let records = [restored];
        let logged = super::last_session_usage(
            &paths,
            &agent_runtime_core::ids::SessionId::new("session"),
            &records,
        );
        assert_eq!(logged.as_ref(), Some(&attributed));
        let mut status = smith_client::status::Status::new("current", "project");
        restore_usage_with_bindings(
            &mut status,
            &records,
            logged.as_ref(),
            [("zai", "glm-5.3"), ("google", "gemini-3.8-flash")],
            |provider, model| Some(price(provider, model)),
        );
        let usage = status.session_usage();
        assert_eq!(usage.bindings.len(), 2);
        for (binding, (provider, model, input, output)) in usage.bindings.iter().zip([
            ("zai", "glm-5.3", 1238, 14),
            ("google", "gemini-3.8-flash", 1163, 1),
        ]) {
            assert_eq!(binding.provider.as_deref(), Some(provider));
            assert_eq!(binding.model, model);
            assert_eq!(binding.totals[&CounterKind::InputUncached], input);
            assert_eq!(binding.totals[&CounterKind::Output], output);
        }
    }

    #[test]
    fn usage_restore_with_only_unattributed_matches_falls_back_to_manifests() {
        use smith_client::usage_log::{append, default_path};
        use smith_runtime::session::{ProjectId, SessionPaths};
        let root = tempfile::tempdir().expect("root");
        let paths = SessionPaths::new(root.path(), &ProjectId::new("project").expect("project"));
        let (_, unattributed, restored) = usage_with_unattributed_followup();
        append(&default_path(paths.directory()), &unattributed).expect("first record");
        append(&default_path(paths.directory()), &unattributed).expect("last record");
        let records = [restored];
        let logged = super::last_session_usage(
            &paths,
            &agent_runtime_core::ids::SessionId::new("session"),
            &records,
        );
        assert!(logged.is_none());
        for manifests in [
            vec![("zai", "glm-5.3")],
            vec![("zai", "glm-5.3"), ("google", "gemini-3.8-flash")],
        ] {
            let mut status = smith_client::status::Status::new("current", "project");
            restore_usage_with_bindings(
                &mut status,
                &records,
                logged.as_ref(),
                manifests.clone(),
                |provider, model| Some(price(provider, model)),
            );
            let usage = status.session_usage();
            assert_eq!(usage.bindings.len(), 1);
            let binding = &usage.bindings[0];
            if manifests.len() == 1 {
                assert_eq!(binding.provider.as_deref(), Some("zai"));
                assert_eq!(binding.model, "glm-5.3");
                assert!(binding.price.is_some());
            } else {
                assert!(binding.provider.is_none());
                assert_eq!(binding.model, "earlier models");
                assert!(binding.price.is_none());
            }
            assert_eq!(binding.totals[&CounterKind::InputUncached], 2401);
            assert_eq!(binding.totals[&CounterKind::Output], 15);
        }
    }

    #[test]
    fn matching_usage_log_restores_both_model_prices_and_keeps_the_active_binding() {
        let logged = logged_usage();
        let mut status = smith_client::status::Status::new("current", "project");
        status.switch_model(Some("google".into()), "current");
        status.restore_turn_count(2);
        restore_usage_with_bindings(
            &mut status,
            &[root_record(23_000)],
            Some(&logged),
            [("zai", "glm-5.3"), ("google", "gemini-3.8-flash")],
            |provider, model| {
                let mut reference = price(provider, model);
                if provider == "google" {
                    reference.table.input = Some(3_000_000);
                }
                Some(reference)
            },
        );
        let usage = status.session_usage();
        assert_eq!(usage.turns, 2);
        assert_eq!(usage.bindings.len(), 2);
        assert_eq!(
            usage.bindings[0].totals[&CounterKind::InputUncached],
            11_000
        );
        assert_eq!(
            usage.bindings[1].totals[&CounterKind::InputUncached],
            12_000
        );
        assert_eq!(status.model, "current");
        let retained = usage.cost_price(status.price()).expect("restored price");
        assert_eq!(
            retained.render_sources(&usage),
            "zai/glm-5.3 $0.022 · google/gemini-3.8-flash $0.036"
        );
        assert_eq!(
            smith_client::status::SessionCost::compute(&usage, retained).micro_usd,
            58_000
        );
        assert_eq!(
            smith_client::status::SessionCost::compute(&usage, retained).label,
            smith_client::status::CostLabel::Exact
        );
    }

    #[test]
    fn mismatched_or_v4_usage_log_falls_back_to_manifests() {
        let original = logged_usage();
        let mut mismatched = original.clone();
        mismatched.totals.insert("input".into(), 1);
        let mut legacy = serde_json::to_value(&original).expect("record");
        legacy["schema_version"] = serde_json::json!(4);
        legacy.as_object_mut().expect("object").remove("bindings");
        let legacy = serde_json::from_value(legacy).expect("v4 record");
        let mut invalid_partition = original;
        invalid_partition.bindings[0]
            .totals
            .insert("input".into(), 1);
        for logged in [mismatched, legacy, invalid_partition] {
            for manifests in [
                vec![("zai", "glm-5.3")],
                vec![("zai", "glm-5.3"), ("google", "gemini-3.8-flash")],
            ] {
                let mut status = smith_client::status::Status::new("current", "project");
                restore_usage_with_bindings(
                    &mut status,
                    &[root_record(23_000)],
                    Some(&logged),
                    manifests.clone(),
                    |provider, model| Some(price(provider, model)),
                );
                let usage = status.session_usage();
                assert_eq!(usage.bindings.len(), 1);
                assert_eq!(
                    usage.bindings[0].model,
                    if manifests.len() == 1 {
                        "glm-5.3"
                    } else {
                        "earlier models"
                    }
                );
            }
        }
    }

    #[test]
    fn child_profile_resolution_keeps_catalog_endpoint_identity_and_unknowns() {
        use smith_config::inventory::{
            ModelInventoryEntry, ProfileInventoryEntry, SelectionInventory,
        };
        use smith_config::model::{AgentPosture, ProfileUse};
        let mut inventory = SelectionInventory {
            profiles: vec![ProfileInventoryEntry {
                name: "child".into(),
                provider: Some("child-alias".into()),
                model: Some("child-model".into()),
                posture: AgentPosture::Build,
                description: None,
                uses: vec![ProfileUse::Child],
                revision: "revision".into(),
                legacy: false,
                selectable: true,
                active: false,
                source: None,
            }],
            models: vec![ModelInventoryEntry {
                provider: "child-alias".into(),
                model: "child-model".into(),
                label: "child model".into(),
                context_tokens: None,
                max_input_tokens: None,
                max_output_tokens: None,
                output_budget: None,
                context_windows: Vec::new(),
                tool_call: None,
                reasoning: None,
                structured_output: None,
                catalog_provider: Some("google".into()),
                catalog_revision: None,
                catalog_retrieved_at_ms: None,
                profiles: vec!["child".into()],
                selectable: true,
                disabled_reason: None,
                active: false,
            }],
            ..SelectionInventory::default()
        };
        let catalog: smith_config::catalog::CatalogSnapshot = serde_json::from_value(serde_json::json!({
            "schema_revision": smith_config::catalog::CATALOG_SCHEMA_REVISION,
            "source_url": "fixture", "source_digest": "fixture", "content_digest": "fixture",
            "source_revision": "revision", "retrieved_at_ms": 0,
            "providers": { "google": {
                "id": "google", "name": "Google", "models": { "child-model": {
                    "id": "child-model", "name": "child model", "tool_call": true,
                    "reasoning": false, "structured_output": false,
                    "cost": { "input": 3_000_000, "output": null, "cache_read": null, "cache_write": null }
                }}
            }}
        })).expect("catalog fixture");
        let binding =
            super::resolve_child_usage_binding("child", &inventory, &catalog).expect("binding");
        assert_eq!(binding.provider.as_deref(), Some("child-alias"));
        assert_eq!(binding.model, "child-model");
        let price = binding.price.expect("child price");
        assert_eq!(price.provider, "child-alias");
        assert_eq!(price.table.input, Some(3_000_000));
        assert!(super::resolve_child_usage_binding("missing", &inventory, &catalog).is_none());
        inventory.models[0].catalog_provider = None;
        let binding = super::resolve_child_usage_binding("child", &inventory, &catalog)
            .expect("known custom binding");
        assert!(
            binding.price.is_none(),
            "no rates borrowed from a matching model name"
        );
        inventory.profiles[0].model = None;
        assert!(super::resolve_child_usage_binding("child", &inventory, &catalog).is_none());
    }

    #[test]
    fn restored_synthetic_usage_stays_out_of_ordinary_turn_totals() {
        let mut status = smith_client::status::Status::new("model", "project");
        let ordinary = UsageRecord {
            source: UsageSource::ProviderAttempt,
            provenance: Provenance {
                attempt_purpose: Some(ProviderAttemptPurpose::Ordinary),
                ..Provenance::default()
            },
            delta: UsageDelta::new()
                .with(CounterKind::InputUncached, 400)
                .with(CounterKind::Output, 20),
        };
        let idle_summary = UsageRecord {
            source: UsageSource::SemanticSummary,
            provenance: Provenance {
                attempt_purpose: Some(ProviderAttemptPurpose::IdleCompaction),
                ..Provenance::default()
            },
            delta: UsageDelta::new()
                .with(CounterKind::InputCached, 900)
                .with(CounterKind::Output, 40),
        };

        restore_usage_records(&mut status, &[ordinary, idle_summary]);

        let usage = status.session_usage();
        assert_eq!(usage.turns, 0);
        assert_eq!(usage.totals[&CounterKind::InputUncached], 400);
        assert_eq!(usage.totals[&CounterKind::Output], 20);
        assert_eq!(usage.synthetic_totals[&CounterKind::InputCached], 900);
        assert_eq!(usage.synthetic_totals[&CounterKind::Output], 40);
        assert_eq!(status.context.value, 400);
        assert_eq!(
            usage.synthetic_by_purpose[&ProviderAttemptPurpose::IdleCompaction]
                [&CounterKind::InputCached],
            900
        );
    }
}

fn change_notice(set: &smith_tools::TurnChangeSet) -> Option<String> {
    // A mutating capability alone does not prove that the turn changed files.
    if set.undone || !set.exact_mutations().any(|edit| edit.before != edit.after) {
        return None;
    }
    let attribution = if set.is_fully_attributable() {
        "undo available"
    } else {
        "contains ambiguous changes; /undo covers Smith's own edits, /diff shows the rest"
    };
    Some(format!("Smith turn {} · {attribution}", set.turn))
}

#[cfg(test)]
mod transcript_notice_tests {
    use super::change_notice;
    use smith_tools::{EditMutation, ToolMutation, TurnChangeSet};

    fn set(mutations: Vec<ToolMutation>) -> TurnChangeSet {
        TurnChangeSet {
            turn: 1,
            mutations,
            undone: false,
        }
    }

    fn edit(before: &[u8], after: &[u8]) -> ToolMutation {
        ToolMutation::Exact(EditMutation {
            call_id: "edit".to_owned(),
            path: "/repo/src/retry.rs".into(),
            before: Some(before.to_vec()),
            after: Some(after.to_vec()),
            before_hash: String::new(),
            after_hash: String::new(),
            recovery_path: None,
        })
    }

    #[test]
    fn empty_and_ambiguous_only_turns_do_not_claim_changed_files() {
        assert_eq!(change_notice(&set(Vec::new())), None);
        assert_eq!(
            change_notice(&set(vec![ToolMutation::Ambiguous {
                call_id: "ls".to_owned(),
                tool: "shell".to_owned()
            }])),
            None
        );
        assert_eq!(change_notice(&set(vec![edit(b"same", b"same")])), None);
    }

    #[test]
    fn a_confirmed_file_change_keeps_its_recovery_notice() {
        let changed = set(vec![edit(b"before", b"after")]);
        assert_eq!(
            change_notice(&changed).as_deref(),
            Some("Smith turn 1 · undo available")
        );
        let mixed = set(vec![
            edit(b"before", b"after"),
            ToolMutation::Ambiguous {
                call_id: "shell".to_owned(),
                tool: "shell".to_owned(),
            },
        ]);
        assert!(
            change_notice(&mixed)
                .unwrap()
                .contains("contains ambiguous changes")
        );
        let mut undone = changed;
        undone.undone = true;
        assert_eq!(change_notice(&undone), None);
    }
}
