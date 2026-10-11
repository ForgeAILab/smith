//! Layout inspection with per-component failures and stable content framing.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use smith_config::trust::ContentDigest;

use crate::manifest::{component_path, confined, relative};
use crate::{
    McpDeclaration, PathVariables, PluginManifest, Result, needs_configuration, read_manifest,
    validate_name,
};

/// A skill body that can enter the existing skill resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillComponent {
    /// Namespaced skill name.
    pub name: String,
    /// Confined absolute body path.
    pub path: PathBuf,
    /// Cheap frontmatter description, when present.
    pub description: Option<String>,
    /// Identity of the skill file bytes.
    pub digest: ContentDigest,
}

/// Where a command body comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandSource {
    /// Confined file for the existing file command loader.
    File(PathBuf),
    /// Manifest-authored prompt text.
    Inline(String),
}

/// A command declaration for the existing command resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandComponent {
    /// Namespaced command name.
    pub name: String,
    /// File or inline body source.
    pub source: CommandSource,
    /// Optional completion description.
    pub description: Option<String>,
    /// Optional completion argument syntax.
    pub argument_hint: Option<String>,
    /// Identity of the complete body bytes.
    pub digest: ContentDigest,
}

/// A recognised component which Smith will not load.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NotLoadedComponent {
    /// Stable component category, such as hooks or mcpServers.
    pub kind: String,
    /// Namespaced component name, or a manifest path declaration.
    pub name: String,
    /// Portable plugin-relative source path when one is known.
    pub path: Option<String>,
    /// Explanation suitable for an inventory surface.
    pub reason: String,
}

/// Everything an install confirmation should display, with no trust decision.
#[derive(Debug, Clone)]
pub struct PluginInventory {
    /// Canonical directory inspected.
    pub root: PathBuf,
    /// Effective manifest and metadata.
    pub manifest: PluginManifest,
    /// Loaded skills, sorted by namespaced name.
    pub skills: Vec<SkillComponent>,
    /// Loaded commands, sorted by namespaced name.
    pub commands: Vec<CommandComponent>,
    /// Merged MCP declarations before path substitution, sorted by name.
    pub mcp_servers: Vec<McpDeclaration>,
    /// Recognised unsupported or failed components, sorted and deduplicated.
    pub not_loaded: Vec<NotLoadedComponent>,
    /// Content identity, independent of the installation directory.
    pub digest: ContentDigest,
}

impl PluginInventory {
    /// Returns expanded declarations for display and later config conversion.
    pub fn resolved_mcp(&self, paths: PathVariables<'_>) -> Vec<McpDeclaration> {
        self.mcp_servers
            .iter()
            .map(|server| server.resolve(paths))
            .collect()
    }
}

/// Inspects an unchanged Claude Code layout; component failures stay in inventory.
pub fn inspect(root: &Path, fallback_name: &str) -> Result<PluginInventory> {
    let root = root.canonicalize()?;
    let manifest = read_manifest(&root, fallback_name)?;
    let mut scanner = Scanner {
        root: &root,
        manifest: &manifest,
        skills: BTreeMap::new(),
        commands: BTreeMap::new(),
        mcp: BTreeMap::new(),
        problems: Vec::new(),
    };
    scanner.scan_skills();
    scanner.scan_commands();
    scanner.scan_mcp();
    scanner.scan_unsupported();
    scanner.problems.sort();
    scanner.problems.dedup();
    let skills: Vec<_> = scanner.skills.into_values().collect();
    let commands: Vec<_> = scanner.commands.into_values().collect();
    let mcp_servers: Vec<_> = scanner.mcp.into_values().collect();
    let not_loaded = scanner.problems;
    let digest = crate::digest::directory_digest(&root, &manifest.name)?;
    Ok(PluginInventory {
        root,
        manifest,
        skills,
        commands,
        mcp_servers,
        not_loaded,
        digest,
    })
}

struct Scanner<'a> {
    root: &'a Path,
    manifest: &'a PluginManifest,
    skills: BTreeMap<String, SkillComponent>,
    commands: BTreeMap<String, CommandComponent>,
    mcp: BTreeMap<String, McpDeclaration>,
    problems: Vec<NotLoadedComponent>,
}

