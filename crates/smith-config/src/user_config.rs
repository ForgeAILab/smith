//! Reviewed, comment-preserving edits to `~/.smith/config.toml`.
//!
//! Setup prepares the complete candidate in memory, exposes only a
//! secret-safe leaf-level preview, and publishes through a same-directory
//! temporary file plus rename. A returned commit handle retains the exact
//! prior bytes so runtime preflight can roll the edit back without trying to
//! reverse-merge TOML.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use toml_edit::{DocumentMut, Item, Table};

use crate::credential::CredentialRef;
use crate::model::ConfigFile;
use crate::resolve::CONFIG_FILE;

/// One non-secret leaf changed by a prepared setup edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigChange {
    /// Dotted TOML identity.
    pub key: String,
    /// Prior value when this replaces an existing leaf.
    pub previous: Option<String>,
    /// Proposed value.
    pub proposed: String,
}

/// An existing leaf whose value differs from the setup proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigCollision {
    /// Dotted TOML identity.
    pub key: String,
    /// Existing secret-safe rendering.
    pub existing: String,
    /// Proposed secret-safe rendering.
    pub proposed: String,
}

/// A prepared user configuration candidate.
///
/// Its custom `Debug` deliberately omits both the prior and candidate file
/// contents. Even though setup validates credential references, unrelated
/// user configuration is not diagnostic material.
pub struct PreparedConfigEdit {
    target: PathBuf,
    prior: Option<Vec<u8>>,
    candidate: Vec<u8>,
    changes: Vec<ConfigChange>,
    collisions: Vec<ConfigCollision>,
}

impl fmt::Debug for PreparedConfigEdit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedConfigEdit")
            .field("target", &self.target)
            .field("changes", &self.changes)
            .field("collisions", &self.collisions)
            .finish_non_exhaustive()
    }
}

impl PreparedConfigEdit {
    /// Destination user config.
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// Leaf-level non-secret changes shown for review.
    pub fn changes(&self) -> &[ConfigChange] {
        &self.changes
    }

    /// Differing existing values that require explicit confirmation.
    pub fn collisions(&self) -> &[ConfigCollision] {
        &self.collisions
    }

    /// A stable plain-text review that remains meaningful without color.
    pub fn preview(&self) -> String {
        let mut preview = format!("destination: {}\n", self.target.display());
        for change in &self.changes {
            match &change.previous {
                Some(previous) => preview.push_str(&format!(
                    "~ {}: {} -> {}\n",
                    change.key, previous, change.proposed
                )),
                None => preview.push_str(&format!("+ {} = {}\n", change.key, change.proposed)),
            }
        }
        preview
    }

    /// Publishes the candidate atomically.
    ///
    /// A differing existing leaf is never replaced until
    /// `allow_collisions` records the caller's explicit review decision.
    pub fn commit(
        &self,
        allow_collisions: bool,
    ) -> Result<CommittedConfigEdit, UserConfigEditError> {
        if !allow_collisions && !self.collisions.is_empty() {
            return Err(UserConfigEditError::UnconfirmedCollisions {
                count: self.collisions.len(),
            });
        }
        replace_atomically(&self.target, &self.candidate)?;
        Ok(CommittedConfigEdit {
            target: self.target.clone(),
            prior: self.prior.clone(),
            active: true,
        })
    }
}

/// A published edit that can restore the exact previous file.
#[derive(Debug)]
pub struct CommittedConfigEdit {
    target: PathBuf,
    prior: Option<Vec<u8>>,
    active: bool,
}

impl CommittedConfigEdit {
    /// Makes rollback unnecessary after successful full preflight.
    pub fn accept(mut self) {
        self.active = false;
    }

    /// Restores the exact prior bytes, or removes a newly-created config.
    pub fn rollback(mut self) -> Result<(), UserConfigEditError> {
        self.restore()?;
        self.active = false;
        Ok(())
    }

