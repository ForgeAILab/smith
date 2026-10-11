//! Reviewed user-layer switches, with exact-byte rollback after publication.

use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use smith_client::plugins_report::PluginsReport;
use smith_config::model::{ConfigFile, PluginSection};
use smith_config::resolve::{Layer, ResolvedPlugin};
use smith_config::user_config::{PreparedConfigEdit, prepare_user_config_edit};

pub(crate) struct PreparedPluginSwitch {
    pub(crate) edit: PreparedConfigEdit,
    pub(crate) fingerprint: String,
}

pub(crate) fn prepare_switch(
    user_dir: &Path,
    reports: &PluginsReport,
    id: &str,
    enabled: bool,
) -> Result<PreparedPluginSwitch> {
    if !reports.rows.iter().any(|row| row.name == id) {
        let names = reports
            .rows
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>();
        anyhow::bail!(
            "unknown plugin `{id}`; installed names: {}",
            names.join(", ")
        );
    }
    let prior = match std::fs::read(user_dir.join("config.toml")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error).context("reading the user configuration"),
    };
    let patch = ConfigFile {
        plugins: [(
            id.into(),
            PluginSection {
                modules: Default::default(),
                enabled: Some(enabled),
            },
        )]
        .into(),
        ..ConfigFile::default()
    };
    let edit = prepare_user_config_edit(user_dir, &patch)?;
    let mut digest = Sha256::new();
    digest.update(&prior);
    digest.update(edit.preview().as_bytes());
    Ok(PreparedPluginSwitch {
        edit,
        fingerprint: format!("{:x}", digest.finalize()),
    })
}

pub(crate) fn switch_outcome(id: &str, enabled: bool, resolved: &ResolvedPlugin) -> String {
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
            "plugin `{id}` {} · applied at the safe boundary",
            if enabled { "on" } else { "off" }
        )
    }
}

pub(crate) fn commit_switch(
    user_dir: &Path,
    reports: &PluginsReport,
    request: &smith_client::commands::PluginSwitchRequest,
) -> Result<smith_config::user_config::CommittedConfigEdit> {
    let prepared = prepare_switch(user_dir, reports, &request.name, request.enabled)?;
    anyhow::ensure!(
        prepared.fingerprint == request.fingerprint,
        "the user configuration changed since the preview; run /plugins {} {} again",
        request.name,
        if request.enabled { "on" } else { "off" }
    );
    Ok(prepared.edit.commit(true)?)
}
