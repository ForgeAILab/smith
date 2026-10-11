//! Reviewed user-layer switches, with exact-byte rollback after publication.

use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use smith_config::model::{ConfigFile, ImageGenerationSection, ModuleSection, ToolsSection};
use smith_config::resolve::{Layer, ResolvedModule};
use smith_config::user_config::{PreparedConfigEdit, prepare_user_config_edit};
use smith_module::ModuleReport;

pub(crate) struct PreparedModuleSwitch {
    pub(crate) edit: PreparedConfigEdit,
    pub(crate) fingerprint: String,
}

pub(crate) fn prepare_switch(
    user_dir: &Path,
    reports: &[ModuleReport],
    id: &str,
    enabled: bool,
) -> Result<PreparedModuleSwitch> {
    let Some(report) = reports.iter().find(|report| report.descriptor.id == id) else {
        let mut ids = reports
            .iter()
            .map(|report| report.descriptor.id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        anyhow::bail!("unknown module `{id}`; known ids: {}", ids.join(", "));
    };
    if enabled && !report.descriptor.compiled_in {
        anyhow::bail!("module `{id}` is not in this build");
    }
    let path = user_dir.join("config.toml");
    let prior = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).context("reading the user configuration"),
    };
    let existing = ConfigFile::parse(&prior).map_err(|_| {
        smith_config::user_config::UserConfigEditError::InvalidExistingConfig { path }
    })?;
    let legacy = id == "image-generation"
        && existing
            .tools
            .as_ref()
            .and_then(|tools| tools.image_generation.as_ref())
            .and_then(|image| image.enabled)
            .is_some();
    let mut patch = ConfigFile::default();
    if legacy {
        patch.tools = Some(ToolsSection {
            image_generation: Some(ImageGenerationSection {
                enabled: Some(enabled),
                ..ImageGenerationSection::default()
            }),
        });
    }
    // When both spellings already exist, keep them aligned in this layer.
    if !legacy
        || existing
            .modules
            .get(id)
            .is_some_and(|module| module.enabled.is_some())
    {
        patch.modules.insert(
            id.into(),
            ModuleSection {
                enabled: Some(enabled),
            },
        );
    }
    let edit = prepare_user_config_edit(user_dir, &patch)?;
    let mut digest = Sha256::new();
    digest.update(prior.as_bytes());
    digest.update(edit.preview().as_bytes());
    Ok(PreparedModuleSwitch {
        edit,
        fingerprint: format!("{:x}", digest.finalize()),
    })
}

pub(crate) fn switch_outcome(id: &str, enabled: bool, resolved: &ResolvedModule) -> String {
    if resolved.enabled.source.layer > Layer::UserFile {
        format!(
            "saved `{id}` {} in the user file; {} overrides it with {} = {} ({})",
            if enabled { "on" } else { "off" },
            resolved.enabled.source.layer.label(),
            resolved.enabled.source.key,
            resolved.enabled.value,
            resolved.enabled.source,
        )
    } else {
        format!(
            "module `{id}` {} · applied at the safe boundary",
            if enabled { "on" } else { "off" }
        )
    }
}

pub(crate) fn commit_switch(
    user_dir: &Path,
    reports: &[ModuleReport],
    request: &smith_client::commands::ModuleSwitchRequest,
) -> Result<smith_config::user_config::CommittedConfigEdit> {
    let prepared = prepare_switch(user_dir, reports, &request.id, request.enabled)?;
    anyhow::ensure!(
        prepared.fingerprint == request.fingerprint,
        "the user configuration changed since the preview; run /modules {} {} again",
        request.id,
        if request.enabled { "on" } else { "off" }
    );
    Ok(prepared.edit.commit(true)?)
}