    fn restore(&self) -> Result<(), UserConfigEditError> {
        match &self.prior {
            Some(prior) => replace_atomically(&self.target, prior),
            None => {
                match fs::remove_file(&self.target) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(UserConfigEditError::Io {
                            path: self.target.clone(),
                            message: error.to_string(),
                        });
                    }
                }
                sync_parent(&self.target);
                Ok(())
            }
        }
    }
}

/// Why a reviewed user-config edit could not be prepared or published.
#[derive(Debug, thiserror::Error)]
pub enum UserConfigEditError {
    /// Existing configuration cannot be read.
    #[error("`{path}` could not be read: {message}")]
    Unreadable {
        /// Target path.
        path: PathBuf,
        /// Filesystem diagnostic.
        message: String,
    },
    /// Existing TOML is not safe to merge automatically.
    #[error("`{path}` is not valid Smith configuration; fix it before running setup")]
    InvalidExistingConfig {
        /// Target path.
        path: PathBuf,
    },
    /// A setup effect attempted to persist a value instead of a credential
    /// reference.
    #[error("setup for provider `{provider}` supplied an invalid credential reference")]
    UnsafeCredential {
        /// Provider identity, never the credential value.
        provider: String,
    },
    /// A setup effect attempted to write an authorization-bearing header.
    #[error("setup for provider `{provider}` cannot write authorization-bearing header `{header}`")]
    UnsafeHeader {
        /// Provider identity.
        provider: String,
        /// Header name.
        header: String,
    },
    /// The typed patch could not produce valid Smith configuration.
    #[error("the proposed setup edit could not be represented as valid Smith configuration")]
    InvalidCandidate,
    /// Existing differing values were not explicitly approved.
    #[error("{count} existing configuration value(s) differ; review and confirm the collisions")]
    UnconfirmedCollisions {
        /// Number of collisions.
        count: usize,
    },
    /// Atomic publication failed.
    #[error("`{path}` could not be updated: {message}")]
    Io {
        /// Path being updated.
        path: PathBuf,
        /// Filesystem diagnostic.
        message: String,
    },
}

/// Prepares a comment-preserving edit to `user_dir/config.toml`.
///
/// `patch` contains only the declarations setup owns for this action. Missing
/// fields do not delete anything from the existing document.
pub fn prepare_user_config_edit(
    user_dir: impl AsRef<Path>,
    patch: &ConfigFile,
) -> Result<PreparedConfigEdit, UserConfigEditError> {
    validate_safe_references(patch)?;
    let target = user_dir.as_ref().join(CONFIG_FILE);
    let prior = match fs::read(&target) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(UserConfigEditError::Unreadable {
                path: target,
                message: error.to_string(),
            });
        }
    };

    let existing_text = match &prior {
        Some(bytes) => std::str::from_utf8(bytes)
            .map_err(|_| UserConfigEditError::InvalidExistingConfig {
                path: target.clone(),
            })?
            .to_owned(),
        None => String::new(),
    };
    let existing_file = ConfigFile::parse(&existing_text).map_err(|_| {
        UserConfigEditError::InvalidExistingConfig {
            path: target.clone(),
        }
    })?;
    validate_safe_references(&existing_file).map_err(|_| {
        UserConfigEditError::InvalidExistingConfig {
            path: target.clone(),
        }
    })?;

    let mut candidate = existing_text.parse::<DocumentMut>().map_err(|_| {
        UserConfigEditError::InvalidExistingConfig {
            path: target.clone(),
        }
    })?;
    let patch_text =
        toml::to_string_pretty(patch).map_err(|_| UserConfigEditError::InvalidCandidate)?;
    let patch_document = patch_text
        .parse::<DocumentMut>()
        .map_err(|_| UserConfigEditError::InvalidCandidate)?;

    let mut changes = Vec::new();
    let mut collisions = Vec::new();
    remove_alternative_credential_fields(
        candidate.as_table_mut(),
        patch,
        &mut changes,
        &mut collisions,
    );
    merge_table(
        candidate.as_table_mut(),
        patch_document.as_table(),
        &mut Vec::new(),
        &mut changes,
        &mut collisions,
    );
    let candidate_text = candidate.to_string();
    let candidate_file =
        ConfigFile::parse(&candidate_text).map_err(|_| UserConfigEditError::InvalidCandidate)?;
    validate_safe_references(&candidate_file)?;

    Ok(PreparedConfigEdit {
        target,
        prior,
        candidate: candidate_text.into_bytes(),
        changes,
        collisions,
    })
}