impl Scanner<'_> {
    fn name(&self, name: &str) -> String {
        format!("{}:{name}", self.manifest.name)
    }

    fn problem(&mut self, kind: &str, name: &str, path: Option<String>, reason: impl ToString) {
        self.problems.push(NotLoadedComponent {
            kind: kind.into(),
            name: self.name(name),
            path,
            reason: reason
                .to_string()
                .replace(self.root.to_string_lossy().as_ref(), "."),
        });
    }

    fn declared_paths(&mut self, kind: &str, value: &Value, dot: bool) -> Vec<PathBuf> {
        let values = match value {
            Value::Array(values) => values.iter().collect::<Vec<_>>(),
            other => vec![other],
        };
        let mut paths = Vec::new();
        for value in values {
            let Some(path) = value.as_str() else {
                self.problem(kind, kind, None, "expected a path or an array of paths");
                continue;
            };
            if needs_configuration(path) {
                self.problem(kind, path, Some(path.into()), "needs configuration");
                continue;
            }
            match component_path(self.root, path, dot) {
                Ok(path) => paths.push(path),
                Err(error) => self.problem(kind, path, Some(path.into()), error),
            }
        }
        paths
    }

    fn scan_skills(&mut self) {
        let default = self.root.join("skills");
        if default.exists() {
            self.skills_at(&default);
        }
        if let Some(value) = &self.manifest.skills {
            for path in self.declared_paths("skills", value, true) {
                self.skills_at(&path);
            }
        } else if !default.exists() && self.root.join("SKILL.md").exists() {
            self.skill(&self.manifest.name.clone(), &self.root.join("SKILL.md"));
        }
    }

    fn skills_at(&mut self, directory: &Path) {
        if directory.is_file() {
            let name = directory
                .parent()
                .and_then(Path::file_name)
                .unwrap_or_default()
                .to_string_lossy();
            self.skill(&name, directory);
            return;
        }
        if directory.join("SKILL.md").exists() {
            let name = if directory == self.root {
                self.manifest.name.clone()
            } else {
                directory
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            };
            self.skill(&name, &directory.join("SKILL.md"));
        }
        match entries(self.root, directory) {
            Ok(paths) => {
                for path in paths {
                    if path.is_dir() && path.join("SKILL.md").exists() {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        self.skill(&name, &path.join("SKILL.md"));
                    }
                }
            }
            Err(error) => self.problem(
                "skills",
                "skills",
                relative(self.root, directory).ok(),
                error,
            ),
        }
    }

    fn body(&self, name: &str, path: &Path) -> Result<(PathBuf, Vec<u8>, String)> {
        validate_name(name)?;
        let resolved = confined(self.root, path)?;
        relative(self.root, &resolved)?;
        let bytes = fs::read(&resolved)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| crate::PluginError::Invalid("body is not UTF-8".into()))?
            .to_owned();
        if needs_configuration(&text) {
            return Err(crate::PluginError::Invalid("needs configuration".into()));
        }
        Ok((resolved, bytes, text))
    }

    fn skill(&mut self, name: &str, path: &Path) {
        match self.body(name, path) {
            Ok((resolved, bytes, text)) => {
                let name = self.name(name);
                self.skills.insert(
                    name.clone(),
                    SkillComponent {
                        name,
                        path: resolved,
                        description: frontmatter(&text, "description"),
                        digest: ContentDigest::of(&bytes),
                    },
                );
            }
            Err(error) => self.problem("skills", name, relative(self.root, path).ok(), error),
        }
    }

    fn scan_commands(&mut self) {
        match &self.manifest.commands {
            None => {
                let directory = self.root.join("commands");
                if directory.exists() {
                    self.commands_at(&directory);
                }
            }
            Some(Value::Object(map)) => {
                for (name, declaration) in map {
                    let Some(map) = declaration.as_object() else {
                        self.problem(
                            "commands",
                            name,
                            None,
                            "expected source or content declaration",
                        );
                        continue;
                    };
                    if needs_configuration(&declaration.to_string()) {
                        self.problem(
                            "commands",
                            name,
                            map.get("source").and_then(Value::as_str).map(str::to_owned),
                            "needs configuration",
                        );
                        continue;
                    }
                    let description = map
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let hint = map
                        .get("argumentHint")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    if let Some(source) = map.get("source").and_then(Value::as_str) {
                        if needs_configuration(source) {
                            self.problem(
                                "commands",
                                name,
                                Some(source.into()),
                                "needs configuration",
                            );
                            continue;
                        }
                        match component_path(self.root, source, false) {
                            Ok(path) => self.command(name, &path, description, hint),
                            Err(error) => {
                                self.problem("commands", name, Some(source.into()), error)
                            }
                        }
                    } else if let Some(content) = map.get("content").and_then(Value::as_str) {
                        if needs_configuration(content) {
                            self.problem("commands", name, None, "needs configuration");
                        } else if let Err(error) = validate_name(name) {
                            self.problem("commands", name, None, error);
                        } else {
                            let name = self.name(name);
                            self.commands.insert(
                                name.clone(),
                                CommandComponent {
                                    name,
                                    source: CommandSource::Inline(content.into()),
                                    description,
                                    argument_hint: hint,
                                    digest: ContentDigest::of(content.as_bytes()),
                                },
                            );
                        }
                    } else {
                        self.problem(
                            "commands",
                            name,
                            None,
                            "expected source or content declaration",
                        );
                    }
                }
            }
            Some(value) => {
                for path in self.declared_paths("commands", value, false) {
                    self.commands_at(&path);
                }
            }
        }
    }

    fn commands_at(&mut self, path: &Path) {
        if path.is_file() {
            let name = path.file_stem().unwrap_or_default().to_string_lossy();
            self.command(&name, path, None, None);
        } else {
            match entries(self.root, path) {
                Ok(paths) => {
                    for path in paths {
                        if path.extension().is_some_and(|extension| extension == "md") {
                            let name = path.file_stem().unwrap_or_default().to_string_lossy();
                            self.command(&name, &path, None, None);
                        }
                    }
                }
                Err(error) => self.problem(
                    "commands",
                    "commands",
                    relative(self.root, path).ok(),
                    error,
                ),
            }
        }
    }

    fn command(
        &mut self,
        name: &str,
        path: &Path,
        description: Option<String>,
        hint: Option<String>,
    ) {
        match self.body(name, path) {
            Ok((resolved, bytes, text)) => {
                let name = self.name(name);
                self.commands.insert(
                    name.clone(),
                    CommandComponent {
                        name,
                        source: CommandSource::File(resolved),
                        description: description.or_else(|| frontmatter(&text, "description")),
                        argument_hint: hint.or_else(|| frontmatter(&text, "argument-hint")),
                        digest: ContentDigest::of(&bytes),
                    },
                );
            }
            Err(error) => self.problem("commands", name, relative(self.root, path).ok(), error),
        }
    }

    fn scan_mcp(&mut self) {
        if self.root.join(".mcp.json").exists() {
            self.mcp_file(&self.root.join(".mcp.json"));
        }
        if let Some(value) = &self.manifest.mcp_servers {
            self.mcp_value(value);
        }
    }

    fn mcp_value(&mut self, value: &Value) {
        match value {
            Value::Array(values) => {
                for value in values {
                    self.mcp_value(value);
                }
            }
            Value::String(path) => {
                if needs_configuration(path) {
                    self.problem("mcpServers", path, Some(path.into()), "needs configuration");
                } else if path.ends_with(".mcpb") || path.ends_with(".dxt") {
                    self.problem(
                        "mcpServers",
                        path,
                        Some(path.into()),
                        "MCP bundles are not supported",
                    );
                } else {
                    match component_path(self.root, path, false) {
                        Ok(path) => self.mcp_file(&path),
                        Err(error) => self.problem("mcpServers", path, Some(path.into()), error),
                    }
                }
            }
            Value::Object(map) => {
                if let Some(servers) = map.get("mcpServers") {
                    self.mcp_value(servers);
                } else {
                    for (name, value) in map {
                        self.mcp_server(name, value);
                    }
                }
            }
            _ => self.problem(
                "mcpServers",
                "mcpServers",
                None,
                "expected a JSON path or server map",
            ),
        }
    }

    fn mcp_file(&mut self, path: &Path) {
        let value = confined(self.root, path)
            .and_then(|path| Ok(serde_json::from_slice::<Value>(&fs::read(path)?)?));
        match value {
            Ok(value) => {
                let servers = value.get("mcpServers").unwrap_or(&value);
                if let Some(map) = servers.as_object() {
                    for (name, declaration) in map {
                        self.mcp_server(name, declaration);
                    }
                } else {
                    self.problem(
                        "mcpServers",
                        "mcpServers",
                        relative(self.root, path).ok(),
                        "MCP declaration file must contain a server map",
                    );
                }
            }
            Err(error) => self.problem(
                "mcpServers",
                "mcpServers",
                relative(self.root, path).ok(),
                error,
            ),
        }
    }

    fn mcp_server(&mut self, name: &str, value: &Value) {
        let full_name = self.name(name);
        // A later declaration also wins when it is unusable: an older runnable
        // declaration must not hide an override that needs configuration.
        self.mcp.remove(&full_name);
        self.problems
            .retain(|item| !(item.kind == "mcpServers" && item.name == full_name));
        if needs_configuration(&value.to_string()) {
            self.problem("mcpServers", name, None, "needs configuration");
            return;
        }
        if let Some(path) = value.as_str()
            && (path.ends_with(".mcpb") || path.ends_with(".dxt"))
        {
            self.problem(
                "mcpServers",
                name,
                Some(path.into()),
                "MCP bundles are not supported",
            );
            return;
        }
        let result = validate_name(name)
            .and_then(|()| Ok(serde_json::from_value::<McpDeclaration>(value.clone())?));
        match result {
            Ok(mut declaration) => {
                let transport =
                    declaration
                        .transport
                        .as_deref()
                        .unwrap_or(if declaration.url.is_some() {
                            "http"
                        } else {
                            "stdio"
                        });
                let reason = match transport {
                    "stdio"
                        if declaration
                            .command
                            .as_ref()
                            .is_some_and(|s| !s.trim().is_empty())
                            && declaration.url.is_none() =>
                    {
                        None
                    }
                    "http"
                        if declaration
                            .url
                            .as_ref()
                            .is_some_and(|s| !s.trim().is_empty())
                            && declaration.command.is_none() =>
                    {
                        None
                    }
                    "sse" | "ws" | "websocket" => {
                        Some(format!("{transport} transport is not supported"))
                    }
                    "http" | "stdio" => Some("expected command/args/env or url/headers".into()),
                    _ => Some(format!("{transport} transport is not supported")),
                };
                if let Some(reason) = reason {
                    self.problem("mcpServers", name, None, reason);
                } else {
                    declaration.name = full_name.clone();
                    self.mcp.insert(full_name, declaration);
                }
            }
            Err(error) => self.problem("mcpServers", name, None, error),
        }
    }

    fn scan_unsupported(&mut self) {
        for (key, locations) in [
            ("hooks", &["hooks/hooks.json"][..]),
            ("agents", &["agents"][..]),
            ("lspServers", &[".lsp.json"][..]),
            ("outputStyles", &["output-styles", "outputStyles"][..]),
            ("workflows", &["workflows"][..]),
            ("themes", &["themes"][..]),
            ("monitors", &["monitors"][..]),
            ("channels", &["channels"][..]),
            ("bin", &["bin"][..]),
            ("settings", &["settings.json"][..]),
            ("userConfig", &[][..]),
        ] {
            let mut present = BTreeSet::new();
            for location in locations {
                if fs::symlink_metadata(self.root.join(location)).is_ok() {
                    present.insert((*location).to_owned());
                }
            }
            let declared = self.manifest.effective.contains_key(key);
            let configured = self.manifest.effective.get(key).is_some_and(|value| {
                needs_configuration(&value.to_string())
                    || referenced_configuration(self.root, value)
            });
            let reason = if key == "userConfig" || configured {
                "needs configuration"
            } else if key == "hooks" {
                "hooks are not run"
            } else {
                "component is not supported"
            };
            if present.is_empty() && declared {
                self.problem(key, key, None, reason);
            }
            for path in present {
                let reason = if contains_configuration(
                    self.root,
                    &self.root.join(&path),
                    &mut BTreeSet::new(),
                ) {
                    "needs configuration"
                } else {
                    reason
                };
                self.problem(key, key, Some(path), reason);
            }
        }
    }
}

