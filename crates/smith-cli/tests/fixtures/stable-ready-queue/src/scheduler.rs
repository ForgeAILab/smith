use std::collections::{BTreeMap, BTreeSet};

/// One declared unit of work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Stable task identity.
    pub id: String,
    /// Task identities that must complete in an earlier batch.
    pub dependencies: Vec<String>,
}

impl Task {
    /// Creates a task from string-like values.
    pub fn new<I, S>(id: impl Into<String>, dependencies: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            id: id.into(),
            dependencies: dependencies.into_iter().map(Into::into).collect(),
        }
    }
}

/// Why a task list could not be scheduled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    /// More than one task declared the same identity.
    DuplicateTask(String),
    /// A dependency does not name any declared task.
    UnknownDependency {
        /// The task containing the invalid dependency.
        task: String,
        /// The missing task identity.
        dependency: String,
    },
    /// The remaining task identities participate in, or are blocked by, a cycle.
    Cycle(Vec<String>),
}

/// Produces stable parallel batches for `tasks`.
///
/// Every task appears exactly once. Dependencies always appear in earlier
/// batches, never merely earlier in the same batch. Tasks that become ready
/// together retain their original declaration order. Duplicate identities and
/// unknown dependencies are reported before cycle detection. Cycle members are
/// returned in ascending identity order for deterministic diagnostics.
pub fn schedule_batches(tasks: &[Task]) -> Result<Vec<Vec<String>>, ScheduleError> {
    // This implementation predates the stable-order and validation contract.
    // It intentionally uses an ordered map, which also hides duplicate IDs.
    let mut remaining = tasks
        .iter()
        .map(|task| (task.id.clone(), task.dependencies.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut completed = BTreeSet::new();
    let mut batches = Vec::new();

    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .filter(|(_, dependencies)| {
                dependencies
                    .iter()
                    .all(|dependency| completed.contains(dependency))
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();

        if ready.is_empty() {
            return Err(ScheduleError::Cycle(remaining.into_keys().collect()));
        }

        for id in &ready {
            remaining.remove(id);
            completed.insert(id.clone());
        }
        batches.push(ready);
    }

    Ok(batches)
}