/// Prepares removal of every explicit checkpoint-key source so Smith returns
/// to its platform-protected default.
///
/// The caller must first prove that changing key sources cannot strand
/// encrypted checkpoint state. This function only performs the reviewed,
/// comment-preserving user-config transaction and never opens the credential
/// service itself.
pub fn prepare_checkpoint_key_source_removal(
    user_dir: impl AsRef<Path>,
) -> Result<PreparedConfigEdit, UserConfigEditError> {
    let target = user_dir.as_ref().join(CONFIG_FILE);
    let prior = match fs::read(&target) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(UserConfigEditError::Unreadable {
                path: target,
                message: error.to_string(),
            });
        }
    };
    let existing_text = match &prior {
        Some(bytes) => std::str::from_utf8(bytes)
            .map_err(|_| UserConfigEditError::InvalidExistingConfig {
                path: target.clone(),
            })?
            .to_owned(),
        None => String::new(),
    };
    let existing_file = ConfigFile::parse(&existing_text).map_err(|_| {
        UserConfigEditError::InvalidExistingConfig {
            path: target.clone(),
        }
    })?;
    validate_safe_references(&existing_file).map_err(|_| {
        UserConfigEditError::InvalidExistingConfig {
            path: target.clone(),
        }
    })?;
    let mut candidate = existing_text.parse::<DocumentMut>().map_err(|_| {
        UserConfigEditError::InvalidExistingConfig {
            path: target.clone(),
        }
    })?;
    let mut changes = Vec::new();
    let mut collisions = Vec::new();
    if let Some(table) = candidate
        .as_table_mut()
        .get_mut("persistence")
        .and_then(Item::as_table_mut)
    {
        for field in ["checkpoint_key", "checkpoint_key_credential"] {
            let Some(previous_item) = table.remove(field) else {
                continue;
            };
            let path = vec!["persistence".to_owned(), field.to_owned()];
            let key = display_path(&path);
            let previous = safe_render(&path, &previous_item);
            changes.push(ConfigChange {
                key: key.clone(),
                previous: Some(previous.clone()),
                proposed: "<platform default>".to_owned(),
            });
            collisions.push(ConfigCollision {
                key,
                existing: previous,
                proposed: "<platform default>".to_owned(),
            });
        }
    }
    let candidate_text = candidate.to_string();
    let candidate_file =
        ConfigFile::parse(&candidate_text).map_err(|_| UserConfigEditError::InvalidCandidate)?;
    validate_safe_references(&candidate_file)?;
    Ok(PreparedConfigEdit {
        target,
        prior,
        candidate: candidate_text.into_bytes(),
        changes,
        collisions,
    })
}

