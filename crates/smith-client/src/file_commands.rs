//! Finding file-backed prompt templates and resolving their layers and trust.
//!
//! A command lives at `<root>/commands/<name>.md`; the file stem names it,
//! and frontmatter cannot rename it. Discovery keeps declarations and named
//! problems, never bodies: invocation reads the current bytes again.
//!
//! Project-root containment and trust are applied by the resolver via
//! [`smith_config::trust::Executable::from_file`]. This module reads the path
//! as given and grants no authority to a project command.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use smith_config::trust::{ContentDigest, Executable, ExecutableKind, TrustStatus, TrustStore};
use smith_runtime::harness::{
    CapabilitySet, Contribution, ModuleId, ModuleProvenance, ModuleRevision, ModuleSpec,
    ModuleTrust,
};

/// The directory beneath each state root that holds command files.
const COMMANDS_DIR: &str = "commands";

/// How many sorted `.md` file candidates one layer may read.
pub const MAX_COMMANDS_PER_LAYER: usize = 256;

/// How large a command file may be, including frontmatter, in bytes.
pub const MAX_BODY_BYTES: u64 = 64 * 1024;

/// How many lines after the opening delimiter may include the closing one.
pub const MAX_FRONT_MATTER_LINES: usize = 64;

/// How long an indexed description may be, in Unicode characters.
///
/// Explicit values over this bound are refused, as for skills. A description
/// taken from the body is truncated instead, so missing metadata never hides
/// an otherwise usable command.
pub const MAX_DESCRIPTION_CHARS: usize = 256;

/// How long an explicit argument hint may be, in Unicode characters.
///
/// Values over this bound are refused rather than silently altered.
pub const MAX_ARGUMENT_HINT_CHARS: usize = 128;

/// The source of a file command, without any trust or shadowing decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandLayer {
    /// The user's state root.
    User,
    /// The project's `.smith/` state root.
    Project,
}

impl CommandLayer {
    /// The stable label used by command surfaces.
    pub const fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

/// One command declaration read from disk, identified by the bytes read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCommand {
    /// The file stem, without a leading slash.
    pub name: String,
    /// The state root that supplied the command.
    pub layer: CommandLayer,
    /// The bounded description shown in discovery surfaces.
    pub description: String,
    /// Optional bounded argument syntax shown beside the command.
    pub argument_hint: Option<String>,
    /// Where invocation must re-read the body.
    pub path: PathBuf,
    /// Identity of the complete file, including frontmatter.
    pub content: ContentDigest,
}

/// A command file Smith found and could not use, kept for a surface to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandProblem {
    /// The file stem, or the file name when no stem is available.
    pub name: String,
    /// The state root that supplied the rejected file.
    pub layer: CommandLayer,
    /// The path that was rejected.
    pub path: PathBuf,
    /// Why it was rejected.
    pub reason: String,
}

impl CommandProblem {
    fn new(path: PathBuf, layer: CommandLayer, reason: impl Into<String>) -> Self {
        Self {
            name: problem_name(&path),
            layer,
            path,
            reason: reason.into(),
        }
    }
}

/// What one layer's command directory yielded.
#[derive(Debug, Clone, Default)]
pub struct LayerDiscovery {
    /// Usable declarations, ordered by name.
    pub commands: Vec<FileCommand>,
    /// Files that were refused, ordered by name.
    pub problems: Vec<CommandProblem>,
}

/// A freshly read command, with its body and identity from the same bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedCommand {
    /// The explicit description or bounded first non-empty body line.
    pub description: String,
    /// Optional bounded argument syntax.
    pub argument_hint: Option<String>,
    /// Prompt text after optional frontmatter, without further interpretation.
    pub body: String,
    /// Identity of the complete file, including frontmatter.
    pub content: ContentDigest,
}

/// Whether a discovered command may supply prompt text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandState {
    /// Admitted at this layer.
    Runnable,
    /// No decision covers this project file.
    NeedsTrust,
    /// A previous decision covers different content.
    Changed,
    /// This content was explicitly refused.
    Denied,
    /// A trusted project command wins this user command's name.
    Shadowed,
}

