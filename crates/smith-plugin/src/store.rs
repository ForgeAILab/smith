//! Filesystem transactions and explicit preparation/commit boundaries.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use smith_config::trust::ContentDigest;
use tempfile::TempDir;

use crate::manifest::confined;
use crate::source::copy_directory;
use crate::{
    GitFetcher, InventoryDifference, Marketplace, MarketplaceRecord, PluginError, PluginInventory,
    PluginSource, Result, inspect, validate_name,
};

/// The recorded source and identity of an installed version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPlugin {
    /// Validated plugin name.
    pub name: String,
    /// Manifest version, when present.
    pub version: Option<String>,
    /// Original source; marketplace entries retain their catalogue identity.
    pub source: PluginSource,
    /// Requested git branch or tag.
    pub reference: Option<String>,
    /// Git commit of the fetched checkout.
    pub resolved_commit: Option<String>,
    /// Digest the caller reviewed before committing.
    pub digest: ContentDigest,
    /// Confirmed module grants, bound to this record's digest.
    #[serde(default)]
    pub module_grants: BTreeMap<String, crate::ModuleCapabilities>,
    /// Canonical directory from which the installed version is inspected.
    pub install_directory: PathBuf,
}

/// Content verification, without enabling a plugin or making a trust decision.
#[derive(Debug, Clone)]
pub enum InstalledState {
    /// Installed bytes match the recorded digest.
    Unchanged(PluginInventory),
    /// Installed bytes no longer match; callers must withhold this inventory.
    Changed {
        /// Persisted record identifying the reviewed version.
        installed: Box<InstalledPlugin>,
        /// Present inventory, for a new confirmation.
        inventory: PluginInventory,
    },
    /// The directory or manifest cannot be inspected.
    Failed {
        /// Persisted record.
        installed: InstalledPlugin,
        /// Reason the installed version could not be inspected.
        reason: String,
    },
}

/// A private copied snapshot, ready for a caller to display and confirm.
#[derive(Debug)]
pub struct PreparedPlugin {
    inventory: PluginInventory,
    source: PluginSource,
    reference: Option<String>,
    resolved_commit: Option<String>,
    _temporary: TempDir,
}

impl PreparedPlugin {
    /// The exact snapshot a confirmation must describe.
    pub fn inventory(&self) -> &PluginInventory {
        &self.inventory
    }
    /// The source that will be persisted on commit.
    pub fn source(&self) -> &PluginSource {
        &self.source
    }
    /// Full git commit, when fetched from git.
    pub fn resolved_commit(&self) -> Option<&str> {
        self.resolved_commit.as_deref()
    }
}

/// A proposed replacement which has not changed the installed version.
#[derive(Debug)]
pub struct PreparedUpdate {
    /// Record which must still be current when the update is committed.
    pub installed: InstalledPlugin,
    /// Added, removed, and changed components for confirmation.
    pub difference: InventoryDifference,
    /// Snapshot to be committed only after the caller confirms it.
    pub prepared: PreparedPlugin,
    /// Local modifications will be discarded if this replacement is confirmed.
    pub discards_local_changes: bool,
    /// Inspection failure in the old installation, if its inventory is unavailable.
    pub previous_failure: Option<String>,
    reviewed_content: std::result::Result<ContentDigest, String>,
}

/// Filesystem store rooted at a caller-supplied Smith state directory.
#[derive(Debug, Clone)]
pub struct PluginStore {
    state_root: PathBuf,
    git: GitFetcher,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct InstalledRecords {
    #[serde(default)]
    plugins: BTreeMap<String, InstalledPlugin>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct MarketplaceRecords {
    #[serde(default)]
    marketplaces: BTreeMap<String, MarketplaceRecord>,
}

impl PluginStore {
    /// Selects a state root without creating directories or fetching content.
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
            git: GitFetcher::default(),
        }
    }