/// Prepares removal of one provider's explicit credential source while
/// preserving its endpoint, models, profiles, headers, and defaults.
///
/// This transaction never opens the credential service. The caller may use
/// the prior typed configuration to remove a Keychain entry after publishing
/// this edit, rolling the edit back if protected-storage cleanup fails.
pub fn prepare_provider_credential_removal(
    user_dir: impl AsRef<Path>,
    provider: &str,
) -> Result<PreparedConfigEdit, UserConfigEditError> {
    let target = user_dir.as_ref().join(CONFIG_FILE);
    let prior = match fs::read(&target) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(UserConfigEditError::Unreadable {
                path: target,
                message: error.to_string(),
            });
        }
    };
    let existing_text = match &prior {
        Some(bytes) => std::str::from_utf8(bytes)
            .map_err(|_| UserConfigEditError::InvalidExistingConfig {
                path: target.clone(),
            })?
            .to_owned(),
        None => String::new(),
    };
    let existing_file = ConfigFile::parse(&existing_text).map_err(|_| {
        UserConfigEditError::InvalidExistingConfig {
            path: target.clone(),
        }
    })?;
    validate_safe_references(&existing_file).map_err(|_| {
        UserConfigEditError::InvalidExistingConfig {
            path: target.clone(),
        }
    })?;
    let mut candidate = existing_text.parse::<DocumentMut>().map_err(|_| {
        UserConfigEditError::InvalidExistingConfig {
            path: target.clone(),
        }
    })?;
    let mut changes = Vec::new();
    let mut collisions = Vec::new();
    if let Some(table) = candidate
        .as_table_mut()
        .get_mut("providers")
        .and_then(Item::as_table_mut)
        .and_then(|providers| providers.get_mut(provider))
        .and_then(Item::as_table_mut)
    {
        for field in ["credential", "credentials", "api_key"] {
            let Some(previous_item) = table.remove(field) else {
                continue;
            };
            let path = vec![
                "providers".to_owned(),
                provider.to_owned(),
                field.to_owned(),
            ];
            let key = display_path(&path);
            let previous = safe_render(&path, &previous_item);
            changes.push(ConfigChange {
                key: key.clone(),
                previous: Some(previous.clone()),
                proposed: "<disconnected>".to_owned(),
            });
            collisions.push(ConfigCollision {
                key,
                existing: previous,
                proposed: "<disconnected>".to_owned(),
            });
        }
    }
    let candidate_text = candidate.to_string();
    let candidate_file =
        ConfigFile::parse(&candidate_text).map_err(|_| UserConfigEditError::InvalidCandidate)?;
    validate_safe_references(&candidate_file)?;
    Ok(PreparedConfigEdit {
        target,
        prior,
        candidate: candidate_text.into_bytes(),
        changes,
        collisions,
    })
}

/// Replacing a provider credential is intentionally different from an
/// additive setup patch: the two source fields are mutually exclusive, so a
/// reviewed new source removes the old alternative before the ordinary merge.
fn remove_alternative_credential_fields(
    candidate: &mut Table,
    patch: &ConfigFile,
    changes: &mut Vec<ConfigChange>,
    collisions: &mut Vec<ConfigCollision>,
) {
    for (provider, section) in &patch.providers {
        // The three spellings are one declaration: a single account, an
        // ordered pool, or an inline key. A patch that chooses one replaces
        // the others, and the removal is recorded so the preview shows the
        // pool or key the new declaration displaces.
        let spellings = [
            ("credential", section.credential.is_some()),
            ("credentials", !section.credentials.is_empty()),
            ("api_key", section.api_key.is_some()),
        ];
        if spellings.iter().filter(|(_, present)| *present).count() != 1 {
            continue;
        }
        let Some(provider_table) = candidate
            .get_mut("providers")
            .and_then(Item::as_table_mut)
            .and_then(|providers| providers.get_mut(provider))
            .and_then(Item::as_table_mut)
        else {
            continue;
        };
        for alternative in spellings
            .iter()
            .filter(|(_, present)| !*present)
            .map(|(name, _)| *name)
        {
            let Some(previous_item) = provider_table.remove(alternative) else {
                continue;
            };

            let path = vec![
                "providers".to_owned(),
                provider.clone(),
                alternative.to_owned(),
            ];
            let key = display_path(&path);
            let previous = safe_render(&path, &previous_item);
            changes.push(ConfigChange {
                key: key.clone(),
                previous: Some(previous.clone()),
                proposed: "<removed>".to_owned(),
            });
            collisions.push(ConfigCollision {
                key,
                existing: previous,
                proposed: "<removed>".to_owned(),
            });
        }
    }

    let Some(persistence) = &patch.persistence else {
        return;
    };
    let alternative = match (
        persistence.checkpoint_key.is_some(),
        persistence.checkpoint_key_credential.is_some(),
    ) {
        (true, false) => "checkpoint_key_credential",
        (false, true) => "checkpoint_key",
        _ => return,
    };
    let Some(table) = candidate
        .get_mut("persistence")
        .and_then(Item::as_table_mut)
    else {
        return;
    };
    let Some(previous_item) = table.remove(alternative) else {
        return;
    };
    let path = vec!["persistence".to_owned(), alternative.to_owned()];
    let key = display_path(&path);
    let previous = safe_render(&path, &previous_item);
    changes.push(ConfigChange {
        key: key.clone(),
        previous: Some(previous.clone()),
        proposed: "<removed>".to_owned(),
    });
    collisions.push(ConfigCollision {
        key,
        existing: previous,
        proposed: "<removed>".to_owned(),
    });
}

