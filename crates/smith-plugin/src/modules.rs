//! WASM inventory only: this crate never instantiates executable components.
use crate::{NotLoadedComponent, PluginError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Manifest capability requests, persisted with the confirmed bundle digest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleCapabilities {
    /// Exact DNS hosts (HTTPS) or explicit HTTP origins.
    #[serde(default)]
    pub network: Vec<String>,
    /// Exact destinations to which task data may be sent.
    #[serde(default)]
    pub data_egress: Vec<String>,
    /// Read confined workspace files.
    #[serde(default)]
    pub workspace_read: bool,
    /// Write confined workspace files.
    #[serde(default)]
    pub workspace_write: bool,
    /// Read plugin data files and key-value storage.
    #[serde(default)]
    pub plugin_data_read: bool,
    /// Write plugin data files and key-value storage.
    #[serde(default)]
    pub plugin_data_write: bool,
    /// Read recent history in the current session.
    #[serde(default)]
    pub session_history: bool,
}
impl ModuleCapabilities {
    /// Plain words for every requested capability, in deterministic order.
    pub fn labels(&self) -> Vec<String> {
        let mut labels = Vec::new();
        for (kind, values) in [
            ("network", &self.network),
            ("send task data to", &self.data_egress),
        ] {
            labels.extend(values.iter().map(|host| format!("{kind}: {host}")));
        }
        for (enabled, label) in [
            (self.workspace_read, "read files in the workspace"),
            (self.workspace_write, "write files in the workspace"),
            (
                self.plugin_data_read,
                "read plugin data and key-value storage",
            ),
            (
                self.plugin_data_write,
                "write plugin data and key-value storage",
            ),
            (self.session_history, "read recent session history"),
        ] {
            if enabled {
                labels.push(label.into());
            }
        }
        labels
    }
}
/// One valid declaration, with a stable local id derived from its relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmModuleComponent {
    /// Namespaced configuration id: wasm.<plugin hash>.<path hash>.
    pub id: String,
    /// Plugin-relative path, also the user configuration narrowing key.
    pub wasm: String,
    /// Confined canonical component path.
    pub path: PathBuf,
    /// Identity of the component bytes covered by the bundle digest.
    pub digest: smith_config::trust::ContentDigest,
    /// Requested authority; installation binds this to the bundle digest.
    pub capabilities: ModuleCapabilities,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Declaration {
    wasm: String,
    #[serde(default)]
    capabilities: ModuleCapabilities,
}

pub(crate) fn scan(
    root: &Path,
    manifest: &crate::PluginManifest,
    problems: &mut Vec<NotLoadedComponent>,
) -> Vec<WasmModuleComponent> {
    let Some(value) = &manifest.modules else {
        return Vec::new();
    };
    let Some(values) = value.as_array() else {
        problems.push(NotLoadedComponent {
            kind: "modules".into(),
            name: manifest.name.clone(),
            path: None,
            reason: "modules must be an array".into(),
        });
        return Vec::new();
    };
    let mut modules = Vec::new();
    for value in values {
        let wasm = value
            .get("wasm")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let parsed = (|| -> Result<WasmModuleComponent> {
            let mut declaration: Declaration = serde_json::from_value(value.clone())?;
            let path = crate::manifest::component_path(root, &declaration.wasm, false)?;
            if !path.is_file() || path.extension().is_none_or(|ext| ext != "wasm") {
                return Err(PluginError::Invalid("module must name a .wasm file".into()));
            }
            for endpoints in [
                &mut declaration.capabilities.network,
                &mut declaration.capabilities.data_egress,
            ] {
                endpoints.sort();
                endpoints.dedup();
                if endpoints.len() > 32 {
                    return Err(PluginError::Invalid(
                        "at most 32 endpoints per capability".into(),
                    ));
                }
                for endpoint in endpoints.iter() {
                    endpoint_parts(endpoint)?;
                }
            }
            let wasm = crate::manifest::relative(root, &path)?;
            let plugin = smith_config::trust::ContentDigest::of(manifest.name.as_bytes());
            let file = smith_config::trust::ContentDigest::of(wasm.as_bytes());
            let id = format!("wasm.{}.{}", &plugin.as_hex()[..16], &file.as_hex()[..16]);
            if modules.iter().any(|m: &WasmModuleComponent| m.id == id) {
                return Err(PluginError::Invalid("duplicate module path".into()));
            }
            Ok(WasmModuleComponent {
                id,
                wasm,
                digest: smith_config::trust::ContentDigest::of(&std::fs::read(&path)?),
                path,
                capabilities: declaration.capabilities,
            })
        })();
        match parsed {
            Ok(module) => modules.push(module),
            Err(error) => problems.push(NotLoadedComponent {
                kind: "modules".into(),
                name: format!("{}:{wasm}", manifest.name),
                path: Some(wasm.into()),
                reason: error
                    .to_string()
                    .replace(root.to_string_lossy().as_ref(), "."),
            }),
        }
    }
    modules.sort_by(|a, b| a.id.cmp(&b.id));
    modules
}
/// Validates an exact host or explicit HTTP origin; returns its host and HTTP flag.
/// HTTP ports remain part of the origin and must be checked by the sandbox.
pub fn endpoint_parts(endpoint: &str) -> Result<(String, bool)> {
    if endpoint.len() > 253 {
        return Err(PluginError::Invalid(
            "network endpoints must be at most 253 bytes".into(),
        ));
    }
    if endpoint.starts_with("http://") {
        let url = url::Url::parse(endpoint)
            .map_err(|_| PluginError::Invalid("invalid HTTP origin".into()))?;
        if url.origin().ascii_serialization() != endpoint
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(PluginError::Invalid(
                "network HTTP grants must be exact origins".into(),
            ));
        }
        return Ok((
            url.host_str()
                .ok_or_else(|| PluginError::Invalid("missing host".into()))?
                .into(),
            true,
        ));
    }
    if endpoint.is_empty()
        || endpoint.len() > 253
        || endpoint.starts_with('.')
        || endpoint.ends_with('.')
        || !endpoint
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-".contains(&b))
    {
        return Err(PluginError::Invalid(
            "network grants must name exact lowercase hosts or HTTP origins".into(),
        ));
    }
    Ok((endpoint.into(), false))
}