fn referenced_configuration(root: &Path, value: &Value) -> bool {
    match value {
        Value::String(path) if path.starts_with("./") => component_path(root, path, false)
            .is_ok_and(|path| contains_configuration(root, &path, &mut BTreeSet::new())),
        Value::Array(values) => values
            .iter()
            .any(|value| referenced_configuration(root, value)),
        Value::Object(values) => values
            .values()
            .any(|value| referenced_configuration(root, value)),
        _ => false,
    }
}

fn contains_configuration(root: &Path, path: &Path, visited: &mut BTreeSet<PathBuf>) -> bool {
    use std::io::Read;

    let Ok(path) = confined(root, path) else {
        return false;
    };
    if !visited.insert(path.clone()) {
        return false;
    }
    if path.is_dir() {
        return entries(root, &path).is_ok_and(|paths| {
            paths
                .iter()
                .any(|path| contains_configuration(root, path, visited))
        });
    }
    if !path.is_file() {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let prefix = b"${user_config.";
    let mut matched = 0;
    let mut reference_started = false;
    let mut buffer = [0; 8192];
    loop {
        let Ok(count) = file.read(&mut buffer) else {
            return false;
        };
        if count == 0 {
            return false;
        }
        for byte in &buffer[..count] {
            if reference_started && *byte == b'}' {
                return true;
            }
            if *byte == prefix[matched] {
                matched += 1;
                if matched == prefix.len() {
                    reference_started = true;
                    matched = 0;
                }
            } else {
                matched = usize::from(*byte == prefix[0]);
            }
        }
    }
}

fn entries(root: &Path, directory: &Path) -> Result<Vec<PathBuf>> {
    confined(root, directory)?;
    let mut entries = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    Ok(entries)
}

fn frontmatter(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return None;
    }
    for line in lines.take(64) {
        if line == "---" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim() == key
        {
            return Some(value.trim().trim_matches(['\'', '"']).to_owned());
        }
    }
    None
}
