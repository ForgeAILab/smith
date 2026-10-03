//! Local shell-shortcut output and its plain-text rendering.

/// The output of a local `!command`, excluded from model history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellReport {
    /// Command output, with an explicit empty outcome.
    pub output: ShellOutput,
    /// Whether the tool reported a failure.
    pub is_error: bool,
}

/// Free-text output or a command with nothing to display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellOutput {
    /// The command returned only whitespace or no text.
    Empty,
    /// The command's output, without inferred fields or labels.
    Output(String),
}

impl ShellReport {
    /// The existing message when a command has no displayable output.
    pub const EMPTY_MESSAGE: &str = "No output.";

    /// Captures output and classifies an empty result before rendering.
    pub fn new(output: impl Into<String>, is_error: bool) -> Self {
        let output = output.into();
        Self {
            output: if output.trim().is_empty() {
                ShellOutput::Empty
            } else {
                ShellOutput::Output(output)
            },
            is_error,
        }
    }
}

/// Renders the existing transcript body without terminal dependencies.
pub fn render_plain(report: &ShellReport) -> String {
    match &report.output {
        ShellOutput::Empty => ShellReport::EMPTY_MESSAGE.to_owned(),
        ShellOutput::Output(output) => output.clone(),
    }
}
