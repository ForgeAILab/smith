use std::fmt;
use std::sync::Arc;

use agent_runtime::harness::{
    ComponentDescriptor, ComponentPhase, ContextContributor, HistoryProjector, ModelInterceptor,
    ToolOutputProcessor, ToolViewResolver, TurnCommitHook,
};
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::tool::Tool;

/// Executable values and bounded client declarations returned by a module.
#[derive(Debug, Clone)]
pub enum ModuleContribution {
    /// Tool governed by the shared registry, security, and approval contracts.
    Tool(Arc<dyn Tool>),
    /// One component in an existing Agent Runtime harness phase.
    Pipeline(PipelineComponent),
    /// Observe-only canonical event sink.
    Observer {
        /// Stable evidence identity; the observer contract has no descriptor.
        name: String,
        /// Shared canonical observer implementation.
        observer: Arc<dyn EventObserver>,
    },
    /// Declaration only; command dispatch is host-owned.
    Command(SlashCommand),
    /// Live bounded declaration; clients own rendering.
    StatusItem {
        /// Stable evidence identity.
        name: String,
        /// Cheap, non-blocking source of the current declaration.
        source: Arc<dyn StatusSource>,
    },
}

/// The six shared harness seams, with no parallel execution contract.
#[derive(Debug, Clone)]
pub enum PipelineComponent {
    /// Projects canonical history.
    HistoryProjector(Arc<dyn HistoryProjector>),
    /// Contributes authoritative context.
    ContextContributor(Arc<dyn ContextContributor>),
    /// Narrows the tool view.
    ToolViewResolver(Arc<dyn ToolViewResolver>),
    /// Patches non-context model options.
    ModelInterceptor(Arc<dyn ModelInterceptor>),
    /// Processes exact tool outcomes.
    ToolOutputProcessor(Arc<dyn ToolOutputProcessor>),
    /// Runs after a terminal commit.
    TurnCommitHook(Arc<dyn TurnCommitHook>),
}

impl PipelineComponent {
    /// Shared phase used for evidence and phase-scoped collision checks.
    pub fn phase(&self) -> ComponentPhase {
        match self {
            Self::HistoryProjector(_) => ComponentPhase::History,
            Self::ContextContributor(_) => ComponentPhase::Context,
            Self::ToolViewResolver(_) => ComponentPhase::ToolView,
            Self::ModelInterceptor(_) => ComponentPhase::Model,
            Self::ToolOutputProcessor(_) => ComponentPhase::ToolOutput,
            Self::TurnCommitHook(_) => ComponentPhase::TurnCommit,
        }
    }

    /// Shared metadata, including pipeline-owned ordering constraints.
    pub fn descriptor(&self) -> ComponentDescriptor {
        match self {
            Self::HistoryProjector(component) => component.descriptor(),
            Self::ContextContributor(component) => component.descriptor(),
            Self::ToolViewResolver(component) => component.descriptor(),
            Self::ModelInterceptor(component) => component.descriptor(),
            Self::ToolOutputProcessor(component) => component.descriptor(),
            Self::TurnCommitHook(component) => component.descriptor(),
        }
    }
}

/// Plain slash-command declaration; it carries no execution or renderer handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommand {
    /// Command name without the slash.
    pub name: String,
    /// Short user-facing description.
    pub description: String,
}

/// Live status declaration, without a renderer or canonical state mutation.
pub trait StatusSource: Send + Sync + fmt::Debug {
    /// Returns the current bounded value; must be cheap and non-blocking.
    fn current(&self) -> Option<StatusItem>;
}

/// Maximum Unicode scalar values in a contributed status label.
pub const MAX_STATUS_LABEL_CHARS: usize = 80;

/// Optional presentation emphasis, independent of terminal styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusSeverity {
    /// Informational state.
    Info,
    /// Attention requested.
    Warning,
    /// A module reports a failure.
    Error,
}

/// Bounded declarative status data; clients can apply a tighter display bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusItem {
    name: String,
    label: String,
    severity: Option<StatusSeverity>,
}

impl StatusItem {
    /// Bounds labels before a client sees them, without splitting UTF-8.
    pub fn new(
        name: impl Into<String>,
        label: impl AsRef<str>,
        severity: Option<StatusSeverity>,
    ) -> Self {
        Self {
            name: name.into(),
            label: label
                .as_ref()
                .chars()
                .take(MAX_STATUS_LABEL_CHARS)
                .collect(),
            severity,
        }
    }

    /// Stable evidence name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Bounded label for client rendering.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Optional presentation emphasis.
    pub fn severity(&self) -> Option<StatusSeverity> {
        self.severity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_unicode_status_labels_remain_bounded_and_valid_utf8() {
        let item = StatusItem::new(
            "pressure",
            "警".repeat(MAX_STATUS_LABEL_CHARS + 10),
            Some(StatusSeverity::Warning),
        );
        assert_eq!(item.label().chars().count(), MAX_STATUS_LABEL_CHARS);
        assert_eq!(item.severity(), Some(StatusSeverity::Warning));
        assert_eq!(item.name(), "pressure");
    }
}
