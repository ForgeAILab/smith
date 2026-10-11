//! Plain MCP declarations and single-pass path variable expansion.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Three caller-supplied paths available to plugin content.
#[derive(Debug, Clone, Copy)]
pub struct PathVariables<'a> {
    /// Installed plugin directory.
    pub plugin_root: &'a Path,
    /// Persistent plugin data directory.
    pub plugin_data: &'a Path,
    /// Active workspace directory.
    pub project_dir: &'a Path,
}

/// Expands only the three plugin path variables; replacement text is not scanned.
pub fn substitute_variables(text: &str, paths: PathVariables<'_>) -> String {
    let mut output = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let token = &rest[start..];
        let Some(end) = token.find('}') else {
            output.push_str(token);
            return output;
        };
        let replacement = match &token[2..end] {
            "CLAUDE_PLUGIN_ROOT" => Some(paths.plugin_root),
            "CLAUDE_PLUGIN_DATA" => Some(paths.plugin_data),
            "CLAUDE_PROJECT_DIR" => Some(paths.project_dir),
            _ => None,
        };
        match replacement {
            Some(path) => output.push_str(&path.to_string_lossy()),
            None => output.push_str(&token[..=end]),
        }
        rest = &token[end + 1..];
    }
    output.push_str(rest);
    output
}

/// Whether content refers to an option Smith cannot yet obtain for a plugin.
pub fn needs_configuration(text: &str) -> bool {
    text.match_indices("${user_config.")
        .any(|(start, _)| text[start..].contains('}'))
}

/// A local stdio or remote streamable HTTP server, before or after expansion.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpDeclaration {
    /// Namespaced server name.
    #[serde(default)]
    pub name: String,
    /// Omitted or stdio for a process, http for streamable HTTP.
    #[serde(default, rename = "type")]
    pub transport: Option<String>,
    /// Executable for a stdio server.
    #[serde(default)]
    pub command: Option<String>,
    /// Ordered process arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Process environment declarations.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Streamable HTTP endpoint.
    #[serde(default)]
    pub url: Option<String>,
    /// Remote request headers.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

impl McpDeclaration {
    /// Expands all supported string fields and supplies stdio plugin environment.
    pub fn resolve(&self, paths: PathVariables<'_>) -> Self {
        let expand = |value: &str| substitute_variables(value, paths);
        let mut resolved = self.clone();
        resolved.command = self.command.as_deref().map(expand);
        resolved.url = self.url.as_deref().map(expand);
        resolved.args = self.args.iter().map(|arg| expand(arg)).collect();
        resolved.env = self
            .env
            .iter()
            .map(|(k, v)| (k.clone(), expand(v)))
            .collect();
        resolved.headers = self
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), expand(v)))
            .collect();
        if resolved.command.is_some() {
            resolved.env.insert(
                "CLAUDE_PLUGIN_ROOT".into(),
                paths.plugin_root.to_string_lossy().into_owned(),
            );
            resolved.env.insert(
                "CLAUDE_PLUGIN_DATA".into(),
                paths.plugin_data.to_string_lossy().into_owned(),
            );
        }
        resolved
    }

    /// The command and ordered arguments, or remote endpoint, for an inventory.
    pub fn command_line(&self) -> String {
        match &self.command {
            Some(command) => std::iter::once(command.as_str())
                .chain(self.args.iter().map(String::as_str))
                .map(|s| format!("{s:?}"))
                .collect::<Vec<_>>()
                .join(" "),
            None => self.url.clone().unwrap_or_default(),
        }
    }
}