fn validate_safe_references(file: &ConfigFile) -> Result<(), UserConfigEditError> {
    if let Some(persistence) = &file.persistence {
        if persistence.checkpoint_key.is_some() && persistence.checkpoint_key_credential.is_some() {
            return Err(UserConfigEditError::InvalidCandidate);
        }
        if let Some(reference) = &persistence.checkpoint_key_credential
            && CredentialRef::parse(reference).is_err()
        {
            return Err(UserConfigEditError::InvalidCandidate);
        }
    }
    for (provider, section) in &file.providers {
        if section.credential.is_some() && section.api_key.is_some() {
            return Err(UserConfigEditError::UnsafeCredential {
                provider: provider.clone(),
            });
        }
        if let Some(reference) = &section.credential
            && CredentialRef::parse(reference).is_err()
        {
            return Err(UserConfigEditError::UnsafeCredential {
                provider: provider.clone(),
            });
        }
        if let Some(header) = section.headers.keys().find(|header| {
            matches!(
                header.to_ascii_lowercase().as_str(),
                "authorization" | "proxy-authorization" | "x-api-key" | "api-key"
            )
        }) {
            return Err(UserConfigEditError::UnsafeHeader {
                provider: provider.clone(),
                header: header.clone(),
            });
        }
    }
    Ok(())
}

fn merge_table(
    existing: &mut Table,
    proposed: &Table,
    prefix: &mut Vec<String>,
    changes: &mut Vec<ConfigChange>,
    collisions: &mut Vec<ConfigCollision>,
) {
    for (key, proposed_item) in proposed.iter() {
        prefix.push(key.to_owned());
        match existing.get_mut(key) {
            Some(existing_item) => {
                if existing_item.as_table().is_some() && proposed_item.as_table().is_some() {
                    merge_table(
                        existing_item
                            .as_table_mut()
                            .expect("the table shape was checked"),
                        proposed_item
                            .as_table()
                            .expect("the table shape was checked"),
                        prefix,
                        changes,
                        collisions,
                    );
                } else if !semantically_equal(existing_item, proposed_item) {
                    let path = display_path(prefix);
                    let previous = safe_render(prefix, existing_item);
                    let next = safe_render(prefix, proposed_item);
                    changes.push(ConfigChange {
                        key: path.clone(),
                        previous: Some(previous.clone()),
                        proposed: next.clone(),
                    });
                    collisions.push(ConfigCollision {
                        key: path,
                        existing: previous,
                        proposed: next,
                    });
                    *existing_item = proposed_item.clone();
                }
            }
            None => {
                record_added_leaves(proposed_item, prefix, changes);
                existing.insert(key, proposed_item.clone());
            }
        }
        prefix.pop();
    }
}

fn record_added_leaves(item: &Item, prefix: &mut Vec<String>, changes: &mut Vec<ConfigChange>) {
    if let Some(table) = item.as_table() {
        for (key, child) in table.iter() {
            prefix.push(key.to_owned());
            record_added_leaves(child, prefix, changes);
            prefix.pop();
        }
    } else {
        changes.push(ConfigChange {
            key: display_path(prefix),
            previous: None,
            proposed: safe_render(prefix, item),
        });
    }
}

