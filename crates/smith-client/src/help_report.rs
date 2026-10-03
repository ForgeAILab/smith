//! The local `/help` guide and its plain-text rendering.
//!
//! Command names, argument hints, and descriptions remain separate fields.
//! The terminal client selects presentation from the guide's sections.

/// The command guide captured when `/help` is invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpReport {
    /// Instruction preceding the getting-started commands.
    pub introduction: String,
    /// Short command suggestions for getting started.
    pub getting_started: Vec<HelpCommand>,
    /// Commands in the primary registry group, in discovery order.
    pub primary: Vec<HelpCommand>,
    /// Commands in the advanced registry group, in discovery order.
    pub advanced: Vec<HelpCommand>,
    /// Plain-text capture guidance, kept separately so terminal key wording
    /// does not change the recorder's existing plain-text output.
    pub composer: Vec<String>,
    /// Keys and input forms offered by the terminal, in plain words.
    pub keys: Vec<HelpKey>,
}

impl HelpReport {
    /// The terminal's short orientation section.
    pub const START_HERE_HEADING: &str = "Start here";
    /// The terminal's key table heading.
    pub const KEYS_HEADING: &str = "Keys";
    /// The existing heading for the getting-started section.
    pub const GETTING_STARTED_HEADING: &str = "Getting started";
    /// The existing heading for primary commands.
    pub const PRIMARY_HEADING: &str = "Primary";
    /// The existing heading for advanced commands.
    pub const ADVANCED_HEADING: &str = "Advanced";
    /// The existing heading for composer guidance.
    pub const COMPOSER_HEADING: &str = "Composer";
}

/// One terminal key or input form and its action, without layout delimiters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpKey {
    /// Key name and any condition needed to use it.
    pub key: String,
    /// The action in plain words.
    pub description: String,
}

/// One command's display values, without presentation delimiters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpCommand {
    /// Registry name without the leading slash.
    pub name: String,
    /// Optional argument syntax, empty for bare command suggestions.
    pub argument_hint: String,
    /// Existing command description or getting-started suggestion.
    pub description: String,
}

impl HelpCommand {
    /// The existing command invocation, without its description.
    pub fn invocation(&self) -> String {
        if self.argument_hint.is_empty() {
            format!("/{}", self.name)
        } else {
            format!("/{} {}", self.name, self.argument_hint)
        }
    }
}

/// Renders the transcript body for the existing plain-text capture surface.
/// No terminal libraries or title/label parsing are involved.
pub fn render_plain(report: &HelpReport) -> String {
    let mut lines = vec![
        HelpReport::GETTING_STARTED_HEADING.to_owned(),
        report.introduction.clone(),
    ];
    lines.extend(report.getting_started.iter().map(plain_command));
    for (heading, commands) in [
        (HelpReport::PRIMARY_HEADING, &report.primary),
        (HelpReport::ADVANCED_HEADING, &report.advanced),
    ] {
        lines.push(String::new());
        lines.push(heading.to_owned());
        lines.extend(commands.iter().map(plain_command));
    }
    lines.push(String::new());
    lines.push(HelpReport::COMPOSER_HEADING.to_owned());
    lines.extend(report.composer.iter().cloned());
    lines.join("\n")
}

fn plain_command(command: &HelpCommand) -> String {
    format!("{} — {}", command.invocation(), command.description)
}
