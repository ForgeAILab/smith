//! Effective manifests and confined component paths.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{PluginError, Result};

/// Metadata and component declarations after the Smith overlay is applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    /// Validated plugin name, using the caller's fallback when omitted.
    pub name: String,
    /// Optional author-provided version.
    pub version: Option<String>,
    /// Optional summary.
    pub description: Option<String>,
    /// Optional display label.
    pub display_name: Option<String>,
    /// Manifest default, after applying the Smith overlay.
    pub default_enabled: bool,
    /// Effective skill path declarations.
    pub skills: Option<Value>,
    /// Effective command path or inline declarations.
    pub commands: Option<Value>,
    /// Effective MCP path or inline declarations.
    pub mcp_servers: Option<Value>,
    /// Overlaid JSON, retained for recognised unsupported keys.
    pub(crate) effective: Map<String, Value>,
}

/// Validates a name usable both as a namespace and a store directory.
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '@' | ':' | '/' | '\\'))
    {
        return Err(PluginError::Invalid(format!(
            "invalid plugin or component name `{name}`"
        )));
    }
    Ok(())
}

/// Reads optional Claude and Smith manifests, replacing keys with overlay keys.
pub fn read_manifest(root: &Path, fallback_name: &str) -> Result<PluginManifest> {
    let root = root.canonicalize()?;
    let mut effective = read_object(&root, ".claude-plugin/plugin.json")?;
    effective.extend(read_object(&root, ".smith-plugin/plugin.json")?);
    let name = string(&effective, "name")?.unwrap_or_else(|| fallback_name.to_owned());
    validate_name(&name)?;
    let default_enabled = match effective.get("defaultEnabled") {
        None => true,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(PluginError::Invalid(
                "defaultEnabled must be a boolean".into(),
            ));
        }
    };
    Ok(PluginManifest {
        name,
        version: string(&effective, "version")?,
        description: string(&effective, "description")?,
        display_name: string(&effective, "displayName")?,
        default_enabled,
        skills: effective.get("skills").cloned(),
        commands: effective.get("commands").cloned(),
        mcp_servers: effective.get("mcpServers").cloned(),
        effective,
    })
}

fn string(object: &Map<String, Value>, key: &str) -> Result<Option<String>> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(PluginError::Invalid(format!("{key} must be a string"))),
    }
}

fn read_object(root: &Path, relative: &str) -> Result<Map<String, Value>> {
    let path = root.join(relative);
    if fs::symlink_metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        return Ok(Map::new());
    }
    let path = confined(root, &path)?;
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| PluginError::Invalid(format!("{relative} must be an object")))
}

pub(crate) fn confined(root: &Path, path: &Path) -> Result<PathBuf> {
    let resolved = path.canonicalize()?;
    if !resolved.starts_with(root) {
        return Err(PluginError::PathEscape(path.to_path_buf()));
    }
    Ok(resolved)
}

pub(crate) fn component_path(root: &Path, value: &str, allow_dot: bool) -> Result<PathBuf> {
    if !(value.starts_with("./") || allow_dot && value == ".") || value.contains('\\') {
        return Err(PluginError::Invalid(format!(
            "`{value}` must be ./-relative"
        )));
    }
    let mut depth = 0usize;
    for component in Path::new(value).components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return Err(PluginError::PathEscape(PathBuf::from(value))),
        }
    }
    let path = root.join(value);
    if !path.exists() {
        return Err(PluginError::Invalid(format!("`{value}` is missing")));
    }
    confined(root, &path)
}

pub(crate) fn relative(root: &Path, path: &Path) -> Result<String> {
    path.strip_prefix(root)
        .map_err(|_| PluginError::PathEscape(path.to_path_buf()))?
        .components()
        .map(|part| {
            part.as_os_str()
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| PluginError::Invalid("plugin paths must be UTF-8".into()))
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}