    /// Uses an injected git executable for fetching and tests.
    pub fn with_git_program(mut self, program: impl Into<std::ffi::OsString>) -> Self {
        self.git = GitFetcher::new(program);
        self
    }

    /// Persistent data path, which survives updates and ordinary removal.
    pub fn data_directory(&self, name: &str) -> Result<PathBuf> {
        validate_name(name)?;
        Ok(self.absolute_state_root()?.join("plugins/data").join(name))
    }

    /// Planned absolute version path, available before an installation is committed.
    ///
    /// A confirmation can use this path for variable expansion rather than the
    /// temporary snapshot path. This method creates nothing.
    pub fn install_directory(&self, name: &str, digest: &ContentDigest) -> Result<PathBuf> {
        validate_name(name)?;
        Ok(self
            .absolute_state_root()?
            .join("plugins/store")
            .join(name)
            .join(digest_prefix(digest)?))
    }

    /// Lists persisted records without creating a store.
    pub fn list(&self) -> Result<Vec<InstalledPlugin>> {
        Ok(self.installed_records()?.plugins.into_values().collect())
    }

    /// Reads one installed version and detects modifications against its digest.
    pub fn load(&self, name: &str) -> Result<InstalledState> {
        let installed = self.record(name)?;
        let inventory = self
            .checked_install_path(&installed)
            .and_then(|root| inspect(&root, name));
        Ok(match inventory {
            Ok(inventory) if inventory.digest == installed.digest => {
                InstalledState::Unchanged(inventory)
            }
            Ok(inventory) => InstalledState::Changed {
                installed: Box::new(installed),
                inventory,
            },
            Err(error) => InstalledState::Failed {
                installed,
                reason: error.to_string(),
            },
        })
    }

    /// Fetches and copies a snapshot; nothing is written to the plugin store.
    pub fn prepare_install(
        &self,
        source: &PluginSource,
        fallback_name: Option<&str>,
    ) -> Result<PreparedPlugin> {
        let (resolved, fallback) = match source {
            PluginSource::Marketplace {
                plugin,
                marketplace,
            } => (
                self.resolve_marketplace(marketplace, plugin)?,
                Some(plugin.clone()),
            ),
            other => (other.clone(), fallback_name.map(str::to_owned)),
        };
        let fetched = self.git.fetch(&resolved)?;
        let fallback = fallback
            .or_else(|| source_name(&fetched.source))
            .unwrap_or_else(|| "plugin".into());
        let temporary = tempfile::tempdir()?;
        let snapshot = temporary.path().join("plugin");
        copy_directory(&fetched.root, &snapshot)?;
        let inventory = inspect(&snapshot, &fallback)?;
        let recorded_source = if matches!(source, PluginSource::Marketplace { .. }) {
            source.clone()
        } else {
            fetched.source
        };
        Ok(PreparedPlugin {
            inventory,
            source: recorded_source,
            reference: resolved.reference().map(str::to_owned),
            resolved_commit: fetched.resolved_commit,
            _temporary: temporary,
        })
    }

    /// Atomically records an already-confirmed snapshot. This method never prompts.
    pub fn commit_install(&self, prepared: PreparedPlugin) -> Result<InstalledPlugin> {
        let root = self.ensure_root()?;
        let _lock = StoreLock::acquire(&root)?;
        self.commit_prepared(&root, prepared)
    }

    /// Fetches an update and computes its difference while leaving the old record intact.
    pub fn prepare_update(&self, name: &str) -> Result<PreparedUpdate> {
        let installed = self.record(name)?;
        let prepared = self.prepare_install(&installed.source, Some(&installed.name))?;
        if prepared.inventory.manifest.name != installed.name {
            return Err(PluginError::Invalid(
                "an update cannot rename an installed plugin".into(),
            ));
        }
        self.prepare_replacement(prepared)
    }

