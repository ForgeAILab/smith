use super::*;

pub(super) fn reject_project_granted_authority(
    config: &ResolvedConfig,
    project_root: &Path,
) -> Result<(), HostSessionError> {
    if config.approval.mode.value == ApprovalMode::AllowAll
        && controlled_by_project(&config.approval.mode.source, project_root)
    {
        return Err(HostSessionError::ProjectGrantedAuthority {
            setting: "approval.mode",
            provenance: config.approval.mode.source.clone(),
        });
    }
    if let Some(auto_approve) = &config.approval.auto_approve
        && !auto_approve.value.is_empty()
        && controlled_by_project(&auto_approve.source, project_root)
    {
        return Err(HostSessionError::ProjectGrantedAuthority {
            setting: "approval.auto_approve",
            provenance: auto_approve.source.clone(),
        });
    }
    if let Some(rule) = config
        .approval
        .auto
        .iter()
        .find(|rule| controlled_by_project(&rule.source, project_root))
    {
        return Err(HostSessionError::ProjectGrantedAuthority {
            setting: "approval.auto",
            provenance: rule.source.clone(),
        });
    }
    Ok(())
}

pub(super) fn reject_project_controlled_persistence(
    config: &ResolvedConfig,
    project_root: &Path,
) -> Result<(), HostSessionError> {
    for (setting, source) in [
        ("persistence.enabled", &config.persistence.enabled.source),
        (
            "persistence.sessions_dir",
            &config.persistence.sessions_dir.source,
        ),
        (
            "persistence.journal_events",
            &config.persistence.journal_events.source,
        ),
    ] {
        if controlled_by_project(source, project_root) {
            return Err(HostSessionError::ProjectControlledPersistence {
                setting,
                provenance: source.clone(),
            });
        }
    }
    for (setting, value) in [
        (
            "persistence.checkpoint_key",
            config
                .persistence
                .checkpoint_key
                .as_ref()
                .map(|key| &key.source),
        ),
        (
            "persistence.checkpoint_key_credential",
            config
                .persistence
                .checkpoint_key_credential
                .as_ref()
                .map(|credential| &credential.source),
        ),
    ] {
        if let Some(source) = value
            && controlled_by_project(source, project_root)
        {
            return Err(HostSessionError::ProjectControlledPersistence {
                setting,
                provenance: source.clone(),
            });
        }
    }
    Ok(())
}

fn controlled_by_project(source: &Source, project_root: &Path) -> bool {
    let project_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let project_config = project_root.join(".smith");
    matches!(source.layer, Layer::ProjectFile | Layer::ProjectLocalFile)
        || source.file.as_ref().is_some_and(|file| {
            file.starts_with(&project_config)
                || file
                    .canonicalize()
                    .unwrap_or_else(|_| file.clone())
                    .starts_with(&project_config)
        })
}

/// Lists saved sessions for `project_root`, newest first.
pub async fn list(
    config: &ResolvedConfig,
    project_root: impl AsRef<Path>,
) -> Result<Vec<SessionListing>, HostSessionError> {
    reject_project_controlled_persistence(config, project_root.as_ref())?;
    if !config.persistence.enabled.value {
        return Ok(Vec::new());
    }
    Ok(FileSessionStore::new(paths(config, project_root.as_ref())?)
        .list()
        .await?)
}

/// Validates host-owned authority and persistence boundaries without creating
/// a runtime or session.
pub fn validate_host_policy(
    config: &ResolvedConfig,
    project_root: impl AsRef<Path>,
) -> Result<(), HostSessionError> {
    reject_project_granted_authority(config, project_root.as_ref())?;
    reject_project_controlled_persistence(config, project_root.as_ref())
}

/// Resolves the configured session directory for one canonical project.
pub fn paths(
    config: &ResolvedConfig,
    project_root: impl AsRef<Path>,
) -> Result<SessionPaths, HostSessionError> {
    reject_project_controlled_persistence(config, project_root.as_ref())?;
    let project = project_id(project_root)?;
    Ok(SessionPaths::from_sessions_dir(
        &config.persistence.sessions_dir.value,
        &project,
    ))
}

/// Derives a stable, path-safe project identity from its canonical path.
pub fn project_id(project_root: impl AsRef<Path>) -> Result<ProjectId, RuntimeError> {
    let canonical = project_root.as_ref().canonicalize().map_err(|error| {
        RuntimeError::new(
            ErrorKind::Config,
            format!(
                "cannot resolve project root `{}`: {error}",
                project_root.as_ref().display()
            ),
        )
    })?;
    let fingerprint = agent_runtime::registry::Fingerprint::of(path_bytes(&canonical));
    ProjectId::new(fingerprint.as_str())
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> &[u8] {
    // Smith's first supported hosts are macOS and Linux, where the branch
    // above hashes the exact OS bytes. This fallback keeps other targets
    // buildable until their native path encoding gets a release contract.
    path.to_str().unwrap_or_default().as_bytes()
}

/// Mints an explicit identity so persistence observers can be attached before
/// Agent Runtime emits `SessionStarted`.
pub fn mint_session_id() -> SessionId {
    SessionId::new(format!("session-{}", uuid::Uuid::new_v4()))
}