impl CommandState {
    /// A short listing or invocation reason, including the approval route.
    pub fn reason(self, name: &str) -> String {
        match self {
            Self::Runnable => "runnable".to_owned(),
            Self::NeedsTrust => format!("needs approval: /commands trust {name}"),
            Self::Changed => format!("content changed: /commands trust {name}"),
            Self::Denied => format!("approval denied: /commands trust {name}"),
            Self::Shadowed => "shadowed by a trusted project command".to_owned(),
        }
    }
}

impl From<TrustStatus> for CommandState {
    fn from(status: TrustStatus) -> Self {
        match status {
            TrustStatus::Trusted => Self::Runnable,
            TrustStatus::Untrusted => Self::NeedsTrust,
            TrustStatus::Changed => Self::Changed,
            TrustStatus::Denied => Self::Denied,
        }
    }
}

/// A declaration retained even when trust or a higher layer withholds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    /// Bounded metadata and the path invocation re-reads.
    pub command: FileCommand,
    /// Admission or withholding decision at discovery time.
    pub state: CommandState,
}

/// Resolved command metadata, ordered by name with project before user.
#[derive(Debug, Clone, Default)]
pub struct CommandCatalog {
    entries: Vec<CatalogEntry>,
    problems: Vec<CommandProblem>,
}

impl CommandCatalog {
    /// An empty catalog for clients that have no file-command basis.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Discovers fixed user and project directories without creating them.
    ///
    /// Read, containment, and trust lookup failures become named problems.
    pub fn discover(user_root: Option<&Path>, project: Option<&Path>, trust: &TrustStore) -> Self {
        let mut catalog = Self::empty();
        if let Some(root) = user_root {
            let found = discover_layer(root, CommandLayer::User);
            catalog.problems.extend(found.problems);
            catalog
                .entries
                .extend(found.commands.into_iter().map(|command| CatalogEntry {
                    command,
                    state: CommandState::Runnable,
                }));
        }
        if let Some(project) = project {
            let found = discover_layer(&project.join(".smith"), CommandLayer::Project);
            catalog.problems.extend(found.problems);
            for command in found.commands {
                match project_state(project, trust, &command.path, &command.content) {
                    Ok(state) => catalog.entries.push(CatalogEntry { command, state }),
                    Err(reason) => catalog.problems.push(CommandProblem::new(
                        command.path,
                        CommandLayer::Project,
                        reason,
                    )),
                }
            }
        }
        let winners = catalog
            .entries
            .iter()
            .filter(|entry| {
                entry.command.layer == CommandLayer::Project
                    && entry.state == CommandState::Runnable
            })
            .map(|entry| entry.command.name.clone())
            .collect::<std::collections::BTreeSet<_>>();
        for entry in &mut catalog.entries {
            if entry.command.layer == CommandLayer::User && winners.contains(&entry.command.name) {
                entry.state = CommandState::Shadowed;
            }
        }
        catalog.entries.sort_by(|left, right| {
            left.command.name.cmp(&right.command.name).then_with(|| {
                layer_order(left.command.layer).cmp(&layer_order(right.command.layer))
            })
        });
        catalog.problems.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| layer_order(left.layer).cmp(&layer_order(right.layer)))
                .then_with(|| left.path.cmp(&right.path))
        });
        catalog
    }

    /// All declarations, including withheld and shadowed entries.
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// Every refusal encountered during discovery.
    pub fn problems(&self) -> &[CommandProblem] {
        &self.problems
    }

    /// The runnable winner, or the withheld project entry if no user fallback exists.
    pub fn resolve(&self, name: &str) -> Option<&CatalogEntry> {
        let mut withheld = None;
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.command.name == name)
        {
            if entry.state == CommandState::Runnable {
                return Some(entry);
            }
            if entry.command.layer == CommandLayer::Project {
                withheld = Some(entry);
            }
        }
        withheld
    }

    /// Admitted winners in the catalog's deterministic name order.
    pub fn runnable(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.state == CommandState::Runnable)
    }

    /// Re-reads the selected file and rechecks project trust before expanding text.
    ///
    /// # Errors
    ///
    /// Refuses missing, malformed, escaping, or no-longer-trusted commands.
    /// A selected project command losing trust never silently falls back.
    pub fn prepare(
        &self,
        name: &str,
        arguments: &str,
        project: Option<&Path>,
        trust: &TrustStore,
    ) -> Result<PreparedCommand, InvocationRefusal> {
        let refuse = |reason| InvocationRefusal {
            name: name.to_owned(),
            reason,
        };
        let entry = self
            .resolve(name)
            .ok_or_else(|| refuse("command is not in the catalog".to_owned()))?;
        let loaded = read_command(&entry.command.path).map_err(refuse)?;
        if entry.command.layer == CommandLayer::Project {
            let project =
                project.ok_or_else(|| refuse("project root is unavailable".to_owned()))?;
            let state = project_state(project, trust, &entry.command.path, &loaded.content)
                .map_err(refuse)?;
            if state != CommandState::Runnable {
                return Err(refuse(state.reason(name)));
            }
        }
        Ok(PreparedCommand {
            prompt: expand(loaded.body.trim(), arguments).trim().to_owned(),
            name: name.to_owned(),
            layer: entry.command.layer,
        })
    }
}

