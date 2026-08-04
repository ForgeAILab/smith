//! Which account each provider is currently using, remembered across sessions.
//!
//! Rotation is expensive to redo. A user who moved to their second account
//! because the first was spent should not be put back on the spent one by
//! restarting Smith, and should not have to re-answer the same prompt every
//! morning. So the choice is sticky, and it lives in user-scope state rather
//! than in a session journal: it is a property of the person and their
//! accounts, not of one conversation.
//!
//! The stored value is the credential **reference**, not the pool position.
//! Positions are not stable — editing `credentials` to add an account at the
//! top would silently repoint every remembered choice at a different account,
//! and the user would have no way to see that it happened. A reference that no
//! longer appears in the pool is simply forgotten, which falls back to the
//! declared first member.
//!
//! Nothing secret is written here. A credential reference names *where* a
//! secret lives; it is what the user typed into their own configuration file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use serde::{Deserialize, Serialize};

use crate::private_storage::{ensure_private_directory, read_private, write_private_atomically};

/// The file, beneath the user state root, holding remembered accounts.
const ACCOUNTS_FILE: &str = "accounts.json";

/// The largest accounts file Smith will read.
///
/// The content is a short map of provider names to references; anything
/// approaching this size is corruption or someone else's file, and reading it
/// into memory would be the wrong response either way.
const MAX_ACCOUNTS_BYTES: usize = 64 * 1024;

/// The on-disk shape, versioned so a later format can be recognized rather
/// than misread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AccountsFile {
    /// Schema revision of this file.
    #[serde(default = "current_revision")]
    revision: u32,
    /// Provider name → the credential reference last active for it.
    #[serde(default)]
    active: BTreeMap<String, String>,
}

fn current_revision() -> u32 {
    1
}

/// Written by hand rather than derived: a derived `Default` would zero the
/// revision, so a store that had never read a file would write revision 0 and
/// then refuse to read its own file back.
impl Default for AccountsFile {
    fn default() -> Self {
        Self {
            revision: current_revision(),
            active: BTreeMap::new(),
        }
    }
}

/// Remembered active accounts, one per provider.
#[derive(Debug, Clone)]
pub struct ActiveAccounts {
    path: PathBuf,
    file: AccountsFile,
}

impl ActiveAccounts {
    /// Loads remembered accounts from beneath `user_state`.
    ///
    /// A missing file is an empty memory, not an error: the first run of a
    /// pooled provider has nothing to remember yet. An *unreadable* or
    /// malformed file is also treated as empty rather than fatal — the worst
    /// case is starting on the declared first account, which is exactly what a
    /// user with no history gets, and refusing to start a session over a
    /// corrupted preference file would be a wildly disproportionate response.
    pub async fn load(user_state: impl AsRef<Path>) -> Self {
        let path = user_state.as_ref().join(ACCOUNTS_FILE);
        let file = match read_private(&path).await {
            Ok(Some(bytes)) if bytes.len() <= MAX_ACCOUNTS_BYTES => {
                serde_json::from_slice::<AccountsFile>(&bytes)
                    .ok()
                    .filter(|file| file.revision == current_revision())
                    .unwrap_or_default()
            }
            _ => AccountsFile::default(),
        };
        Self { path, file }
    }

    /// An in-memory store that persists nowhere, for tests and direct
    /// embedders that own their own state.
    pub fn ephemeral() -> Self {
        Self {
            path: PathBuf::new(),
            file: AccountsFile::default(),
        }
    }

    /// The reference last active for `provider`, if one is remembered.
    pub fn active(&self, provider: &str) -> Option<&str> {
        self.file.active.get(provider).map(String::as_str)
    }

    /// Records `reference` as the active account for `provider`.
    ///
    /// Returns whether anything changed, so a caller can skip a write when the
    /// session simply started on the account it was already using.
    pub fn remember(&mut self, provider: &str, reference: &str) -> bool {
        if self.file.active.get(provider).map(String::as_str) == Some(reference) {
            return false;
        }
        self.file
            .active
            .insert(provider.to_owned(), reference.to_owned());
        true
    }

    /// Resolves the remembered account for `provider` to a position in `pool`.
    ///
    /// Returns `None` when nothing is remembered, or when the remembered
    /// reference is no longer declared — a pool the user edited since. Both
    /// cases fall back to the declared first member, which is the same place a
    /// first-ever run starts.
    pub fn position_in(&self, provider: &str, pool: &[String]) -> Option<usize> {
        let remembered = self.active(provider)?;
        pool.iter().position(|reference| reference == remembered)
    }