    /// Treats a freshly fetched add as an update of the same installed name.
    /// The current local inventory is reviewed even when it differs from trust.
    pub fn prepare_replacement(&self, prepared: PreparedPlugin) -> Result<PreparedUpdate> {
        let name = &prepared.inventory.manifest.name;
        let installed = self.record(name)?;
        let reviewed_content = self.content_identity(&installed);
        let (old, discards_local_changes, previous_failure) = match self.load(name)? {
            InstalledState::Unchanged(inventory) => (inventory, false, None),
            InstalledState::Changed { inventory, .. } => (inventory, true, None),
            InstalledState::Failed { reason, .. } => {
                // There is no reliable old component inventory. The fetched
                // inventory is still reviewed in full, and every new component
                // is shown as added rather than guessing at the broken manifest.
                let mut empty = prepared.inventory.clone();
                empty.skills.clear();
                empty.commands.clear();
                empty.mcp_servers.clear();
                empty.not_loaded.clear();
                (empty, true, Some(reason))
            }
        };
        let difference = InventoryDifference::between(&old, &prepared.inventory);
        Ok(PreparedUpdate {
            installed,
            difference,
            prepared,
            discards_local_changes,
            previous_failure,
            reviewed_content,
        })
    }

    /// Commits an already-confirmed update if its base version is still current.
    pub fn commit_update(&self, update: PreparedUpdate) -> Result<InstalledPlugin> {
        let root = self.ensure_root()?;
        let _lock = StoreLock::acquire(&root)?;
        if self.record(&update.installed.name)? != update.installed {
            return Err(PluginError::Changed);
        }
        if self.content_identity(&update.installed) != update.reviewed_content {
            return Err(PluginError::Changed);
        }
        self.commit_prepared(&root, update.prepared)
    }

    /// Removes installed content and the record, retaining data unless requested.
    pub fn remove(&self, name: &str, delete_data: bool) -> Result<InstalledPlugin> {
        validate_name(name)?;
        let root = self.ensure_root()?;
        let _lock = StoreLock::acquire(&root)?;
        let mut records = self.installed_records()?;
        let installed = records
            .plugins
            .remove(name)
            .ok_or_else(|| PluginError::Invalid(format!("plugin `{name}` is not installed")))?;
        let directory = root.join("store").join(name);
        let temporary = tempfile::tempdir_in(&root)?;
        let trash = temporary.path().join("removed");
        let moved = if directory.exists() {
            confined(&root, &directory)?;
            fs::rename(&directory, &trash)?;
            true
        } else {
            false
        };
        let data = root.join("data").join(name);
        let data_trash = temporary.path().join("data");
        let data_moved = if delete_data && data.exists() {
            if let Err(error) =
                confined(&root, &data).and_then(|_| Ok(fs::rename(&data, &data_trash)?))
            {
                if moved {
                    fs::rename(&trash, &directory)?;
                }
                return Err(error);
            }
            true
        } else {
            false
        };
        if let Err(error) = atomic_record(&root.join("installed.toml"), &records) {
            if moved {
                fs::rename(&trash, &directory)?;
            }
            if data_moved {
                fs::rename(&data_trash, &data)?;
            }
            return Err(error);
        }
        temporary.close()?;
        Ok(installed)
    }

    /// Fetches, validates, and atomically registers a marketplace copy.
    pub fn add_marketplace(&self, source: &PluginSource) -> Result<MarketplaceRecord> {
        let fetched = self.git.fetch(source)?;
        let catalogue = Marketplace::read(&fetched.root)?;
        let root = self.ensure_root()?;
        let _lock = StoreLock::acquire(&root)?;
        let mut records = self.marketplace_records()?;
        if records.marketplaces.contains_key(&catalogue.name) {
            return Err(PluginError::Invalid(format!(
                "marketplace `{}` is already registered",
                catalogue.name
            )));
        }
        self.commit_marketplace(&root, &fetched, &catalogue.name, &mut records)
    }