/// Expanded prompt text; preparing a command executes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommand {
    /// Prompt ready for ordinary user submission.
    pub prompt: String,
    /// The selected command's name.
    pub name: String,
    /// The layer whose current file supplied the prompt.
    pub layer: CommandLayer,
}

/// A local refusal that must not submit a provider request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationRefusal {
    /// The command the user requested.
    pub name: String,
    /// Why it cannot run, with an approval route when trust is missing.
    pub reason: String,
}

impl std::fmt::Display for InvocationRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "`/{}` cannot run: {}", self.name, self.reason)
    }
}

impl std::error::Error for InvocationRefusal {}

fn layer_order(layer: CommandLayer) -> u8 {
    match layer {
        CommandLayer::Project => 0,
        CommandLayer::User => 1,
    }
}

fn project_state(
    project: &Path,
    trust: &TrustStore,
    path: &Path,
    content: &ContentDigest,
) -> Result<CommandState, String> {
    let executable = Executable::from_file(project, ExecutableKind::SlashCommand, path)
        .map_err(|error| error.message)?;
    // Match the identity to the bytes the reader actually supplied.
    if executable.digest() != content {
        return Ok(CommandState::Changed);
    }
    trust
        .status(project, &executable)
        .map(CommandState::from)
        .map_err(|error| error.message)
}

/// Records runnable winners as content-only modules with their source layers.
pub fn contribution_modules(catalog: &CommandCatalog) -> Vec<ModuleSpec> {
    catalog
        .runnable()
        .map(|entry| {
            let command = &entry.command;
            let layer = command.layer.label();
            ModuleSpec {
                id: ModuleId::parse(format!("commands/{layer}/{}", command.name))
                    .expect("validated command name fits a module id"),
                revision: ModuleRevision::parse(command.content.as_hex())
                    .expect("content digest fits a module revision"),
                provenance: ModuleProvenance::UserManifest(format!(
                    "{layer}:{}",
                    command.path.display()
                )),
                trust: ModuleTrust::ContentOnly,
                contributions: vec![Contribution::Command {
                    name: command.name.clone(),
                }],
                requested_capabilities: CapabilitySet::new(),
                granted_capabilities: CapabilitySet::new(),
            }
        })
        .collect()
}

