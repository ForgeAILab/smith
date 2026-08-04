//! Typed local commands and status/context rendering.

use super::*;

pub(super) fn tool_call_for_display(
    event: &RuntimeEvent,
) -> Option<agent_runtime_core::ids::ToolCallId> {
    match event {
        RuntimeEvent::ToolCallRequested { call, .. }
        | RuntimeEvent::ToolCallCompleted { call, .. } => Some(call.clone()),
        _ => None,
    }
}

pub(super) async fn handle_local_command(
    app: &mut App,
    host: &HostSession,
    project: &std::path::Path,
    command: CommandAction,
) {
    match command {
        CommandAction::Account(_) => {
            // Resolved entirely in the TUI against live pool state, which the
            // host does not need to be consulted about.
            app.show_local_error("account", "account selection is handled by the picker");
        }
        CommandAction::Connect(_) | CommandAction::Disconnect(_) => {
            app.show_local_error(
                "connect",
                "connection commands must run at the safe session-rebuild boundary",
            );
        }
        CommandAction::Context => {
            app.show_local_result(
                "context",
                render_context_view(&app.status, host.runtime().policy()),
            );
        }
        CommandAction::Timeline => {
            let events = match host.timeline_events().await {
                Ok(events) => events,
                Err(error) => {
                    app.show_local_error("timeline", format!("timeline unavailable: {error}"));
                    return;
                }
            };
            let timeline = render_runtime_timeline(&events);
            let mut lines = timeline.lines;
            if lines.is_empty() {
                lines.extend(host.session().snapshot().manifests.iter().map(|manifest| {
                    format!(
                        "root {} · committed · {}/{} · {} activated capability/capabilities",
                        manifest.turn,
                        manifest.manifest.model.provider,
                        manifest.manifest.model.model,
                        manifest.manifest.activation.len(),
                    )
                }));
            }
            if let Some(coordinator) = host
                .runtime()
                .delegation()
                .and_then(|delegation| delegation.coordinator())
            {
                lines.extend(
                    coordinator
                        .list()
                        .into_iter()
                        .filter(|child| !timeline.children.contains(&child.child))
                        .map(|child| {
                            format!(
                                "child {} · session {} · {:?} · {:?} · resumable {} · {}/{} turns",
                                child.child,
                                child.session,
                                child.durability,
                                child.state,
                                child.resumable(),
                                child.turns_used,
                                child.max_turns,
                            )
                        }),
                );
            }
            lines.extend(
                host.changes()
                    .timeline()
                    .into_iter()
                    .enumerate()
                    .map(|(index, entry)| format!("recovery recovery-{} · {entry}", index + 1)),
            );
            if lines.len() > 100 {
                lines.drain(..lines.len().saturating_sub(100));
            }
            if lines.is_empty() {
                app.show_local_empty("timeline", "No turns, children, or recovery actions yet.");
            } else {
                app.show_local_result("timeline", lines.join("\n"));
            }
        }
        CommandAction::Status => {
            let policy = host.runtime().policy();
            let git = GitChanges::discover(project)
                .and_then(|git| git.status_summary())
                .unwrap_or_else(|_| "unavailable (not a Git worktree)".to_owned());
            let child_count = host
                .runtime()
                .delegation()
                .and_then(|delegation| delegation.coordinator())
                .map_or(0, |coordinator| coordinator.list().len());
            let attribution = host.changes().latest().map_or_else(
                || {
                    if host.changes().has_historical_records() {
                        "historical metadata only; not automatically undoable".to_owned()
                    } else {
                        "no attributable turn recorded".to_owned()
                    }
                },
                |set| {
                    if set.is_fully_attributable() && !set.undone {
                        format!("Smith turn {} · undo available", set.turn)
                    } else {
                        format!("Smith turn {} · automatic undo unavailable", set.turn)
                    }
                },
            );
            let context = render_context_status(&app.status, policy);
            let harness = render_harness_status(&app.status);
            let reasoning = render_reasoning_status(policy);
            let goal = host.goal().map_or_else(
                |error| format!("unavailable ({error})"),
                |goal| goal.as_ref().map_or_else(|| "none".to_owned(), render_goal),
            );
            let connections = if app.resources.disconnections.is_empty() {
                "none".to_owned()
            } else {
                app.resources
                    .disconnections
                    .iter()
                    .map(|entry| format!("{} ({})", entry.label, entry.detail))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            app.show_local_result(
                "status",
                format!(
                    "session: {}\nprofile: {} · posture {} · use {} · rev {} · source {}{}\n\
                     provider: {}\nmodel: {}\npermission: {:?}\n\
                     {reasoning}\n\
                     protected mid-turn recovery: {}\n\
                     {harness}\n{context}\nproject: {}\nGit: {}\n\
                     goal: {goal}\nconnections: {connections}\nchildren: {}\nchange attribution: {}",
                    host.session().id(),
                    policy.agent_profile,
                    policy.agent_posture.as_str(),
                    policy
                        .agent_profile_uses
                        .iter()
                        .map(|placement| placement.as_str())
                        .collect::<Vec<_>>()
                        .join("+"),
                    bounded_text(&policy.agent_profile_revision, 12),
                    bounded_text(&policy.agent_profile_source, 80),
                    if policy.agent_profile_legacy {
                        " · legacy adapter; migrate to [profiles]"
                    } else {
                        ""
                    },
                    policy.provider_name,
                    policy.model,
                    policy.approval_mode,
                    policy.mid_turn_durability.as_str(),
                    project.display(),
                    git,
                    child_count,
                    attribution,
                ),
            );
        }
        CommandAction::Goal(action) => {
            let result = match action {
                GoalAction::Show => match host.goal() {
                    Ok(Some(goal)) => {
                        app.show_local_result("goal", render_goal(&goal));
                        return;
                    }
                    Ok(None) if host.runtime().goal_component().is_some() => {
                        app.show_local_empty(
                            "goal",
                            "No persistent goal. Create one with `/goal <objective>`.",
                        );
                        return;
                    }
                    Ok(None) => {
                        app.show_local_error(
                            "goal",
                            "Persistent goals require a persisted root session; they are unavailable in ephemeral and child sessions.",
                        );
                        return;
                    }
                    Err(error) => Err(error),
                },
                GoalAction::Create(objective) => {
                    host.control_goal(GoalCommand::Create {
                        objective,
                        token_budget: None,
                    })
                    .await
                }
                GoalAction::Edit(objective) => match host.goal() {
                    Ok(Some(goal)) => {
                        host.control_goal(GoalCommand::Edit {
                            id: goal.id,
                            generation: goal.generation,
                            objective,
                        })
                        .await
                    }
                    Ok(None) => {
                        app.show_local_error("goal", "No goal to edit; use `/goal <objective>`.");
                        return;
                    }
                    Err(error) => Err(error),
                },
                GoalAction::Budget(token_budget) => match host.goal() {
                    Ok(Some(goal)) => {
                        host.control_goal(GoalCommand::SetBudget {
                            id: goal.id,
                            generation: goal.generation,
                            token_budget,
                        })
                        .await
                    }
                    Ok(None) => {
                        app.show_local_error(
                            "goal",
                            "No goal budget to change; use `/goal <objective>` first.",
                        );
                        return;
                    }
                    Err(error) => Err(error),
                },
                GoalAction::Pause => match host.goal() {
                    Ok(Some(goal)) => {
                        host.control_goal(GoalCommand::Pause {
                            id: goal.id,
                            generation: goal.generation,
                        })
                        .await
                    }
                    Ok(None) => {
                        app.show_local_error("goal", "No active goal to pause.");
                        return;
                    }
                    Err(error) => Err(error),
                },
                GoalAction::Resume => match host.goal() {
                    Ok(Some(goal)) => {
                        host.control_goal(GoalCommand::Resume {
                            id: goal.id,
                            generation: goal.generation,
                        })
                        .await
                    }
                    Ok(None) => {
                        app.show_local_error("goal", "No stopped goal to resume.");
                        return;
                    }
                    Err(error) => Err(error),
                },
                GoalAction::Clear => match host.goal() {
                    Ok(Some(goal)) => {
                        host.control_goal(GoalCommand::Clear {
                            id: goal.id,
                            generation: goal.generation,
                        })
                        .await
                    }
                    Ok(None) => {
                        app.show_local_error("goal", "No goal to clear.");
                        return;
                    }
                    Err(error) => Err(error),
                },
            };
            match result {
                Ok(result) => match result.goal {
                    Some(goal) => app.show_local_result("goal", render_goal(&goal)),
                    None => app.show_local_result("goal", "Goal cleared."),
                },
                Err(error) => app.show_local_error("goal", error.to_string()),
            }
        }
        CommandAction::Agent(selected) => {
            let Some(coordinator) = host
                .runtime()
                .delegation()
                .and_then(|delegation| delegation.coordinator())
            else {
                app.show_local_error(
                    "agents",
                    "Child delegation is unavailable for this session.",
                );
                return;
            };
            let children = coordinator.list();
            let selected = match selected.as_deref() {
                Some("parent") => {
                    app.inspected_child = None;
                    app.show_local_result(
                        "agent",
                        "Returned to the root timeline; the root composer remained focused.",
                    );
                    return;
                }
                Some("next" | "previous") if children.is_empty() => None,
                Some(direction @ ("next" | "previous")) => {
                    let current = app
                        .inspected_child
                        .as_deref()
                        .and_then(|current| {
                            children
                                .iter()
                                .position(|status| status.child.as_str() == current)
                        })
                        .unwrap_or(0);
                    let index = if direction == "next" {
                        (current + 1) % children.len()
                    } else {
                        current.checked_sub(1).unwrap_or(children.len() - 1)
                    };
                    Some(children[index].child.as_str().to_owned())
                }
                Some(selected) => Some(selected.to_owned()),
                None => None,
            };
            if let Some(selected) = selected {
                let Some(status) = children
                    .iter()
                    .find(|status| status.child.as_str() == selected)
                else {
                    app.show_local_error("agents", format!("No child named `{selected}`."));
                    return;
                };
                app.inspected_child = Some(selected);
                app.show_local_result(
                    "agent",
                    format!(
                        "child: {}\nchild session: {}\ndurability: {:?}\nstate: {:?}\nresumable: {}\nturns: {}/{}\ntokens: {}\nworkspace: {:?}\nincompatibility: {}\nresult: {}\n\ncontinue: @{} <new follow-up task>\nexact recovery: /agent resume {}\nnavigation: /agent previous · /agent next · /agent parent",
                        status.child,
                        status.session,
                        status.durability,
                        status.state,
                        status.resumable(),
                        status.turns_used,
                        status.max_turns,
                        status.tokens_used,
                        status.workspace,
                        status.incompatibility.as_deref().unwrap_or("none"),
                        status.last_result.as_deref().unwrap_or("not available"),
                        status.child,
                        status.child,
                    ),
                );
            } else if children.is_empty() {
                app.show_local_empty("agents", "No child agents in this session.");
            } else {
                app.show_local_result(
                    "agents",
                    children
                        .iter()
                        .map(|status| {
                            format!(
                                "{} · {:?} · {:?} · resumable {} · {}/{} turns · {} tokens",
                                status.child,
                                status.durability,
                                status.state,
                                status.resumable(),
                                status.turns_used,
                                status.max_turns,
                                status.tokens_used,
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
        }
        CommandAction::Diff(scope) => {
            if scope.as_deref() == Some("last-turn") {
                match host.changes().undo_preview() {
                    Ok(preview) => app.show_local_result("diff · last Smith turn", preview),
                    Err(error) => app.show_local_error("diff · last Smith turn", error.message),
                }
            } else {
                match GitChanges::discover(project).and_then(|git| git.inspect(scope.as_deref())) {
                    Ok(view) if view.content == "No changes in this scope." => {
                        app.show_local_empty(view.title, view.content);
                    }
                    Ok(view) => app.show_local_result(view.title, view.content),
                    Err(error) => app.show_local_error("diff", error.message),
                }
            }
        }
        CommandAction::Review(scope) => {
            let scope = scope.unwrap_or_else(|| "all".to_owned());
            match GitChanges::discover(project)
                .and_then(|git| git.inspect(Some(scope.as_str())))
            {
                Ok(view) if view.content == "No changes in this scope." => {
                    app.transcript.push_notice("review", view.content);
                }
                Ok(view) => app.confirm_review(
                    scope,
                    format!(
                        "scope: {}\nprovider-backed: yes\nworkspace authority: read-only\n\
                         The reviewer can read, list, and search but cannot edit or run shell commands.\n\n{}",
                        view.title, view.content
                    ),
                ),
                Err(error) => app.transcript.push_error(error.message),
            }
        }
        CommandAction::Undo => match host.changes().undo_preview() {
            Ok(preview) => app.confirm_undo(preview),
            Err(error) => app.transcript.push_error(error.message),
        },
        CommandAction::Redo => match host.changes().redo_preview() {
            Ok(preview) => app.confirm_redo(preview),
            Err(error) => app.show_local_error("redo", error.message),
        },
        CommandAction::Revert(Some(scope)) => {
            match GitChanges::discover(project).and_then(|git| git.preview_revert(&scope)) {
                Ok(mut preview) => {
                    let path = scope.split('#').next().unwrap_or(scope.as_str());
                    if let Ok(canonical) = project.join(path).canonicalize()
                        && host.changes().latest_owns_path(&canonical)
                    {
                        preview.content =
                            preview
                                .content
                                .replacen("origin: unknown", "origin: Smith", 1);
                    }
                    host.changes().record_revert_event(
                        &preview.scope,
                        &preview.fingerprint,
                        "previewed",
                    );
                    app.confirm_revert(preview.scope, preview.fingerprint, preview.content);
                }
                Err(error) => app.transcript.push_error(error.message),
            }
        }
        CommandAction::Revert(None) => app
            .transcript
            .push_error("usage: /revert FILE or /revert FILE#HUNK; use /diff to choose a scope"),
        CommandAction::Help
        | CommandAction::Details
        | CommandAction::NewSession
        | CommandAction::Resume(_)
        | CommandAction::Profile(_)
        | CommandAction::Provider(_)
        | CommandAction::Model(_)
        | CommandAction::Think(_)
        | CommandAction::Effort(_)
        | CommandAction::AgentResume(_)
        | CommandAction::Quit => {
            unreachable!("the reducer handles this command before host dispatch")
        }
    }
}

pub(super) fn render_goal(goal: &GoalProjection) -> String {
    let status = goal.status.as_str();
    let usage = goal
        .usage
        .charged_tokens
        .map_or_else(|| "unknown".to_owned(), |tokens| tokens.to_string());
    let budget = goal
        .token_budget
        .map_or_else(|| "none".to_owned(), |tokens| tokens.to_string());
    let provenance = goal.usage.provenance.as_str();
    let reason = goal.stopped_reason.as_ref().map_or_else(
        || "none".to_owned(),
        |reason| {
            reason.detail.as_ref().map_or_else(
                || reason.code.clone(),
                |detail| format!("{} · {detail}", reason.code),
            )
        },
    );
    format!(
        "{}\nstatus: {status}\ntokens: {usage} · {provenance}\nbudget: {budget}\nactive elapsed: {}\nreason: {reason}\nid: {} · generation {}",
        goal.objective,
        render_elapsed(Duration::from_millis(goal.usage.active_elapsed_ms)),
        goal.id,
        goal.generation,
    )
}

#[derive(Debug, Default)]
pub(super) struct RuntimeTimeline {
    pub(super) lines: Vec<String>,
    pub(super) children: BTreeSet<ChildId>,
}

#[derive(Debug, Default)]
pub(super) struct TurnTimelineState {
    plan: Option<BTreeMap<String, u32>>,
    passed_gates: u32,
    failed_gates: u32,
}

pub(super) fn render_runtime_timeline(events: &[EventEnvelope]) -> RuntimeTimeline {
    let mut timeline = RuntimeTimeline::default();
    let mut turns = BTreeMap::<String, TurnTimelineState>::new();
    let mut tools = BTreeMap::<String, String>::new();

    for event in events {
        match &event.payload {
            RuntimeEvent::PlanUpdated { counts, .. } => {
                if let Some(turn) = &event.turn {
                    turns.entry(turn.as_str().to_owned()).or_default().plan = Some(counts.clone());
                }
            }
            RuntimeEvent::ToolCallRequested { call, name, .. } => {
                tools.insert(call.as_str().to_owned(), name.clone());
            }
            RuntimeEvent::ToolCallCompleted { call, is_error, .. }
                if tools.get(call.as_str()).is_some_and(|name| name == "shell") =>
            {
                if let Some(turn) = &event.turn {
                    let state = turns.entry(turn.as_str().to_owned()).or_default();
                    if *is_error {
                        state.failed_gates = state.failed_gates.saturating_add(1);
                    } else {
                        state.passed_gates = state.passed_gates.saturating_add(1);
                    }
                }
            }
            RuntimeEvent::TurnCompleted { finish, .. } => {
                if let Some(turn) = &event.turn {
                    let state = turns.remove(turn.as_str()).unwrap_or_default();
                    timeline.lines.push(format!(
                        "root {} · {} · {} · gates {} passed/{} failed",
                        turn,
                        turn_finish_label(finish),
                        render_terminal_plan(state.plan.as_ref()),
                        state.passed_gates,
                        state.failed_gates,
                    ));
                }
            }
            RuntimeEvent::ChildSpawned {
                child,
                workspace,
                max_turns,
                ..
            } => {
                timeline.children.insert(child.clone());
                timeline.lines.push(format!(
                    "child {child} · started · {workspace:?} · {max_turns} turn limit"
                ));
            }
            RuntimeEvent::ChildNeedsInput { child, .. } => {
                timeline.children.insert(child.clone());
                timeline.lines.push(format!("child {child} · needs input"));
            }
            RuntimeEvent::ChildCompleted { child, .. } => {
                timeline.children.insert(child.clone());
                timeline
                    .lines
                    .push(format!("child {child} · task completed"));
            }
            RuntimeEvent::ChildStopped { child, reason } => {
                timeline.children.insert(child.clone());
                timeline
                    .lines
                    .push(format!("child {child} · stopped ({reason:?})"));
            }
            RuntimeEvent::ChildFailed { child, .. } => {
                timeline.children.insert(child.clone());
                timeline.lines.push(format!("child {child} · failed"));
            }
            _ => {}
        }
    }

    timeline
}

pub(super) fn render_terminal_plan(counts: Option<&BTreeMap<String, u32>>) -> String {
    let Some(counts) = counts else {
        return "plan none".to_owned();
    };
    let count = |status: &str| counts.get(status).copied().unwrap_or_default();
    format!(
        "plan {} active/{} pending/{} done/{} cancelled",
        count("in_progress"),
        count("pending"),
        count("completed"),
        count("cancelled")
    )
}

pub(super) fn turn_finish_label(finish: &TurnFinish) -> String {
    match finish {
        TurnFinish::Completed => "completed".to_owned(),
        TurnFinish::Cancelled { reason } => format!("cancelled ({reason:?})"),
        TurnFinish::LimitReached { limit } => format!("limit reached ({limit:?})"),
        TurnFinish::NeedsInput { request } => format!("needs input ({request})"),
        TurnFinish::Failed => "failed".to_owned(),
    }
}

pub(super) fn render_harness_status(status: &Status) -> String {
    let capabilities = &status.capabilities;
    let mut lines = Vec::new();
    match &capabilities.registry {
        Some((fingerprint, entries)) => {
            lines.push(format!(
                "registry snapshot: {fingerprint} · {entries} entries"
            ));
        }
        None => lines.push("registry snapshot: waiting for live lifecycle".to_owned()),
    }
    if let Some((fingerprint, visible)) = &capabilities.view {
        lines.push(format!(
            "scoped capability view: {fingerprint} · {visible} visible"
        ));
    }
    if let Some((revision, candidates)) = &capabilities.retrieval {
        let candidates = if candidates.is_empty() {
            "(none)".to_owned()
        } else {
            candidates.join(", ")
        };
        lines.push(format!(
            "latest capability retrieval: {revision} · {candidates}"
        ));
    }
    if let Some((epoch, active)) = &capabilities.activation {
        let active = if active.is_empty() {
            "(none)".to_owned()
        } else {
            active.join(", ")
        };
        lines.push(format!("activation epoch: {epoch} · {active}"));
    }
    if let Some(plan) = &status.context_plan {
        lines.push(format!(
            "context provenance: {} · cache {}",
            plan.fingerprint, plan.cache_fingerprint
        ));
    }
    if capabilities.compactions > 0 {
        lines.push(format!(
            "context compaction: {} run(s) · {} tokens reclaimed",
            capabilities.compactions, capabilities.reclaimed_tokens
        ));
    }
    lines.join("\n")
}

pub(super) fn render_context_status(status: &Status, policy: &RuntimePolicy) -> String {
    let limits = policy.model_profile.limits;
    let declared_reserve = policy
        .context_policy
        .output_reserve
        .saturating_add(policy.context_policy.reasoning_reserve);
    let input_budget = limits.input_budget(declared_reserve);
    let exact = |tokens: u32| TokenCount::reported(u64::from(tokens)).render();
    let with_confidence = |tokens: u32, confidence: EstimationConfidence| match confidence {
        EstimationConfidence::Exact => TokenCount::reported(u64::from(tokens)).render(),
        EstimationConfidence::Estimated => TokenCount::estimated(u64::from(tokens)).render(),
    };

    let mut lines = Vec::new();
    if let Some(plan) = &status.context_plan {
        let percent_prefix = if plan.confidence == EstimationConfidence::Estimated {
            "~"
        } else {
            ""
        };
        lines.push(format!(
            "context window: {percent_prefix}{}% input left ({} used / {} budget)",
            plan.percent_left(),
            plan.render_input(),
            exact(plan.input_budget_tokens),
        ));
        lines.push(format!(
            "model window: {} total · {} reserved",
            exact(limits.context_tokens),
            exact(plan.reserved_tokens),
        ));
        lines.push(format!(
            "context plan: {} · {} segments",
            plan.confidence_label(),
            plan.segment_count,
        ));
        for (kind, tokens) in &plan.totals {
            lines.push(format!(
                "  {}: {}",
                kind.replace('_', " "),
                with_confidence(*tokens, plan.confidence),
            ));
        }
        let compaction_target = exact(policy.compaction_policy.low_watermark);
        if let Some(summary_tokens) = plan.totals.get("summary").filter(|tokens| **tokens > 0) {
            lines.push(format!(
                "compaction: applied · {} summary · {} recovery target",
                with_confidence(*summary_tokens, plan.confidence),
                compaction_target,
            ));
        } else {
            lines.push(format!(
                "compaction: enabled on overflow · {compaction_target} recovery target"
            ));
        }
    } else {
        lines.push(format!(
            "context window: not planned yet (? used / {} input budget)",
            exact(input_budget),
        ));
        lines.push(format!(
            "model window: {} total · {} reserved",
            exact(limits.context_tokens),
            exact(declared_reserve),
        ));
        lines.push("context plan: waiting for first turn".to_owned());
        lines.push(format!(
            "compaction: enabled on overflow · {} recovery target",
            exact(policy.compaction_policy.low_watermark),
        ));
    }
    lines.push(format!(
        "provider input (session): {}",
        status.context.render()
    ));
    lines.push(format!("cache read (session): {}", status.render_cache()));
    lines.push(render_reasoning_status(policy));
    lines.join("\n")
}

pub(super) fn render_reasoning_status(policy: &RuntimePolicy) -> String {
    let support = match policy.reasoning.support {
        ReasoningSupport::Unsupported => "unsupported",
        ReasoningSupport::Fixed => "fixed",
        ReasoningSupport::Controllable => "controllable",
    };
    let efforts = if policy.reasoning.efforts.is_empty() {
        "none".to_owned()
    } else {
        policy.reasoning.efforts.join(", ")
    };
    format!(
        "reasoning: {} · effort {} · {}\nreasoning controls: {support} · switch {} · efforts {efforts} · {}",
        policy.reasoning.effective_state(),
        policy.reasoning.effective_effort(),
        policy.reasoning.selection_source,
        policy.reasoning.switch.as_str(),
        policy.reasoning.capability_source,
    )
}

#[derive(Debug, Clone)]
pub(super) struct ContextDisplayCategory {
    pub(super) label: String,
    pub(super) glyph: &'static str,
    pub(super) tokens: u32,
    pub(super) rank: u8,
}

pub(super) fn render_context_view(status: &Status, policy: &RuntimePolicy) -> String {
    let limits = policy.model_profile.limits;
    let declared_reserve = policy
        .context_policy
        .output_reserve
        .saturating_add(policy.context_policy.reasoning_reserve);
    let input_budget = limits.input_budget(declared_reserve);
    let exact = |tokens: u32| TokenCount::reported(u64::from(tokens)).render();
    let with_confidence = |tokens: u32, confidence: EstimationConfidence| match confidence {
        EstimationConfidence::Exact => TokenCount::reported(u64::from(tokens)).render(),
        EstimationConfidence::Estimated => TokenCount::estimated(u64::from(tokens)).render(),
    };

    let mut lines = vec!["Context usage".to_owned()];
    if let Some(plan) = &status.context_plan {
        let percent_prefix = if plan.confidence == EstimationConfidence::Estimated {
            "~"
        } else {
            ""
        };
        lines.push(format!(
            "{} · {} / {} input tokens · {percent_prefix}{}% left",
            policy.model,
            plan.render_input(),
            exact(plan.input_budget_tokens),
            plan.percent_left(),
        ));
        lines.push(String::new());

        let mut categories = context_display_categories(&plan.totals);
        let categorized = categories.iter().fold(0u32, |total, category| {
            total.saturating_add(category.tokens)
        });
        if plan.input_tokens > categorized {
            categories.push(ContextDisplayCategory {
                label: "other context".to_owned(),
                glyph: glyph::CONTEXT_OTHER,
                tokens: plan.input_tokens - categorized,
                rank: u8::MAX,
            });
        }
        let free_tokens = plan.remaining_tokens();
        let mut grid = categories
            .iter()
            .map(|category| (category.glyph, category.tokens))
            .collect::<Vec<_>>();
        grid.push((glyph::CONTEXT_FREE, free_tokens));
        grid.push((glyph::CONTEXT_RESERVE, plan.reserved_tokens));
        lines.extend(render_context_grid(&grid));
        lines.push(String::new());
        lines.push(match plan.confidence {
            EstimationConfidence::Exact => "Exact usage by category".to_owned(),
            EstimationConfidence::Estimated => "Estimated usage by category".to_owned(),
        });
        for category in &categories {
            lines.push(format!(
                "{} {}: {} ({})",
                category.glyph,
                category.label,
                with_confidence(category.tokens, plan.confidence),
                render_percent(category.tokens, plan.input_budget_tokens),
            ));
        }
        lines.push(format!(
            "{} free input: {} ({})",
            glyph::CONTEXT_FREE,
            with_confidence(free_tokens, plan.confidence),
            render_percent(free_tokens, plan.input_budget_tokens),
        ));
        lines.push(format!(
            "{} output/reasoning reserve: {} ({})",
            glyph::CONTEXT_RESERVE,
            exact(plan.reserved_tokens),
            render_percent(
                plan.reserved_tokens,
                plan.input_budget_tokens
                    .saturating_add(plan.reserved_tokens),
            ),
        ));
        lines.push(format!(
            "model window: {} total · {} input budget",
            exact(limits.context_tokens),
            exact(plan.input_budget_tokens),
        ));
        lines.push(format!(
            "counting: {} · {} segments",
            plan.confidence_label(),
            plan.segment_count,
        ));
        let compaction_target = exact(policy.compaction_policy.low_watermark);
        if let Some(summary_tokens) = plan.totals.get("summary").filter(|tokens| **tokens > 0) {
            lines.push(format!(
                "compaction: applied · {} summary · {} recovery target",
                with_confidence(*summary_tokens, plan.confidence),
                compaction_target,
            ));
        } else {
            lines.push(format!(
                "compaction: enabled on overflow · {compaction_target} recovery target"
            ));
        }
    } else {
        lines.push(format!(
            "{} · usage unavailable until the first turn",
            policy.model
        ));
        lines.push(String::new());
        lines.extend(render_context_grid(&[
            (glyph::CONTEXT_FREE, input_budget),
            (glyph::CONTEXT_RESERVE, declared_reserve),
        ]));
        lines.push(String::new());
        lines.push("Available capacity".to_owned());
        lines.push(format!(
            "{} system instructions: ? (not counted yet)",
            glyph::CONTEXT_SYSTEM,
        ));
        lines.push(format!(
            "{} tool schemas: ? (not counted yet)",
            glyph::CONTEXT_TOOL,
        ));
        lines.push(format!(
            "{} free input: {}",
            glyph::CONTEXT_FREE,
            exact(input_budget),
        ));
        lines.push(format!(
            "{} output/reasoning reserve: {}",
            glyph::CONTEXT_RESERVE,
            exact(declared_reserve),
        ));
        lines.push(format!(
            "model window: {} total · {} input budget",
            exact(limits.context_tokens),
            exact(input_budget),
        ));
        lines.push("counting: waiting for first context plan".to_owned());
        lines.push(format!(
            "compaction: enabled on overflow · {} recovery target",
            exact(policy.compaction_policy.low_watermark),
        ));
    }
    lines.push(format!(
        "provider input (session): {}",
        status.context.render()
    ));
    lines.push(format!("cache read (session): {}", status.render_cache()));
    lines.push(render_reasoning_status(policy));
    lines.join("\n")
}

pub(super) fn context_display_categories(
    totals: &std::collections::BTreeMap<String, u32>,
) -> Vec<ContextDisplayCategory> {
    const INSTRUCTION_KINDS: [&str; 3] = [
        "system_instruction",
        "developer_instruction",
        "ability_instruction",
    ];

    let instruction_tokens = INSTRUCTION_KINDS.iter().fold(0u32, |sum, kind| {
        sum.saturating_add(totals.get(*kind).copied().unwrap_or_default())
    });
    let mut categories = vec![
        ContextDisplayCategory {
            label: "system instructions".to_owned(),
            glyph: glyph::CONTEXT_SYSTEM,
            tokens: instruction_tokens,
            rank: 0,
        },
        ContextDisplayCategory {
            label: "tool schemas".to_owned(),
            glyph: glyph::CONTEXT_TOOL,
            tokens: totals.get("tool_schema").copied().unwrap_or_default(),
            rank: 1,
        },
    ];
    categories.extend(
        totals
            .iter()
            .filter(|(kind, tokens)| {
                **tokens > 0
                    && !INSTRUCTION_KINDS.contains(&kind.as_str())
                    && kind.as_str() != "tool_schema"
            })
            .map(|(kind, tokens)| context_display_category(kind, *tokens)),
    );
    categories.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| left.label.cmp(&right.label))
    });
    categories
}

pub(super) fn context_display_category(kind: &str, tokens: u32) -> ContextDisplayCategory {
    let (label, glyph, rank) = match kind {
        "system_instruction" => ("system instructions".to_owned(), glyph::CONTEXT_SYSTEM, 0),
        "developer_instruction" => (
            "developer instructions".to_owned(),
            glyph::CONTEXT_SYSTEM,
            1,
        ),
        "ability_instruction" => ("ability instructions".to_owned(), glyph::CONTEXT_SYSTEM, 2),
        "tool_schema" => ("tool schemas".to_owned(), glyph::CONTEXT_TOOL, 3),
        "memory" => ("memory".to_owned(), glyph::CONTEXT_HISTORY, 4),
        "history" => ("history".to_owned(), glyph::CONTEXT_HISTORY, 5),
        "tool_result" => ("tool results".to_owned(), glyph::CONTEXT_TOOL, 6),
        "retrieval" => ("retrieved context".to_owned(), glyph::CONTEXT_HISTORY, 7),
        "continuation" => ("continuation".to_owned(), glyph::CONTEXT_OTHER, 8),
        "summary" => ("summary".to_owned(), glyph::CONTEXT_SUMMARY, 9),
        "user_input" => ("user input".to_owned(), glyph::CONTEXT_INPUT, 10),
        other => (other.replace('_', " "), glyph::CONTEXT_OTHER, u8::MAX - 1),
    };
    ContextDisplayCategory {
        label,
        glyph,
        tokens,
        rank,
    }
}

pub(super) fn render_percent(tokens: u32, total: u32) -> String {
    if total == 0 {
        return "0.0%".to_owned();
    }
    let tenths = u64::from(tokens)
        .saturating_mul(1_000)
        .checked_div(u64::from(total))
        .unwrap_or(0);
    format!("{}.{:01}%", tenths / 10, tenths % 10)
}

pub(super) fn render_context_grid(entries: &[(&'static str, u32)]) -> Vec<String> {
    const CELLS: usize = 50;
    const COLUMNS: usize = 10;

    let entries = entries
        .iter()
        .copied()
        .filter(|(_, tokens)| *tokens > 0)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return vec![format!("{} ", glyph::CONTEXT_FREE).repeat(COLUMNS); CELLS / COLUMNS];
    }

    let weight = entries.iter().fold(0u64, |total, (_, tokens)| {
        total.saturating_add(u64::from(*tokens))
    });
    let remaining = CELLS.saturating_sub(entries.len());
    let mut allocations = vec![1usize; entries.len()];
    let mut remainders = Vec::with_capacity(entries.len());
    let mut distributed = 0usize;
    for (index, (_, tokens)) in entries.iter().enumerate() {
        let numerator = (remaining as u64).saturating_mul(u64::from(*tokens));
        let share = numerator.checked_div(weight).unwrap_or(0) as usize;
        allocations[index] = allocations[index].saturating_add(share);
        distributed = distributed.saturating_add(share);
        remainders.push((index, numerator.checked_rem(weight).unwrap_or(0)));
    }
    remainders.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    for (index, _) in remainders
        .into_iter()
        .take(remaining.saturating_sub(distributed))
    {
        allocations[index] = allocations[index].saturating_add(1);
    }

    let cells = entries
        .iter()
        .zip(allocations)
        .flat_map(|((glyph, _), count)| std::iter::repeat_n(*glyph, count))
        .take(CELLS)
        .collect::<Vec<_>>();
    cells.chunks(COLUMNS).map(|row| row.join(" ")).collect()
}

pub(super) enum LocalOutcome {
    Notice {
        /// The transcript block label — "agents" for child lifecycle,
        /// "review" for reviewer starts.
        source: &'static str,
        text: String,
    },
    Error(String),
    Shell {
        content: String,
        is_error: bool,
    },
}
