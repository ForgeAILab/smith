//! Local file-command listings, reload results, and trust outcomes.

use crate::file_commands::{CatalogEntry, CommandCatalog, CommandLayer, CommandProblem};

/// The result of inspecting or changing the file-command catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandsReport {
    /// Neither layer contains commands or discovery problems.
    Empty,
    /// A local command or trust decision was refused.
    Error(String),
    /// All declarations by layer, followed by refused files.
    Indexed {
        /// Nonempty layers in project, user order.
        groups: Vec<CommandGroup>,
        /// Every discovery refusal.
        problems: Vec<CommandProblem>,
    },
    /// The confirmed project content is admitted in this session.
    Trusted {
        /// Command name without the slash.
        name: String,
        /// Content identity covered by the decision.
        digest: String,
    },
    /// Discovery was rebuilt from disk.
    Reloaded {
        /// Number of admitted winners.
        runnable: usize,
        /// Number of declarations, including withheld and shadowed entries.
        entries: usize,
        /// Every discovery refusal after rebuilding.
        problems: Vec<CommandProblem>,
    },
}

/// One source layer, including withheld and shadowed declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandGroup {
    /// Where the declarations were found.
    pub layer: CommandLayer,
    /// Bounded metadata and admission state.
    pub entries: Vec<CatalogEntry>,
}

impl CommandsReport {
    /// Captures every declaration and discovery problem without reading files.
    pub fn from_catalog(catalog: &CommandCatalog) -> Self {
        let groups = [CommandLayer::Project, CommandLayer::User]
            .into_iter()
            .filter_map(|layer| {
                let entries = catalog
                    .entries()
                    .iter()
                    .filter(|entry| entry.command.layer == layer)
                    .cloned()
                    .collect::<Vec<_>>();
                (!entries.is_empty()).then_some(CommandGroup { layer, entries })
            })
            .collect::<Vec<_>>();
        let problems = catalog.problems().to_vec();
        if groups.is_empty() && problems.is_empty() {
            Self::Empty
        } else {
            Self::Indexed { groups, problems }
        }
    }
}

/// Renders the local transcript body for plain-text capture.
pub fn render_plain(report: &CommandsReport) -> String {
    let mut lines = Vec::new();
    let problems = match report {
        CommandsReport::Empty => return "no file commands were found on disk".to_owned(),
        CommandsReport::Error(error) => return error.clone(),
        CommandsReport::Trusted { name, digest } => {
            return format!("`/{name}` is trusted at {digest}; it is available in this session");
        }
        CommandsReport::Indexed { groups, problems } => {
            for group in groups {
                lines.push(group.layer.label().to_owned());
                for entry in &group.entries {
                    lines.push(format!(
                        "  /{} · {} · {}",
                        entry.command.name,
                        entry.state.reason(&entry.command.name),
                        entry.command.description
                    ));
                }
            }
            problems
        }
        CommandsReport::Reloaded {
            runnable,
            entries,
            problems,
        } => {
            lines.push(format!(
                "reloaded file commands: {runnable} runnable · {entries} entries · {} problems",
                problems.len()
            ));
            problems
        }
    };
    if !problems.is_empty() {
        lines.push("not loaded".to_owned());
        for problem in problems {
            lines.push(format!(
                "  {} /{} · {} · {}",
                problem.layer.label(),
                problem.name,
                problem.reason,
                problem.path.display()
            ));
        }
    }
    lines.join("\n")
}