    /// Lists registered marketplaces without fetching them.
    pub fn list_marketplaces(&self) -> Result<Vec<MarketplaceRecord>> {
        Ok(self
            .marketplace_records()?
            .marketplaces
            .into_values()
            .collect())
    }

    /// Returns the stored catalogue and its unsupported entries unchanged.
    pub fn marketplace(&self, name: &str) -> Result<Marketplace> {
        let record = self.marketplace_record(name)?;
        Marketplace::read(&self.checked_marketplace_path(&record)?)
    }

    /// Refreshes a marketplace only on this explicit call.
    pub fn update_marketplace(&self, name: &str) -> Result<MarketplaceRecord> {
        let previous = self.marketplace_record(name)?;
        let fetched = self.git.fetch(&previous.source)?;
        let catalogue = Marketplace::read(&fetched.root)?;
        if catalogue.name != name {
            return Err(PluginError::Invalid(
                "a marketplace update cannot rename the catalogue".into(),
            ));
        }
        let root = self.ensure_root()?;
        let _lock = StoreLock::acquire(&root)?;
        if self.marketplace_record(name)? != previous {
            return Err(PluginError::Changed);
        }
        self.commit_marketplace(&root, &fetched, name, &mut self.marketplace_records()?)
    }

    /// Removes a catalogue copy and record, leaving installed plugins untouched.
    pub fn remove_marketplace(&self, name: &str) -> Result<MarketplaceRecord> {
        validate_name(name)?;
        let root = self.ensure_root()?;
        let _lock = StoreLock::acquire(&root)?;
        let mut records = self.marketplace_records()?;
        let record = records.marketplaces.remove(name).ok_or_else(|| {
            PluginError::Invalid(format!("marketplace `{name}` is not registered"))
        })?;
        let directory = self.checked_marketplace_path(&record)?;
        let temporary = tempfile::tempdir_in(&root)?;
        let trash = temporary.path().join("removed");
        fs::rename(&directory, &trash)?;
        if let Err(error) = atomic_record(&root.join("marketplaces.toml"), &records) {
            fs::rename(&trash, &directory)?;
            return Err(error);
        }
        temporary.close()?;
        Ok(record)
    }

    /// Resolves an entry against the stored catalogue, without fetching the plugin.
    pub fn resolve_marketplace(&self, marketplace: &str, plugin: &str) -> Result<PluginSource> {
        let record = self.marketplace_record(marketplace)?;
        let directory = self.checked_marketplace_path(&record)?;
        Marketplace::read(&directory)?.resolve(&directory, plugin)
    }

    fn content_identity(
        &self,
        installed: &InstalledPlugin,
    ) -> std::result::Result<ContentDigest, String> {
        self.checked_install_path(installed)
            .and_then(|root| crate::digest::directory_digest(&root, &installed.name))
            .map_err(|error| error.to_string())
    }