/// Reads `<root>/commands/*.md` for a user state root or a project's `.smith/`.
///
/// A missing directory is empty and creates nothing. Candidates are sorted
/// before reading; each beyond the count bound is reported by name. A refused
/// candidate still counts toward the read bound, so malformed files cannot
/// make discovery read an unbounded number of bodies.
pub fn discover_layer(root: &Path, layer: CommandLayer) -> LayerDiscovery {
    let directory = root.join(COMMANDS_DIR);
    let mut found = LayerDiscovery::default();
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return found,
        Err(error) => {
            found.problems.push(CommandProblem::new(
                directory,
                layer,
                format!("its command directory cannot be read: {error}"),
            ));
            return found;
        }
    };

    let mut candidates = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                found.problems.push(CommandProblem::new(
                    directory.clone(),
                    layer,
                    format!("a directory entry cannot be read: {error}"),
                ));
                continue;
            }
        };
        let path = entry.path();
        if !is_command_path(&path) {
            continue;
        }
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => candidates.push(path),
            Ok(_) => {}
            Err(error) => found.problems.push(CommandProblem::new(
                path,
                layer,
                format!("it cannot be read: {error}"),
            )),
        }
    }
    // The count bound depends on names, never on filesystem iteration order.
    candidates.sort_by(|left, right| {
        left.file_stem()
            .cmp(&right.file_stem())
            .then_with(|| left.cmp(right))
    });
    for (index, path) in candidates.into_iter().enumerate() {
        if index >= MAX_COMMANDS_PER_LAYER {
            found.problems.push(CommandProblem::new(
                path,
                layer,
                format!(
                    "only the first {MAX_COMMANDS_PER_LAYER} command files are read; \
                     it is beyond the limit"
                ),
            ));
            continue;
        }
        match read_command(&path) {
            Ok(loaded) => found.commands.push(FileCommand {
                name: problem_name(&path),
                layer,
                description: loaded.description,
                argument_hint: loaded.argument_hint,
                path,
                content: loaded.content,
            }),
            Err(reason) => found
                .problems
                .push(CommandProblem::new(path, layer, reason)),
        }
    }
    found
        .problems
        .sort_by(|left, right| left.name.cmp(&right.name));
    found
}

/// Re-reads one `.md` command with the same name, metadata, and file bounds.
///
/// The returned digest covers the bytes supplying both metadata and body.
/// Project-root containment and trust belong to the resolver, via
/// [`smith_config::trust::Executable::from_file`], rather than to this reader.
///
/// # Errors
///
/// Returns a refusal reason for an invalid or reserved name, unreadable or
/// oversized file, non-UTF-8 content, malformed frontmatter, or an empty body.
pub fn read_command(path: &Path) -> Result<LoadedCommand, String> {
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "its file stem is not a UTF-8 command name".to_owned())?;
    validate_name(name)?;
    if is_reserved_name(name) {
        return Err(format!(
            "its name `{name}` is reserved for a built-in command"
        ));
    }
    if !is_command_path(path) {
        return Err("a command file must end in `.md`".to_owned());
    }

    let file = fs::File::open(path).map_err(|error| format!("it cannot be read: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("it cannot be read: {error}"))?;
    if !metadata.is_file() {
        return Err("it is not a command file".to_owned());
    }
    if metadata.len() > MAX_BODY_BYTES {
        return Err(size_problem(metadata.len()));
    }
    // Reading at most one extra byte also catches a file growing after stat.
    let mut bytes = Vec::new();
    file.take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("it cannot be read: {error}"))?;
    if bytes.len() as u64 > MAX_BODY_BYTES {
        return Err(size_problem(bytes.len() as u64));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "it is not UTF-8, so its prompt cannot be read".to_owned())?;
    let front_matter = parse_front_matter(text)?;
    if let Some(declared) = &front_matter.name
        && declared != name
    {
        return Err(format!(
            "its frontmatter calls it `{declared}`, but a command is named by its \
             file stem; rename the file or the field so they agree"
        ));
    }
    if front_matter.body.trim().is_empty() {
        return Err("it has no prompt body after its frontmatter".to_owned());
    }
    let description = front_matter.description.unwrap_or_else(|| {
        front_matter
            .body
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .trim()
            .trim_start_matches('#')
            .trim()
            .chars()
            .take(MAX_DESCRIPTION_CHARS)
            .collect()
    });
    Ok(LoadedCommand {
        description,
        argument_hint: front_matter.argument_hint,
        body: front_matter.body.to_owned(),
        content: ContentDigest::of(&bytes),
    })
}

