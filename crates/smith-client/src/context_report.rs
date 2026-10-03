//! The local `/context` snapshot and its plain-text rendering.
//!
//! Values carry no field labels. Category kinds, usage availability, and
//! compaction state select presentation without inspecting display text.

/// Context information captured when `/context` is invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextReport {
    /// Named context windows offered by the active model.
    pub available_windows: Vec<ContextWindow>,
    /// Model and latest input occupancy, including availability.
    pub summary: String,
    /// Availability and counting provenance of the latest plan.
    pub usage: ContextUsage,
    /// Ordered input categories, including categories with zero tokens.
    pub categories: Vec<ContextCategory>,
    /// Remaining input capacity.
    pub free_input: ContextCapacity,
    /// Output and reasoning capacity held out of the input budget.
    pub reserve: ContextCapacity,
    /// Total model window and enforced input budget.
    pub model_window: String,
    /// Counting provenance and segment count, or their availability.
    pub counting: String,
    /// Compaction state and recovery target.
    pub compaction: ContextCompaction,
    /// Tool output offloading policy.
    pub tool_context: String,
    /// Cumulative provider input for this session.
    pub provider_input: String,
    /// Cumulative cache read for this session.
    pub cache_read: String,
    /// Cache state and lifecycle diagnostics.
    pub cache: String,
    /// Effective reasoning state, effort, and selection source.
    pub reasoning: String,
    /// Supported reasoning controls and their source.
    pub reasoning_controls: String,
}

/// A named model context window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWindow {
    /// Window name.
    pub name: String,
    /// Whether this is the selected window.
    pub active: bool,
}

/// Availability and provenance of the latest input plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextUsage {
    /// No request has been planned yet.
    Unavailable,
    /// An authoritative tokenizer counted the request.
    Exact,
    /// A fallback estimator counted the request.
    Estimated,
}

impl ContextUsage {
    /// The existing category heading for this plan state.
    pub fn category_heading(self) -> &'static str {
        match self {
            Self::Unavailable => "Available capacity",
            Self::Exact => "Exact usage by category",
            Self::Estimated => "Estimated usage by category",
        }
    }
}

/// Semantic category used by the occupancy grid and legend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextCategoryKind {
    /// System, developer, or ability instructions.
    System,
    /// Tool schemas or tool results.
    Tool,
    /// History, memory, or retrieval.
    History,
    /// A compacted summary.
    Summary,
    /// User input.
    Input,
    /// Another runtime-defined category.
    Other,
    /// Remaining input capacity.
    Free,
    /// Output and reasoning reserve.
    Reserve,
}

impl ContextCategoryKind {
    fn plain_glyph(self) -> &'static str {
        match self {
            Self::System => "■",
            Self::Tool => "◆",
            Self::History => "●",
            Self::Summary => "▲",
            Self::Input => "✦",
            Self::Other => "+",
            Self::Free => "·",
            Self::Reserve => "□",
        }
    }
}

/// One input category in the legend and occupancy grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCategory {
    /// Semantic category, independent of its display label.
    pub kind: ContextCategoryKind,
    /// Existing category name, including runtime-defined names.
    pub label: String,
    /// Grid weight; zero-token categories remain visible in the legend.
    pub tokens: u32,
    /// Count and percentage, or their availability.
    pub value: String,
}

/// Capacity in the grid and legend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCapacity {
    /// Grid weight in tokens.
    pub tokens: u32,
    /// Count and optional percentage as displayed by the host.
    pub value: String,
}

/// Compaction details for the latest request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextCompaction {
    /// Compaction is available when the input overflows.
    Enabled {
        /// Recovery target token count.
        recovery_target: String,
    },
    /// The request contains a compacted summary.
    Applied {
        /// Summary token count with counting provenance.
        summary: String,
        /// Recovery target token count.
        recovery_target: String,
    },
}