    fn commit_prepared(&self, root: &Path, prepared: PreparedPlugin) -> Result<InstalledPlugin> {
        let inventory = &prepared.inventory;
        let name = &inventory.manifest.name;
        validate_name(name)?;
        if inspect(&inventory.root, name)?.digest != inventory.digest {
            return Err(PluginError::Changed);
        }
        let parent_path = root.join("store").join(name);
        let _parent_cleanup = EmptyDirectoryCleanup::new(&parent_path);
        let parent = ensure_directory(root, &parent_path)?;
        let destination = parent.join(digest_prefix(&inventory.digest)?);
        let temporary = tempfile::tempdir_in(&parent)?;
        let snapshot = temporary.path().join("plugin");
        copy_directory(&inventory.root, &snapshot)?;
        if inspect(&snapshot, name)?.digest != inventory.digest {
            return Err(PluginError::Changed);
        }
        let existed = destination.exists();
        let backup = temporary.path().join("prior");
        let mut replaced = false;
        if existed {
            let resolved = confined(root, &destination)?;
            if !inspect(&resolved, name).is_ok_and(|current| current.digest == inventory.digest) {
                fs::rename(&destination, &backup)?;
                if let Err(error) = fs::rename(&snapshot, &destination) {
                    fs::rename(&backup, &destination)?;
                    return Err(error.into());
                }
                replaced = true;
            }
        } else {
            fs::rename(&snapshot, &destination)?;
        }
        let data_path = root.join("data").join(name);
        let had_data = data_path.exists();
        let recorded = (|| {
            ensure_directory(root, &data_path)?;
            let installed = InstalledPlugin {
                module_grants: prepared
                    .inventory
                    .modules
                    .iter()
                    .map(|module| (module.id.clone(), module.capabilities.clone()))
                    .collect(),
                name: name.clone(),
                version: inventory.manifest.version.clone(),
                source: prepared.source,
                reference: prepared.reference,
                resolved_commit: prepared.resolved_commit,
                digest: inventory.digest.clone(),
                install_directory: destination.canonicalize()?,
            };
            let mut records = self.installed_records()?;
            records.plugins.insert(name.clone(), installed.clone());
            atomic_record(&root.join("installed.toml"), &records)?;
            Ok(installed)
        })();
        if recorded.is_err() {
            if !existed || replaced {
                fs::remove_dir_all(&destination)?;
            }
            if replaced {
                fs::rename(&backup, &destination)?;
            }
            if !had_data && data_path.exists() {
                fs::remove_dir(&data_path)?;
            }
        }
        recorded
    }

    fn commit_marketplace(
        &self,
        root: &Path,
        fetched: &crate::FetchedSource,
        name: &str,
        records: &mut MarketplaceRecords,
    ) -> Result<MarketplaceRecord> {
        let parent = ensure_directory(root, &root.join("marketplaces"))?;
        let temporary = tempfile::tempdir_in(&parent)?;
        let snapshot = temporary.path().join("catalogue");
        copy_directory(&fetched.root, &snapshot)?;
        if Marketplace::read(&snapshot)?.name != name {
            return Err(PluginError::Changed);
        }
        let destination = parent.join(name);
        let backup = temporary.path().join("prior");
        let existed = destination.exists();
        if existed {
            confined(root, &destination)?;
            fs::rename(&destination, &backup)?;
        }
        if let Err(error) = fs::rename(&snapshot, &destination) {
            if existed {
                fs::rename(&backup, &destination)?;
            }
            return Err(error.into());
        }
        let record = MarketplaceRecord {
            name: name.into(),
            source: fetched.source.clone(),
            resolved_commit: fetched.resolved_commit.clone(),
            directory: destination.clone(),
        };
        records.marketplaces.insert(name.into(), record.clone());
        if let Err(error) = atomic_record(&root.join("marketplaces.toml"), records) {
            fs::remove_dir_all(&destination)?;
            if existed {
                fs::rename(&backup, &destination)?;
            }
            return Err(error);
        }
        Ok(record)
    }

    fn ensure_root(&self) -> Result<PathBuf> {
        fs::create_dir_all(&self.state_root)?;
        let state = self.state_root.canonicalize()?;
        ensure_directory(&state, &state.join("plugins"))
    }

