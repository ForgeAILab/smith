//! Captures the host values for the local `/timeline` report.

use std::collections::{BTreeMap, BTreeSet};

use agent_runtime_core::ids::ChildId;
use smith_client::timeline_report::{
    TimelineChildEvent, TimelineEntry, TimelinePlan, TimelineReport,
};
use smith_runtime::client::{SmithEvent, SmithEventKind, TurnFinish};
use smith_runtime::host::HostSession;

pub(super) async fn report(host: &HostSession) -> TimelineReport {
    let events = match host.client_timeline_events().await {
        Ok(events) => events,
        Err(error) => return TimelineReport::Unavailable(error.to_string()),
    };
    let timeline = runtime_entries(&events);
    let mut entries = timeline.entries;
    if entries.is_empty() {
        entries.extend(host.session().snapshot().manifests.iter().map(|manifest| {
            TimelineEntry::RootManifest {
                turn: manifest.turn.to_string(),
                provider: manifest.manifest.model.provider.to_string(),
                model: manifest.manifest.model.model.to_string(),
                activated_capabilities: manifest.manifest.activation.len(),
            }
        }));
    }
    if let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
    {
        entries.extend(
            coordinator
                .list()
                .into_iter()
                .filter(|child| !timeline.children.contains(&child.child))
                .map(|child| TimelineEntry::ChildSnapshot {
                    child: child.child.to_string(),
                    session: child.session.to_string(),
                    durability: format!("{:?}", child.durability),
                    state: format!("{:?}", child.state),
                    resumable: child.resumable(),
                    turns: crate::submission::turns_label(child.turns_used, child.max_turns),
                }),
        );
    }
    entries.extend(
        host.changes()
            .timeline()
            .into_iter()
            .enumerate()
            .map(|(index, detail)| TimelineEntry::Recovery {
                number: index + 1,
                detail,
            }),
    );
    if entries.len() > 100 {
        entries.drain(..entries.len().saturating_sub(100));
    }
    if entries.is_empty() {
        TimelineReport::Empty
    } else {
        TimelineReport::Entries(entries)
    }
}

#[derive(Debug, Default)]
pub(crate) struct RuntimeTimeline {
    pub(crate) entries: Vec<TimelineEntry>,
    pub(crate) children: BTreeSet<ChildId>,
}

#[derive(Debug, Default)]
struct TurnTimelineState {
    plan: Option<TimelinePlan>,
    passed_gates: u32,
    failed_gates: u32,
}

pub(crate) fn runtime_entries(events: &[SmithEvent]) -> RuntimeTimeline {
    let mut timeline = RuntimeTimeline::default();
    let mut turns = BTreeMap::<String, TurnTimelineState>::new();
    let mut tools = BTreeMap::<String, String>::new();

    for event in events {
        match &event.payload {
            SmithEventKind::PlanUpdated { counts, .. } => {
                if let Some(turn) = &event.turn {
                    let count = |status: &str| counts.get(status).copied().unwrap_or_default();
                    turns.entry(turn.as_str().to_owned()).or_default().plan = Some(TimelinePlan {
                        active: count("in_progress"),
                        pending: count("pending"),
                        done: count("completed"),
                        cancelled: count("cancelled"),
                    });
                }
            }
            SmithEventKind::ToolCallRequested { call, name, .. } => {
                tools.insert(call.as_str().to_owned(), name.clone());
            }
            SmithEventKind::ToolCallCompleted { call, is_error, .. }
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
            SmithEventKind::TurnCompleted { finish, .. } => {
                if let Some(turn) = &event.turn {
                    let state = turns.remove(turn.as_str()).unwrap_or_default();
                    timeline.entries.push(TimelineEntry::RootTurn {
                        turn: turn.to_string(),
                        finish: turn_finish_value(finish),
                        plan: state.plan,
                        passed_gates: state.passed_gates,
                        failed_gates: state.failed_gates,
                    });
                }
            }
            SmithEventKind::ChildSpawned {
                child,
                workspace,
                max_turns,
                ..
            } => {
                timeline.children.insert(child.clone());
                timeline.entries.push(TimelineEntry::ChildEvent {
                    child: child.to_string(),
                    event: TimelineChildEvent::Started {
                        workspace: format!("{workspace:?}"),
                        turn_limit: (*max_turns != u32::MAX).then_some(*max_turns),
                    },
                });
            }
            SmithEventKind::ChildNeedsInput { child, .. } => {
                timeline.children.insert(child.clone());
                timeline.entries.push(TimelineEntry::ChildEvent {
                    child: child.to_string(),
                    event: TimelineChildEvent::NeedsInput,
                });
            }
            SmithEventKind::ChildCompleted { child, .. } => {
                timeline.children.insert(child.clone());
                timeline.entries.push(TimelineEntry::ChildEvent {
                    child: child.to_string(),
                    event: TimelineChildEvent::Completed,
                });
            }
            SmithEventKind::ChildStopped { child, reason } => {
                timeline.children.insert(child.clone());
                timeline.entries.push(TimelineEntry::ChildEvent {
                    child: child.to_string(),
                    event: TimelineChildEvent::Stopped {
                        reason: format!("{reason:?}"),
                    },
                });
            }
            SmithEventKind::ChildFailed { child, .. } => {
                timeline.children.insert(child.clone());
                timeline.entries.push(TimelineEntry::ChildEvent {
                    child: child.to_string(),
                    event: TimelineChildEvent::Failed,
                });
            }
            _ => {}
        }
    }

    timeline
}

fn turn_finish_value(finish: &TurnFinish) -> String {
    match finish {
        TurnFinish::Completed => "completed".to_owned(),
        TurnFinish::Cancelled { reason } => format!("cancelled ({reason:?})"),
        TurnFinish::LimitReached { limit } => format!("limit reached ({limit:?})"),
        TurnFinish::NeedsInput { request } => format!("needs input ({request})"),
        TurnFinish::Failed => "failed".to_owned(),
    }
}