impl ContextCompaction {
    /// The existing compaction display value, without a field label.
    pub fn render_value(&self) -> String {
        match self {
            Self::Enabled { recovery_target } => {
                format!("enabled on overflow · {recovery_target} recovery target")
            }
            Self::Applied {
                summary,
                recovery_target,
            } => format!("applied · {summary} summary · {recovery_target} recovery target"),
        }
    }
}

impl ContextReport {
    /// The existing heading at the start of the context view.
    pub const HEADING: &str = "Context usage";
    /// The existing distinction between occupancy and cumulative usage.
    pub const OCCUPANCY_HINT: &str =
        "Input occupancy above is the last planned request, not cumulative session usage.";

    /// The existing named-window display value, without a field label.
    pub fn available_windows_value(&self) -> String {
        self.available_windows
            .iter()
            .map(|window| {
                if window.active {
                    format!("{} (active)", window.name)
                } else {
                    window.name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Whether every occupancy weight is zero.
    pub fn grid_is_empty(&self) -> bool {
        self.categories.iter().all(|category| category.tokens == 0)
            && self.free_input.tokens == 0
            && self.reserve.tokens == 0
    }

    /// Allocates the existing fifty-cell grid from semantic token weights.
    /// Both plain and terminal renderers choose their own glyphs for the cells.
    pub fn grid(&self) -> Vec<Vec<ContextCategoryKind>> {
        const CELLS: usize = 50;
        const COLUMNS: usize = 10;

        let mut entries = self
            .categories
            .iter()
            .map(|category| (category.kind, category.tokens))
            .collect::<Vec<_>>();
        entries.push((ContextCategoryKind::Free, self.free_input.tokens));
        entries.push((ContextCategoryKind::Reserve, self.reserve.tokens));
        entries.retain(|(_, tokens)| *tokens > 0);
        if entries.is_empty() {
            return vec![vec![ContextCategoryKind::Free; COLUMNS]; CELLS / COLUMNS];
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
            .flat_map(|((kind, _), count)| std::iter::repeat_n(*kind, count))
            .take(CELLS)
            .collect::<Vec<_>>();
        cells.chunks(COLUMNS).map(|row| row.to_vec()).collect()
    }
}

/// Renders the transcript body for the existing plain-text capture surface.
/// No terminal libraries or title/label parsing are involved.
pub fn render_plain(report: &ContextReport) -> String {
    let mut lines = vec![ContextReport::HEADING.to_owned()];
    if !report.available_windows.is_empty() {
        lines.push(format!(
            "available context windows: {}",
            report.available_windows_value(),
        ));
    }
    lines.push(report.summary.clone());
    lines.push(String::new());
    lines.extend(report.grid().iter().map(|row| {
        let mut line = row
            .iter()
            .map(|kind| kind.plain_glyph())
            .collect::<Vec<_>>()
            .join(" ");
        if report.grid_is_empty() {
            line.push(' ');
        }
        line
    }));
    lines.push(String::new());
    lines.push(report.usage.category_heading().to_owned());
    lines.extend(report.categories.iter().map(|category| {
        format!(
            "{} {}: {}",
            category.kind.plain_glyph(),
            category.label,
            category.value,
        )
    }));
    lines.extend([
        format!("· free input: {}", report.free_input.value),
        format!("□ output/reasoning reserve: {}", report.reserve.value),
        format!("model window: {}", report.model_window),
        format!("counting: {}", report.counting),
        format!("compaction: {}", report.compaction.render_value()),
        format!("tool context: {}", report.tool_context),
        ContextReport::OCCUPANCY_HINT.to_owned(),
        format!("provider input (session): {}", report.provider_input),
        format!("cache read (session): {}", report.cache_read),
        format!("cache: {}", report.cache),
        format!("reasoning: {}", report.reasoning),
        format!("reasoning controls: {}", report.reasoning_controls),
    ]);
    lines.join("\n")
}