    /// Writes the remembered accounts, owner-only and atomically.
    ///
    /// A store built by [`ActiveAccounts::ephemeral`] writes nothing.
    pub async fn save(&self) -> Result<(), RuntimeError> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        let Some(directory) = self.path.parent() else {
            return Ok(());
        };
        ensure_private_directory(directory).await?;
        let bytes = serde_json::to_vec_pretty(&self.file).map_err(|error| {
            RuntimeError::new(
                ErrorKind::Serialization,
                format!("the remembered accounts could not be encoded: {error}"),
            )
        })?;
        write_private_atomically(&self.path, &bytes).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> Vec<String> {
        vec![
            "keychain:smith/personal".to_owned(),
            "keychain:smith/work".to_owned(),
        ]
    }

    #[tokio::test]
    async fn nothing_is_remembered_before_a_first_choice() {
        let store = ActiveAccounts::ephemeral();
        assert_eq!(store.active("acme"), None);
        // No memory falls back to the declared first member.
        assert_eq!(store.position_in("acme", &pool()), None);
    }

    #[tokio::test]
    async fn a_remembered_reference_resolves_to_its_position() {
        let mut store = ActiveAccounts::ephemeral();
        assert!(store.remember("acme", "keychain:smith/work"));
        assert_eq!(store.position_in("acme", &pool()), Some(1));
    }

    #[tokio::test]
    async fn remembering_the_same_account_twice_reports_no_change() {
        let mut store = ActiveAccounts::ephemeral();
        assert!(store.remember("acme", "keychain:smith/work"));
        // The caller uses this to skip a pointless write on every session.
        assert!(!store.remember("acme", "keychain:smith/work"));
        assert!(store.remember("acme", "keychain:smith/personal"));
    }

    #[tokio::test]
    async fn a_reordered_pool_keeps_the_same_account_rather_than_the_same_slot() {
        let mut store = ActiveAccounts::ephemeral();
        store.remember("acme", "keychain:smith/work");

        // The user adds an account at the top of the list.
        let reordered = vec![
            "keychain:smith/new".to_owned(),
            "keychain:smith/personal".to_owned(),
            "keychain:smith/work".to_owned(),
        ];
        // Position 1 now means a different account; the reference does not.
        assert_eq!(store.position_in("acme", &reordered), Some(2));
    }

    #[tokio::test]
    async fn a_removed_account_is_forgotten_rather_than_guessed() {
        let mut store = ActiveAccounts::ephemeral();
        store.remember("acme", "keychain:smith/retired");
        assert_eq!(store.position_in("acme", &pool()), None);
    }

    #[tokio::test]
    async fn accounts_are_scoped_per_provider() {
        let mut store = ActiveAccounts::ephemeral();
        store.remember("acme", "keychain:smith/work");
        store.remember("other", "keychain:smith/personal");
        assert_eq!(store.active("acme"), Some("keychain:smith/work"));
        assert_eq!(store.active("other"), Some("keychain:smith/personal"));
    }

    #[tokio::test]
    async fn a_saved_choice_survives_a_reload() {
        let root = tempfile::tempdir().expect("a user state root");

        let mut store = ActiveAccounts::load(root.path()).await;
        store.remember("acme", "keychain:smith/work");
        store.save().await.expect("the accounts file was written");

        let reloaded = ActiveAccounts::load(root.path()).await;
        assert_eq!(reloaded.active("acme"), Some("keychain:smith/work"));
        assert_eq!(reloaded.position_in("acme", &pool()), Some(1));
    }

    #[tokio::test]
    async fn a_missing_file_loads_as_an_empty_memory() {
        let root = tempfile::tempdir().expect("a user state root");
        let store = ActiveAccounts::load(root.path()).await;
        assert_eq!(store.active("acme"), None);
    }

    #[tokio::test]
    async fn a_corrupt_file_loads_as_empty_rather_than_failing_the_session() {
        let root = tempfile::tempdir().expect("a user state root");
        std::fs::write(root.path().join(ACCOUNTS_FILE), b"{not json").expect("a corrupt file");

        // Starting on the declared first account is the same outcome a new
        // user gets; refusing to start would be wildly disproportionate.
        let store = ActiveAccounts::load(root.path()).await;
        assert_eq!(store.active("acme"), None);
    }

    #[tokio::test]
    async fn a_future_revision_is_not_misread_as_this_one() {
        let root = tempfile::tempdir().expect("a user state root");
        std::fs::write(
            root.path().join(ACCOUNTS_FILE),
            br#"{"revision":99,"active":{"acme":"keychain:smith/work"}}"#,
        )
        .expect("a newer file");

        let store = ActiveAccounts::load(root.path()).await;
        assert_eq!(store.active("acme"), None);
    }

    #[tokio::test]
    async fn an_ephemeral_store_writes_nothing() {
        let mut store = ActiveAccounts::ephemeral();
        store.remember("acme", "keychain:smith/work");
        store.save().await.expect("an ephemeral save is a no-op");
    }
}