fn semantically_equal(left: &Item, right: &Item) -> bool {
    match (left.as_value(), right.as_value()) {
        (Some(left), Some(right)) => {
            let left = format!("value = {left}");
            let right = format!("value = {right}");
            toml::from_str::<toml::Table>(&left)
                .ok()
                .and_then(|table| table.get("value").cloned())
                == toml::from_str::<toml::Table>(&right)
                    .ok()
                    .and_then(|table| table.get("value").cloned())
        }
        _ => false,
    }
}

fn safe_render(path: &[String], item: &Item) -> String {
    if path
        .last()
        .is_some_and(|segment| matches!(segment.as_str(), "api_key" | "checkpoint_key"))
    {
        return "[redacted]".to_owned();
    }
    if path.iter().any(|segment| segment == "headers") {
        return "[configured header value]".to_owned();
    }
    item.as_value()
        .map_or_else(|| format!("<{}>", item.type_name()), ToString::to_string)
}

fn display_path(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| {
            if !part.is_empty()
                && part
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
            {
                part.clone()
            } else {
                format!("\"{}\"", part.replace('"', "\\\""))
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

fn replace_atomically(path: &Path, bytes: &[u8]) -> Result<(), UserConfigEditError> {
    let directory = path.parent().ok_or_else(|| UserConfigEditError::Io {
        path: path.to_path_buf(),
        message: "the config path has no parent directory".to_owned(),
    })?;
    fs::create_dir_all(directory).map_err(|error| UserConfigEditError::Io {
        path: directory.to_path_buf(),
        message: error.to_string(),
    })?;
    restrict_directory(directory)?;

    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let temporary = directory.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| UserConfigEditError::Io {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| UserConfigEditError::Io {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        fs::rename(&temporary, path).map_err(|error| UserConfigEditError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        restrict_file(path)?;
        sync_parent(path);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> Result<(), UserConfigEditError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
        UserConfigEditError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> Result<(), UserConfigEditError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(path: &Path) -> Result<(), UserConfigEditError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        UserConfigEditError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })
}

#[cfg(not(unix))]
fn restrict_file(_path: &Path) -> Result<(), UserConfigEditError> {
    Ok(())
}

fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent()
        && let Ok(directory) = fs::File::open(parent)
    {
        let _ = directory.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_source_removal_is_redacted_comment_preserving_and_atomic() {
        const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let directory = tempfile::tempdir().expect("temporary config root");
        let path = directory.path().join(CONFIG_FILE);
        fs::write(
            &path,
            format!(
                "# retained comment\n[persistence]\nenabled = true\ncheckpoint_key = \"{KEY}\"\n"
            ),
        )
        .expect("seed config");

        let prepared =
            prepare_checkpoint_key_source_removal(directory.path()).expect("prepared removal");
        let preview = prepared.preview();
        assert!(preview.contains("[redacted]"), "{preview}");
        assert!(preview.contains("<platform default>"), "{preview}");
        assert!(!preview.contains(KEY), "{preview}");
        assert_eq!(prepared.collisions().len(), 1);
        assert!(prepared.commit(false).is_err(), "confirmation is required");

        prepared.commit(true).expect("commit").accept();
        let stored = fs::read_to_string(path).expect("stored config");
        assert!(stored.contains("# retained comment"), "{stored}");
        assert!(stored.contains("enabled = true"), "{stored}");
        assert!(!stored.contains("checkpoint_key"), "{stored}");
        assert!(!stored.contains(KEY), "{stored}");
    }

    /// The three credential spellings are one declaration: a patch choosing a
    /// pool visibly displaces a single credential, and a patch choosing a
    /// single credential visibly displaces the pool it collapses.
    #[test]
    fn a_credential_patch_replaces_the_alternative_spellings_it_displaces() {
        let directory = tempfile::tempdir().expect("temporary config root");
        let path = directory.path().join(CONFIG_FILE);
        fs::write(
            &path,
            "[providers.chatgpt]\nkind = \"chatgpt-responses\"\ncredential = \"authfile:chatgpt\"\n",
        )
        .expect("seed config");

        // Single → pool: the `credential` key is removed, not left to fight
        // the pool over which spelling declares the account.
        let pool_patch = ConfigFile {
            providers: std::collections::BTreeMap::from([(
                "chatgpt".to_owned(),
                crate::model::ProviderSection {
                    credentials: vec![
                        "authfile:chatgpt".to_owned(),
                        "authfile:chatgpt-2".to_owned(),
                    ],
                    ..crate::model::ProviderSection::default()
                },
            )]),
            ..ConfigFile::default()
        };
        let prepared =
            prepare_user_config_edit(directory.path(), &pool_patch).expect("prepared pool edit");
        prepared.commit(true).expect("commit").accept();
        let stored = fs::read_to_string(&path).expect("stored config");
        assert!(stored.contains("credentials"), "{stored}");
        assert!(!stored.contains("credential = "), "{stored}");

        // Pool → single: replacing the login collapses the pool, and the
        // preview records the pool it removed.
        let single_patch = ConfigFile {
            providers: std::collections::BTreeMap::from([(
                "chatgpt".to_owned(),
                crate::model::ProviderSection {
                    credential: Some("authfile:chatgpt".to_owned()),
                    ..crate::model::ProviderSection::default()
                },
            )]),
            ..ConfigFile::default()
        };
        let prepared = prepare_user_config_edit(directory.path(), &single_patch)
            .expect("prepared single edit");
        assert!(prepared.preview().contains("<removed>"));
        prepared.commit(true).expect("commit").accept();
        let stored = fs::read_to_string(&path).expect("stored config");
        assert!(
            stored.contains("credential = \"authfile:chatgpt\""),
            "{stored}"
        );
        assert!(!stored.contains("credentials ="), "{stored}");
        assert!(!stored.contains("chatgpt-2"), "{stored}");
    }

    /// Disconnecting a pooled provider strips the whole pool, not just the
    /// single-credential spelling.
    #[test]
    fn provider_disconnect_removes_a_credentials_pool() {
        let directory = tempfile::tempdir().expect("temporary config root");
        let path = directory.path().join(CONFIG_FILE);
        fs::write(
            &path,
            "[providers.chatgpt]\nkind = \"chatgpt-responses\"\ncredentials = [\"authfile:chatgpt\", \"authfile:chatgpt-2\"]\n",
        )
        .expect("seed config");

        let prepared = prepare_provider_credential_removal(directory.path(), "chatgpt")
            .expect("prepared disconnect");
        prepared.commit(true).expect("commit").accept();

        let stored = fs::read_to_string(path).expect("stored config");
        assert!(stored.contains("chatgpt-responses"), "{stored}");
        assert!(!stored.contains("credentials"), "{stored}");
        assert!(!stored.contains("chatgpt-2"), "{stored}");
    }

    #[test]
    fn provider_disconnect_removes_only_authentication_fields() {
        const KEY: &str = "openrouter-key-canary";
        let directory = tempfile::tempdir().expect("temporary config root");
        let path = directory.path().join(CONFIG_FILE);
        fs::write(
            &path,
            format!(
                "# retained provider\n[providers.openrouter]\nkind = \"openai-compatible\"\nbase_url = \"https://openrouter.ai/api/v1\"\napi_key = \"{KEY}\"\n"
            ),
        )
        .expect("seed config");

        let prepared = prepare_provider_credential_removal(directory.path(), "openrouter")
            .expect("prepared disconnect");
        let preview = prepared.preview();
        assert!(preview.contains("[redacted]"), "{preview}");
        assert!(preview.contains("<disconnected>"), "{preview}");
        assert!(!preview.contains(KEY), "{preview}");
        prepared.commit(true).expect("commit").accept();

        let stored = fs::read_to_string(path).expect("stored config");
        assert!(stored.contains("# retained provider"), "{stored}");
        assert!(stored.contains("https://openrouter.ai/api/v1"), "{stored}");
        assert!(!stored.contains("api_key"), "{stored}");
        assert!(!stored.contains(KEY), "{stored}");
    }
}