    fn absolute_state_root(&self) -> Result<PathBuf> {
        match self.state_root.canonicalize() {
            Ok(root) => Ok(root),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if self.state_root.is_absolute() {
                    Ok(self.state_root.clone())
                } else {
                    Ok(std::env::current_dir()?.join(&self.state_root))
                }
            }
            Err(error) => Err(error.into()),
        }
    }

    fn installed_records(&self) -> Result<InstalledRecords> {
        read_record(&self.state_root.join("plugins/installed.toml"))
    }

    fn marketplace_records(&self) -> Result<MarketplaceRecords> {
        read_record(&self.state_root.join("plugins/marketplaces.toml"))
    }

    fn record(&self, name: &str) -> Result<InstalledPlugin> {
        validate_name(name)?;
        self.installed_records()?
            .plugins
            .remove(name)
            .ok_or_else(|| PluginError::Invalid(format!("plugin `{name}` is not installed")))
    }

    fn marketplace_record(&self, name: &str) -> Result<MarketplaceRecord> {
        validate_name(name)?;
        self.marketplace_records()?
            .marketplaces
            .remove(name)
            .ok_or_else(|| PluginError::Invalid(format!("marketplace `{name}` is not registered")))
    }

    fn checked_install_path(&self, installed: &InstalledPlugin) -> Result<PathBuf> {
        validate_name(&installed.name)?;
        let root = self.state_root.join("plugins").canonicalize()?;
        let expected = root
            .join("store")
            .join(&installed.name)
            .join(digest_prefix(&installed.digest)?);
        let resolved = confined(&root, &expected)?;
        if resolved != installed.install_directory {
            return Err(PluginError::Invalid(
                "installed directory differs from its store record".into(),
            ));
        }
        Ok(resolved)
    }

    fn checked_marketplace_path(&self, record: &MarketplaceRecord) -> Result<PathBuf> {
        validate_name(&record.name)?;
        let root = self.state_root.join("plugins").canonicalize()?;
        let expected = root.join("marketplaces").join(&record.name);
        let resolved = confined(&root, &expected)?;
        if resolved != record.directory {
            return Err(PluginError::Invalid(
                "marketplace directory differs from its store record".into(),
            ));
        }
        Ok(resolved)
    }
}

fn digest_prefix(digest: &ContentDigest) -> Result<&str> {
    let hex = digest.as_hex();
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err(PluginError::Invalid(
            "invalid recorded content digest".into(),
        ));
    }
    Ok(&hex[..16])
}

fn source_name(source: &PluginSource) -> Option<String> {
    match source {
        PluginSource::Local { path } => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        PluginSource::Git {
            url, subdirectory, ..
        } => {
            let path = subdirectory.as_deref().unwrap_or(url).trim_end_matches('/');
            path.rsplit(['/', ':'])
                .next()
                .map(|name| name.trim_end_matches(".git").to_owned())
        }
        PluginSource::Marketplace { plugin, .. } => Some(plugin.clone()),
    }
}

fn ensure_directory(root: &Path, path: &Path) -> Result<PathBuf> {
    if path == root {
        return Ok(root.to_path_buf());
    }
    if let Some(parent) = path.parent() {
        ensure_directory(root, parent)?;
    }
    if !path.exists() {
        fs::create_dir(path)?;
    }
    let resolved = confined(root, path)?;
    if !resolved.is_dir() {
        return Err(PluginError::Invalid(
            "store path must be a directory".into(),
        ));
    }
    Ok(resolved)
}

fn read_record<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(toml::from_str(&text)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error.into()),
    }
}

fn atomic_record<T: Serialize>(path: &Path, record: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| PluginError::Invalid("record needs a parent directory".into()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(toml::to_string_pretty(record)?.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| PluginError::Io(error.error))?;
    Ok(())
}

struct StoreLock(fs::File);

// Failed transactions may create a parent before copying or recording fails.
// Only remove an initially absent, still-empty directory; successful version
// directories are therefore left alone.
struct EmptyDirectoryCleanup {
    path: PathBuf,
    initially_absent: bool,
}

impl EmptyDirectoryCleanup {
    fn new(path: &Path) -> Self {
        Self {
            path: path.into(),
            initially_absent: !path.exists(),
        }
    }
}

impl Drop for EmptyDirectoryCleanup {
    fn drop(&mut self) {
        if self.initially_absent {
            let _ = fs::remove_dir(&self.path);
        }
    }
}

impl StoreLock {
    fn acquire(root: &Path) -> Result<Self> {
        let path = root.join(".lock");
        if path.exists() {
            confined(root, &path)?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(Self(file))
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}