/// Validates a file stem: 1..=64 lowercase ASCII letters, digits, or hyphens.
///
/// # Errors
///
/// Returns a reason when the name falls outside that grammar. Reservation is
/// checked separately by [`is_reserved_name`].
pub fn validate_name(name: &str) -> Result<(), String> {
    if !(1..=64).contains(&name.len())
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(
            "its name must be 1 to 64 lowercase ASCII letters, digits, or hyphens".to_owned(),
        );
    }
    Ok(())
}

/// Whether a name exactly equals a built-in command's name.
pub fn is_reserved_name(name: &str) -> bool {
    crate::commands::COMMANDS
        .iter()
        .any(|command| command.name == name)
}

/// Expands literal `$ARGUMENTS` occurrences once using trimmed arguments.
///
/// Inserted arguments are never expanded again. Without a placeholder,
/// non-empty arguments follow the body, trimmed at its end, after a blank
/// line; empty arguments leave the body unchanged. All other text, including
/// shell syntax and `!` lines, passes through without execution.
pub fn expand(body: &str, arguments: &str) -> String {
    let arguments = arguments.trim();
    if body.contains("$ARGUMENTS") {
        body.replace("$ARGUMENTS", arguments)
    } else if arguments.is_empty() {
        body.to_owned()
    } else {
        format!("{}\n\n{arguments}", body.trim_end())
    }
}

fn problem_name(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn is_command_path(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(".md"))
}

fn size_problem(size: u64) -> String {
    format!("it is {size} bytes, over the {MAX_BODY_BYTES}-byte limit for a command file")
}

/// The interpreted frontmatter fields, and the prompt after them.
struct FrontMatter<'a> {
    name: Option<String>,
    description: Option<String>,
    argument_hint: Option<String>,
    body: &'a str,
}

/// Parses an optional leading `---`, then `key: value` lines, then `---`.
///
/// This is the same narrow grammar skills use, without YAML features. Unknown
/// keys are ignored; an absent or empty description uses the body fallback.
fn parse_front_matter(text: &str) -> Result<FrontMatter<'_>, String> {
    let mut fields = FrontMatter {
        name: None,
        description: None,
        argument_hint: None,
        body: text,
    };
    let mut rest = match strip_line(text, "---") {
        Some(rest) => rest,
        None if text == "---" => "",
        None => return Ok(fields),
    };
    for number in 1..=MAX_FRONT_MATTER_LINES {
        let Some((line, tail)) = next_line(rest) else {
            break;
        };
        rest = tail;
        if line.trim_end() == "---" {
            fields.body = rest;
            return Ok(fields);
        }
        if line.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(format!("its frontmatter line {number} is not `key: value`"));
        };
        let value = value.trim();
        match key.trim() {
            "name" => fields.name = Some(value.to_owned()),
            "description" => {
                if value.chars().count() > MAX_DESCRIPTION_CHARS {
                    return Err(format!(
                        "its `description` is longer than {MAX_DESCRIPTION_CHARS} characters"
                    ));
                }
                fields.description = (!value.is_empty()).then(|| value.to_owned());
            }
            "argument-hint" => {
                if value.chars().count() > MAX_ARGUMENT_HINT_CHARS {
                    return Err(format!(
                        "its `argument-hint` is longer than {MAX_ARGUMENT_HINT_CHARS} characters"
                    ));
                }
                fields.argument_hint = (!value.is_empty()).then(|| value.to_owned());
            }
            _ => {}
        }
    }
    Err(format!(
        "its frontmatter is not closed with `---` within {MAX_FRONT_MATTER_LINES} lines"
    ))
}

/// `text` past a leading `line` and its terminator, if it starts with one.
fn strip_line<'a>(text: &'a str, line: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(line)?;
    rest.strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))
}

/// The next line without its terminator, and everything after it.
fn next_line(text: &str) -> Option<(&str, &str)> {
    if text.is_empty() {
        return None;
    }
    match text.find('\n') {
        Some(end) => Some((text[..end].trim_end_matches('\r'), &text[end + 1..])),
        None => Some((text, "")),
    }
}

#[cfg(test)]
mod tests;
