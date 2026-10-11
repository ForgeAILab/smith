//! Marketplace catalogues, preserving unsupported source kinds for display.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::{component_path, confined};
use crate::{PluginError, PluginSource, Result, validate_name};

/// A parsed Claude Code marketplace catalogue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marketplace {
    /// Catalogue name.
    pub name: String,
    /// Entries available for installation.
    pub plugins: Vec<MarketplacePlugin>,
}

/// One marketplace entry; source types are validated when resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplacePlugin {
    /// Plugin name and fallback name when the bundle has no manifest.
    pub name: String,
    /// Claude Code source declaration.
    pub source: Value,
    /// Optional catalogue version.
    #[serde(default)]
    pub version: Option<String>,
    /// Optional summary.
    #[serde(default)]
    pub description: Option<String>,
}

/// Persistent metadata for a registered marketplace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketplaceRecord {
    /// Validated catalogue name.
    pub name: String,
    /// Source refreshed by marketplace update.
    pub source: PluginSource,
    /// Git commit from the most recent refresh.
    pub resolved_commit: Option<String>,
    /// Stored copy of the catalogue directory.
    pub directory: std::path::PathBuf,
}

impl Marketplace {
    /// Parses the catalogue in a confined .claude-plugin directory.
    pub fn read(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        let path = confined(&root, &root.join(".claude-plugin/marketplace.json"))?;
        let catalogue: Self = serde_json::from_slice(&fs::read(path)?)?;
        validate_name(&catalogue.name)?;
        let mut names = std::collections::BTreeSet::new();
        for plugin in &catalogue.plugins {
            validate_name(&plugin.name)?;
            if !names.insert(&plugin.name) {
                return Err(PluginError::Invalid(format!(
                    "duplicate marketplace entry `{}`",
                    plugin.name
                )));
            }
        }
        Ok(catalogue)
    }

    /// Resolves one entry, keeping relative paths inside the catalogue copy.
    pub fn resolve(&self, root: &Path, name: &str) -> Result<PluginSource> {
        let plugin = self
            .plugins
            .iter()
            .find(|plugin| plugin.name == name)
            .ok_or_else(|| {
                PluginError::Invalid(format!(
                    "marketplace `{}` has no plugin `{name}`",
                    self.name
                ))
            })?;
        plugin.resolve(root)
    }
}

impl MarketplacePlugin {
    /// Resolves local, GitHub, URL, and git-subdir declarations.
    pub fn resolve(&self, root: &Path) -> Result<PluginSource> {
        let root = root.canonicalize()?;
        if let Some(path) = self.source.as_str() {
            let path = component_path(
                &root,
                &format!("./{}", path.strip_prefix("./").unwrap_or(path)),
                false,
            )?;
            return Ok(PluginSource::Local { path });
        }
        let map = self.source.as_object().ok_or_else(|| {
            PluginError::Invalid("marketplace source must be a string or object".into())
        })?;
        let kind = map
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let field = |key: &str| {
            map.get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| PluginError::Invalid(format!("{kind} source needs `{key}`")))
        };
        let reference = map.get("ref").and_then(Value::as_str).map(str::to_owned);
        match kind {
            "github" => {
                let mut source = PluginSource::parse(&format!("github:{}", field("repo")?))?;
                if let PluginSource::Git {
                    reference: target, ..
                } = &mut source
                {
                    *target = reference;
                }
                Ok(source)
            }
            "url" | "git-subdir" => Ok(PluginSource::Git {
                url: field("url")?.into(),
                reference,
                subdirectory: if kind == "git-subdir" {
                    Some(field("path")?.into())
                } else {
                    None
                },
            }),
            other => Err(PluginError::UnsupportedSource { kind: other.into() }),
        }
    }
}
