//! User-scoped credential references.
//!
//! Smith configuration records *where* a secret lives, never the secret. A
//! provider block carries `credential = "keychain:smith/acme"`; the value
//! behind it is fetched once, at provider construction, and lives only inside
//! [`Secret`], which redacts itself in `Debug`, `Display`, and therefore in
//! every event, log line, and diagnostic that formats it.
//!
//! Four reference forms, in the order a user should reach for them:
//!
//! ```text
//! keychain:<service>/<account>   macOS Keychain, Linux Secret Service
//! authfile:<entry>               Smith's owner-only plaintext auth.json
//! env:<VAR>                      a process environment variable
//! file:<path>                    encrypted file under ~/.smith, externally keyed
//! ```
//!
//! An unprefixed value is rejected rather than guessed at: a configuration
//! file holding a bare key has already leaked it into the repository, and the
//! only useful response is to say so.
//!
//! # No error in this module echoes what it was given
//!
//! A rejected string may *be* the secret, so a diagnostic that quotes it
//! defeats the point of rejecting it. Parse failures therefore describe the
//! accepted forms and nothing else — not even the text before the first colon,
//! which for a pasted `sk-proj:…` key would be a fragment of the key.
//! Resolution failures may name the reference (`env:ACME_API_KEY`), because a
//! locator that parsed is a location, not a value.
//!
//! # The synchronous backend, and where it blocks
//!
//! `keyring` is synchronous while [`SecretStore::resolve`] is async, and this
//! crate deliberately does not depend on `tokio`: configuration must be usable
//! from a plain `main` before any runtime exists. So the real work lives in
//! [`CredentialResolver::resolve_blocking`], and the async impl calls it on
//! the caller's thread.
//!
//! That is sound for the one place Smith resolves credentials — startup,
//! before the terminal is entered, once per provider — and unsound for
//! anything on a turn's hot path. A caller that does own a runtime keeps the
//! resolver `Clone + Send + Sync + 'static` precisely so it can offload the
//! call itself:
//!
//! ```ignore
//! let secret = tokio::task::spawn_blocking(move || resolver.resolve_blocking(&reference)).await??;
//! ```

use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::store::{Secret, SecretStore};
use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::auth_file::{AuthFileBackend, AuthFileError, OwnerOnlyAuthFile};

/// Smith's user state root — `~/.smith`.
///
/// Sessions, trust decisions, monitor output, and credential material live
/// here rather than under a project, so none of it can be committed by
/// accident.
pub fn user_state_root() -> Result<PathBuf, RuntimeError> {
    dirs::home_dir()
        .map(|home| home.join(".smith"))
        .ok_or_else(|| {
            RuntimeError::config(
                "no home directory is available, so Smith cannot locate its user state root",
            )
        })
}

/// Where a credential lives, as configuration spells it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CredentialRef {
    /// An entry in the platform credential service.
    Keychain {
        /// The service the entry belongs to.
        service: String,
        /// The account within that service.
        account: String,
    },
    /// An entry in Smith's fixed owner-only plaintext `auth.json`.
    AuthFile {
        /// Stable product-owned entry name.
        entry: String,
    },
    /// A process environment variable.
    Env {
        /// The variable's name.
        variable: String,
    },
    /// A ciphertext file beneath the user state root, decrypted with key
    /// material kept outside it.
    File {
        /// The path, relative to the user state root.
        path: PathBuf,
    },
}

impl CredentialRef {
    /// Parses a scheme-prefixed reference.
    ///
    /// # Errors
    ///
    /// Returns a [`CredentialRefError`] whose message describes the accepted
    /// forms without repeating `value`, which may be a secret.
    pub fn parse(value: &str) -> Result<Self, CredentialRefError> {
        let Some((scheme, locator)) = value.trim().split_once(':') else {
            return Err(CredentialRefError::Unprefixed);
        };
        match scheme {
            "keychain" => {
                let Some((service, account)) = locator.split_once('/') else {
                    return Err(CredentialRefError::Keychain);
                };
                // Exactly one separator, so the text form round-trips: a second
                // slash would reappear in a different place.
                if service.is_empty() || account.is_empty() || account.contains('/') {
                    return Err(CredentialRefError::Keychain);
                }
                Ok(Self::Keychain {
                    service: service.to_owned(),
                    account: account.to_owned(),
                })
            }
            "authfile" => {
                if !is_auth_file_entry(locator) {
                    return Err(CredentialRefError::AuthFile);
                }
                Ok(Self::AuthFile {
                    entry: locator.to_owned(),
                })
            }
            "env" => {
                if !is_variable_name(locator) {
                    return Err(CredentialRefError::Env);
                }
                Ok(Self::Env {
                    variable: locator.to_owned(),
                })
            }
            "file" => {
                // Absolute paths and `..` are refused here rather than at
                // resolution, so a reference can never *describe* credential
                // material outside user state — a project-supplied config
                // cannot aim the fallback at a file it controls.
                let path = Path::new(locator);
                if locator.is_empty()
                    || path
                        .components()
                        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
                {
                    return Err(CredentialRefError::File);
                }
                Ok(Self::File {
                    path: path.to_path_buf(),
                })
            }
            _ => Err(CredentialRefError::UnknownScheme),
        }
    }
}

/// Whether `name` is usable as an environment variable name.
fn is_variable_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|first: char| first.is_ascii_digit())
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn is_auth_file_entry(entry: &str) -> bool {
    !entry.is_empty()
        && entry.len() <= 64
        && entry
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
}

impl fmt::Display for CredentialRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keychain { service, account } => write!(f, "keychain:{service}/{account}"),
            Self::AuthFile { entry } => write!(f, "authfile:{entry}"),
            Self::Env { variable } => write!(f, "env:{variable}"),
            Self::File { path } => write!(f, "file:{}", path.display()),
        }
    }
}

impl FromStr for CredentialRef {
    type Err = CredentialRefError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for CredentialRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CredentialRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        // `custom` rather than `invalid_value`, which would put the rejected
        // string — possibly a plaintext key — into the message.
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// Why a configured string is not a usable credential reference.
///
/// Every message is a fixed string. None of them repeats the rejected input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialRefError {
    /// The value has no `scheme:` prefix, so it is a value rather than a
    /// reference.
    #[error(
        "a credential must be a reference, not a key: write \
         `keychain:<service>/<account>`, `authfile:<entry>`, `env:<VAR>`, or `file:<path>`"
    )]
    Unprefixed,
    /// The prefix is not one Smith knows.
    #[error("unknown credential scheme: expected `keychain:`, `authfile:`, `env:`, or `file:`")]
    UnknownScheme,
    /// A `keychain:` reference does not name one service and one account.
    #[error("a keychain credential must be written `keychain:<service>/<account>`")]
    Keychain,
    /// An `authfile:` reference does not name a bounded product entry.
    #[error("an auth-file credential must name a product entry, as `authfile:chatgpt`")]
    AuthFile,
    /// An `env:` reference does not name a variable.
    #[error("an environment credential must name a variable, as `env:ACME_API_KEY`")]
    Env,
    /// A `file:` reference is empty, absolute, or climbs out of user state.
    #[error(
        "an encrypted-file credential must name a path inside `~/.smith`, \
         as `file:credentials/acme.enc`"
    )]
    File,
}

/// The platform credential service, as this crate needs it.
///
/// Injectable so that a test never opens the developer's login keychain: the
/// production implementation is [`OsKeychain`], and everything else is a stand-in.
pub trait Keychain: Send + Sync {
    /// Reads the secret stored under `service` and `account`.
    ///
    /// # Errors
    ///
    /// Returns why the service could not produce it — absent entry, refused
    /// access, or no service at all.
    fn secret(&self, service: &str, account: &str) -> Result<Secret, KeychainError>;
}

/// Why a credential service did not produce a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeychainError {
    /// The service is reachable and holds no such entry.
    #[error("no entry is stored")]
    Missing,
    /// The service exists and refused access: a locked keychain, a denied
    /// prompt, a session without an unlocked collection.
    #[error("the credential service refused access ({0})")]
    Denied(String),
    /// No credential service is usable in this session.
    #[error("no credential service is available ({0})")]
    Unavailable(String),
    /// Several stored credentials match the reference, so none of them is
    /// unambiguously the one meant.
    #[error("more than one stored credential matches")]
    Ambiguous,
    /// The entry exists but is not a text secret.
    #[error("the stored entry is not text")]
    Unusable,
    /// The platform will not address an entry by this service or account.
    #[error("the credential service rejected the reference ({0})")]
    Rejected(String),
}

/// The platform credential service: macOS Keychain, Linux Secret Service.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeychain;

impl Keychain for OsKeychain {
    fn secret(&self, service: &str, account: &str) -> Result<Secret, KeychainError> {
        let entry = keyring::Entry::new(service, account).map_err(from_keyring)?;
        entry.get_password().map(Secret::new).map_err(from_keyring)
    }
}

/// The minimal credential-service operations guided setup needs.
///
/// This is separate from the runtime-facing read-only [`Keychain`] boundary.
/// Its read is used only to capture the value that an interrupted setup must
/// restore; values remain wrapped in [`Secret`] throughout.
pub trait CredentialEnrollmentBackend: Send + Sync {
    /// Reads the prior value for transactional restore.
    fn prior(&self, service: &str, account: &str) -> Result<Option<Secret>, KeychainError>;

    /// Stores a replacement value.
    fn store(&self, service: &str, account: &str, secret: &Secret) -> Result<(), KeychainError>;

    /// Removes the entry.
    fn remove(&self, service: &str, account: &str) -> Result<(), KeychainError>;
}

/// Enrollment backend for macOS Keychain or Linux Secret Service.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsCredentialEnrollmentBackend;

impl CredentialEnrollmentBackend for OsCredentialEnrollmentBackend {
    fn prior(&self, service: &str, account: &str) -> Result<Option<Secret>, KeychainError> {
        let entry = keyring::Entry::new(service, account).map_err(from_keyring)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(from_keyring(error)),
        }
    }

    fn store(&self, service: &str, account: &str, secret: &Secret) -> Result<(), KeychainError> {
        let entry = keyring::Entry::new(service, account).map_err(from_keyring)?;
        entry.set_password(secret.expose()).map_err(from_keyring)
    }

    fn remove(&self, service: &str, account: &str) -> Result<(), KeychainError> {
        let entry = keyring::Entry::new(service, account).map_err(from_keyring)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(from_keyring(error)),
        }
    }
}

/// A credential change retained until setup preflight accepts or restores it.
pub struct EnrollmentReceipt {
    reference: CredentialRef,
    prior: Option<Secret>,
}

impl EnrollmentReceipt {
    /// The non-secret reference setup records in configuration.
    pub fn reference(&self) -> &CredentialRef {
        &self.reference
    }
}

impl fmt::Debug for EnrollmentReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnrollmentReceipt")
            .field("reference", &self.reference)
            .field("had_prior_value", &self.prior.is_some())
            .finish()
    }
}

/// Injectable, reversible setup-time credential enrollment.
#[derive(Clone)]
pub struct CredentialEnroller {
    keychain: Arc<dyn CredentialEnrollmentBackend>,
    auth_file: Arc<dyn AuthFileBackend>,
}

impl CredentialEnroller {
    /// Uses the operating-system credential service for `keychain:` and
    /// Smith's fixed owner-only file for `authfile:`.
    pub fn new() -> Self {
        Self {
            keychain: Arc::new(OsCredentialEnrollmentBackend),
            auth_file: Arc::new(OwnerOnlyAuthFile::discover()),
        }
    }

    /// Uses an injected keychain backend and the production auth-file backend.
    /// Existing API-key tests use this without touching the auth-file path.
    pub fn with_backend(backend: Arc<dyn CredentialEnrollmentBackend>) -> Self {
        Self {
            keychain: backend,
            auth_file: Arc::new(OwnerOnlyAuthFile::discover()),
        }
    }

    /// Uses injected keychain and auth-file backends.
    ///
    /// This is the lifecycle-test seam that proves ChatGPT never calls the
    /// keychain backend.
    pub fn with_backends(
        keychain: Arc<dyn CredentialEnrollmentBackend>,
        auth_file: Arc<dyn AuthFileBackend>,
    ) -> Self {
        Self {
            keychain,
            auth_file,
        }
    }

    /// Stores `secret` at a reviewed Smith-managed reference.
    ///
    /// Environment references are configuration-only and are never read or
    /// copied by enrollment.
    pub fn enroll(
        &self,
        reference: &CredentialRef,
        secret: &Secret,
    ) -> Result<EnrollmentReceipt, CredentialEnrollmentError> {
        let prior = match reference {
            CredentialRef::Keychain { service, account } => {
                let prior = self.keychain.prior(service, account).map_err(|cause| {
                    CredentialEnrollmentError::Backend {
                        reference: reference.clone(),
                        operation: EnrollmentOperation::ReadPrior,
                        cause,
                    }
                })?;
                self.keychain
                    .store(service, account, secret)
                    .map_err(|cause| CredentialEnrollmentError::Backend {
                        reference: reference.clone(),
                        operation: EnrollmentOperation::Store,
                        cause,
                    })?;
                prior
            }
            CredentialRef::AuthFile { entry } => {
                self.auth_file.replace(entry, secret).map_err(|cause| {
                    CredentialEnrollmentError::AuthFile {
                        reference: reference.clone(),
                        operation: EnrollmentOperation::Store,
                        cause,
                    }
                })?
            }
            CredentialRef::Env { .. } | CredentialRef::File { .. } => {
                return Err(CredentialEnrollmentError::NotStored {
                    reference: reference.clone(),
                });
            }
        };
        Ok(EnrollmentReceipt {
            reference: reference.clone(),
            prior,
        })
    }

    /// Restores the value captured before [`Self::enroll`].
    pub fn restore(&self, receipt: EnrollmentReceipt) -> Result<(), CredentialEnrollmentError> {
        match &receipt.reference {
            CredentialRef::Keychain { service, account } => match &receipt.prior {
                Some(prior) => self.keychain.store(service, account, prior),
                None => self.keychain.remove(service, account),
            }
            .map_err(|cause| CredentialEnrollmentError::Backend {
                reference: receipt.reference,
                operation: EnrollmentOperation::Restore,
                cause,
            }),
            CredentialRef::AuthFile { entry } => match &receipt.prior {
                Some(prior) => self.auth_file.store(entry, prior),
                None => self.auth_file.remove(entry),
            }
            .map_err(|cause| CredentialEnrollmentError::AuthFile {
                reference: receipt.reference,
                operation: EnrollmentOperation::Restore,
                cause,
            }),
            CredentialRef::Env { .. } | CredentialRef::File { .. } => {
                Err(CredentialEnrollmentError::NotStored {
                    reference: receipt.reference,
                })
            }
        }
    }

    /// Removes an entry created for an abandoned setup when no receipt is
    /// available to restore.
    pub fn cleanup(&self, reference: &CredentialRef) -> Result<(), CredentialEnrollmentError> {
        match reference {
            CredentialRef::Keychain { service, account } => self
                .keychain
                .remove(service, account)
                .map_err(|cause| CredentialEnrollmentError::Backend {
                    reference: reference.clone(),
                    operation: EnrollmentOperation::Cleanup,
                    cause,
                }),
            CredentialRef::AuthFile { entry } => {
                self.auth_file
                    .remove(entry)
                    .map_err(|cause| CredentialEnrollmentError::AuthFile {
                        reference: reference.clone(),
                        operation: EnrollmentOperation::Cleanup,
                        cause,
                    })
            }
            CredentialRef::Env { .. } | CredentialRef::File { .. } => {
                Err(CredentialEnrollmentError::NotStored {
                    reference: reference.clone(),
                })
            }
        }
    }
}

impl Default for CredentialEnroller {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CredentialEnroller {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialEnroller")
            .finish_non_exhaustive()
    }
}

/// A stable setup-time credential operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentOperation {
    /// Capture the value an interrupted setup would restore.
    ReadPrior,
    /// Store the reviewed replacement.
    Store,
    /// Restore the captured prior state.
    Restore,
    /// Remove an abandoned new entry.
    Cleanup,
}

impl fmt::Display for EnrollmentOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ReadPrior => "read prior value",
            Self::Store => "store value",
            Self::Restore => "restore prior value",
            Self::Cleanup => "remove abandoned value",
        })
    }
}

/// Why setup could not enroll or restore a credential.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialEnrollmentError {
    /// The selected reference is managed externally rather than stored.
    #[error(
        "`{reference}` is externally managed; enrollment stores only \
         `keychain:<service>/<account>` and `authfile:<entry>` references"
    )]
    NotStored {
        /// Reviewed reference.
        reference: CredentialRef,
    },
    /// The credential service failed.
    #[error("could not {operation} at `{reference}`: {cause}")]
    Backend {
        /// Reviewed reference.
        reference: CredentialRef,
        /// Operation that failed.
        operation: EnrollmentOperation,
        /// Classified service failure.
        cause: KeychainError,
    },
    /// Smith's owner-only auth file failed.
    #[error("could not {operation} at `{reference}`: {cause}")]
    AuthFile {
        /// Reviewed reference.
        reference: CredentialRef,
        /// Operation that failed.
        operation: EnrollmentOperation,
        /// Fixed auth-file failure classification.
        cause: AuthFileError,
    },
}

impl CredentialEnrollmentError {
    /// Whether setup should keep the user at authentication and offer the
    /// environment-reference path.
    pub fn can_use_environment_instead(&self) -> bool {
        matches!(
            self,
            Self::Backend {
                cause: KeychainError::Denied(_) | KeychainError::Unavailable(_),
                ..
            }
        )
    }
}

/// The default setup keychain location for a provider.
pub fn setup_keychain_reference(provider: &str) -> Result<CredentialRef, CredentialRefError> {
    CredentialRef::parse(&format!("keychain:smith/{provider}"))
}

/// An externally managed environment credential reference.
///
/// This validates only the variable name and never reads its value.
pub fn setup_environment_reference(variable: &str) -> Result<CredentialRef, CredentialRefError> {
    CredentialRef::parse(&format!("env:{variable}"))
}

/// Classifies a `keyring` failure without carrying any of its payloads.
///
/// `BadEncoding` holds the undecodable bytes and `Ambiguous` holds the matching
/// credentials; both are dropped here rather than formatted, so nothing from
/// the store can reach a message.
fn from_keyring(err: keyring::Error) -> KeychainError {
    match err {
        keyring::Error::NoEntry => KeychainError::Missing,
        keyring::Error::NoStorageAccess(cause) => KeychainError::Denied(cause.to_string()),
        keyring::Error::PlatformFailure(cause) => KeychainError::Unavailable(cause.to_string()),
        keyring::Error::BadEncoding(_) => KeychainError::Unusable,
        keyring::Error::Ambiguous(_) => KeychainError::Ambiguous,
        keyring::Error::TooLong(attribute, limit) => KeychainError::Rejected(format!(
            "`{attribute}` is longer than the platform limit of {limit} characters"
        )),
        keyring::Error::Invalid(attribute, reason) => {
            KeychainError::Rejected(format!("`{attribute}` is invalid: {reason}"))
        }
        // `keyring::Error` is `#[non_exhaustive]`; a variant added later is
        // still a failure of the service, never a leak of our value.
        other => KeychainError::Unavailable(other.to_string()),
    }
}

/// A read-only view of the process environment.
///
/// Injectable for the same reason as [`Keychain`], and because a test cannot
/// set a variable in a crate that forbids `unsafe`.
pub trait Environment: Send + Sync {
    /// The value of `name`, if it is set and valid UTF-8.
    fn value(&self, name: &str) -> Option<Secret>;
}

/// The real process environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn value(&self, name: &str) -> Option<Secret> {
        std::env::var(name).ok().map(Secret::new)
    }
}

/// Turns credential references into secrets.
#[derive(Clone)]
pub struct CredentialResolver {
    keychain: Arc<dyn Keychain>,
    auth_file: Arc<dyn AuthFileBackend>,
    environment: Arc<dyn Environment>,
    user_state: PathBuf,
}

impl CredentialResolver {
    /// Resolves against the platform credential service and the process
    /// environment, confining the encrypted-file fallback to `user_state`.
    pub fn new(user_state: impl Into<PathBuf>) -> Self {
        let user_state = user_state.into();
        Self {
            keychain: Arc::new(OsKeychain),
            auth_file: Arc::new(OwnerOnlyAuthFile::new(&user_state)),
            environment: Arc::new(ProcessEnvironment),
            user_state,
        }
    }

    /// Resolves with the fallback confined to `~/.smith`.
    ///
    /// # Errors
    ///
    /// Fails when there is no home directory to root user state in.
    pub fn discover() -> Result<Self, RuntimeError> {
        Ok(Self::new(user_state_root()?))
    }

    /// Replaces the credential service.
    #[must_use]
    pub fn with_keychain(mut self, keychain: Arc<dyn Keychain>) -> Self {
        self.keychain = keychain;
        self
    }

    /// Replaces Smith's owner-only auth-file backend.
    #[must_use]
    pub fn with_auth_file(mut self, auth_file: Arc<dyn AuthFileBackend>) -> Self {
        self.auth_file = auth_file;
        self
    }

    /// Replaces the environment.
    #[must_use]
    pub fn with_environment(mut self, environment: Arc<dyn Environment>) -> Self {
        self.environment = environment;
        self
    }

    /// The root the encrypted-file fallback is confined to.
    pub fn user_state(&self) -> &Path {
        &self.user_state
    }

    /// Resolves `reference` on the calling thread.
    ///
    /// This is the whole implementation; the [`SecretStore`] impl is a wrapper.
    /// It blocks for as long as the platform takes to answer, which can include
    /// an unlock prompt — see the module documentation for where that is
    /// acceptable.
    ///
    /// # Errors
    ///
    /// Returns which reference failed and why, never what it holds.
    pub fn resolve_blocking(&self, reference: &CredentialRef) -> Result<Secret, CredentialError> {
        match reference {
            CredentialRef::Keychain { service, account } => self
                .keychain
                .secret(service, account)
                .map_err(|cause| match cause {
                    KeychainError::Missing => CredentialError::Missing {
                        reference: reference.clone(),
                    },
                    cause => CredentialError::Backend {
                        reference: reference.clone(),
                        cause,
                    },
                }),
            CredentialRef::AuthFile { entry } => self
                .auth_file
                .read(entry)
                .map_err(|cause| CredentialError::AuthFileBackend {
                    reference: reference.clone(),
                    cause,
                })?
                .ok_or_else(|| CredentialError::Missing {
                    reference: reference.clone(),
                }),
            CredentialRef::Env { variable } => {
                self.environment
                    .value(variable)
                    .ok_or_else(|| CredentialError::Missing {
                        reference: reference.clone(),
                    })
            }
            CredentialRef::File { path } => Err(self.encrypted_file(reference, path)),
        }
    }

    /// Diagnoses an encrypted-file reference.
    ///
    /// The checks run in the order the user can act on them: a misplaced file
    /// is a configuration mistake, a missing one is a setup step, a readable
    /// one is a live exposure, and only then does the absent cipher matter.
    fn encrypted_file(&self, reference: &CredentialRef, path: &Path) -> CredentialError {
        // A canonical root keeps the containment check honest where `~` sits
        // behind a symlink, as `/tmp` does on macOS.
        let root = self
            .user_state
            .canonicalize()
            .unwrap_or_else(|_| self.user_state.clone());
        // An absolute `path` replaces the root here rather than extending it,
        // which the containment check then catches.
        let candidate = root.join(path);
        let outside = || CredentialError::OutsideUserState {
            reference: reference.clone(),
            root: root.display().to_string(),
        };
        if !within(&root, &candidate) {
            return outside();
        }

        let Ok(metadata) = std::fs::metadata(&candidate) else {
            return CredentialError::Missing {
                reference: reference.clone(),
            };
        };
        // Now that the file exists, resolve it: a symlink placed inside the
        // root can still point out of it.
        if !candidate
            .canonicalize()
            .is_ok_and(|real| real.starts_with(&root))
        {
            return outside();
        }
        if group_or_world_readable(&metadata) {
            return CredentialError::Exposed {
                reference: reference.clone(),
            };
        }

        CredentialError::Unavailable {
            reference: reference.clone(),
        }
    }
}

impl fmt::Debug for CredentialResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The backends are omitted deliberately. This type is printed in
        // configuration diagnostics, and an injected backend may hold key
        // material that a derived `Debug` would spill into them.
        f.debug_struct("CredentialResolver")
            .field("user_state", &self.user_state)
            .finish_non_exhaustive()
    }
}

/// Whether `candidate` stays inside `root` once `.` and `..` are applied.
///
/// Lexical, because the file need not exist yet; `..` pops rather than being
/// compared as a component, which is what stops `<root>/../etc/passwd` from
/// passing a plain prefix test.
fn within(root: &Path, candidate: &Path) -> bool {
    let mut resolved = PathBuf::new();
    for part in candidate.components() {
        match part {
            Component::ParentDir => {
                if !resolved.pop() {
                    return false;
                }
            }
            Component::CurDir => {}
            other => resolved.push(other),
        }
    }
    resolved.starts_with(root)
}

#[cfg(unix)]
fn group_or_world_readable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o077 != 0
}

#[cfg(not(unix))]
fn group_or_world_readable(_metadata: &std::fs::Metadata) -> bool {
    // Windows has no comparable mode bits, and Smith's fallback is not
    // supported there yet.
    false
}

/// Why a parsed reference did not produce a secret.
///
/// Each variant names the reference and the reason. None of them can carry a
/// value: the only payloads are locators, a root path, and a service failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    /// The configured string is not a reference at all.
    #[error(transparent)]
    Reference(#[from] CredentialRefError),
    /// The backend is working and holds nothing under this reference.
    #[error("`{reference}` resolves to nothing: no credential is stored there")]
    Missing {
        /// The reference that resolved to nothing.
        reference: CredentialRef,
    },
    /// The backend could not answer.
    #[error("`{reference}` could not be read: {cause}")]
    Backend {
        /// The reference that could not be read.
        reference: CredentialRef,
        /// What the credential service reported.
        cause: KeychainError,
    },
    /// Smith's owner-only auth file could not answer.
    #[error("`{reference}` could not be read: {cause}")]
    AuthFileBackend {
        /// The reference that could not be read.
        reference: CredentialRef,
        /// Fixed auth-file failure classification.
        cause: AuthFileError,
    },
    /// The reference points outside the user state root.
    #[error("`{reference}` resolves outside `{root}`: credential material never lives elsewhere")]
    OutsideUserState {
        /// The offending reference.
        reference: CredentialRef,
        /// The root it had to stay inside.
        root: String,
    },
    /// The ciphertext file is readable by users other than its owner.
    #[error("`{reference}` is readable by other users: restrict it with `chmod 600` and rotate it")]
    Exposed {
        /// The exposed reference.
        reference: CredentialRef,
    },
    /// The encrypted-file fallback has no cipher to apply.
    #[error(
        "`{reference}` cannot be decrypted: Smith has no encrypted-file cipher yet, so its \
         external key source cannot be used — store this credential with \
         `keychain:<service>/<account>` or supply it as `env:<VAR>` instead"
    )]
    Unavailable {
        /// The reference that cannot be decrypted.
        reference: CredentialRef,
    },
}

impl From<CredentialError> for RuntimeError {
    fn from(err: CredentialError) -> Self {
        // Everything here is fixed by the user, in configuration or in their
        // credential service, and `Config` is the runtime's kind for that. Only
        // a genuinely absent entry is a lookup miss.
        let kind = match err {
            CredentialError::Missing { .. } => ErrorKind::NotFound,
            _ => ErrorKind::Config,
        };
        RuntimeError::new(kind, err.to_string())
    }
}

#[async_trait]
impl SecretStore for CredentialResolver {
    /// Resolves the reference spelled by `key`.
    ///
    /// `Ok(None)` means the backend answered and holds nothing, which the trait
    /// treats as absence rather than failure; callers wanting the precise
    /// "`env:ACME_API_KEY` resolves to nothing" wording should use
    /// [`CredentialResolver::resolve_blocking`].
    ///
    /// Blocks the calling task; see the module documentation.
    async fn resolve(&self, key: &str) -> Result<Option<Secret>, RuntimeError> {
        let reference = CredentialRef::parse(key).map_err(CredentialError::from)?;
        match self.resolve_blocking(&reference) {
            Ok(secret) => Ok(Some(secret)),
            Err(CredentialError::Missing { .. }) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_round_trips_through_its_configured_text() {
        for text in [
            "keychain:smith/acme",
            "authfile:chatgpt",
            "env:ACME_API_KEY",
            "file:credentials/acme.enc",
        ] {
            let reference = CredentialRef::parse(text).expect("a reference");
            assert_eq!(reference.to_string(), text);
            assert_eq!(
                text.parse::<CredentialRef>().expect("a reference"),
                reference
            );
        }
    }

    #[test]
    fn surrounding_whitespace_does_not_change_a_reference() {
        assert_eq!(
            CredentialRef::parse("  env:ACME_API_KEY \t").expect("a reference"),
            CredentialRef::Env {
                variable: "ACME_API_KEY".to_owned(),
            }
        );
    }

    #[test]
    fn a_keychain_reference_takes_exactly_one_service_and_one_account() {
        assert_eq!(
            CredentialRef::parse("keychain:smith/acme"),
            Ok(CredentialRef::Keychain {
                service: "smith".to_owned(),
                account: "acme".to_owned(),
            })
        );
        for rejected in ["keychain:smith", "keychain:/acme", "keychain:smith/", ""] {
            assert!(CredentialRef::parse(rejected).is_err(), "{rejected}");
        }
        assert_eq!(
            CredentialRef::parse("keychain:smith/acme/extra"),
            Err(CredentialRefError::Keychain)
        );
    }

    #[test]
    fn a_file_reference_cannot_leave_the_user_state_root() {
        for rejected in ["file:", "file:/etc/passwd", "file:../../etc/passwd"] {
            assert_eq!(
                CredentialRef::parse(rejected),
                Err(CredentialRefError::File)
            );
        }
    }

    #[test]
    fn an_environment_reference_takes_a_variable_name() {
        assert!(CredentialRef::parse("env:ACME_API_KEY").is_ok());
        for rejected in ["env:", "env:1KEY", "env:ACME KEY", "env:ACME-KEY"] {
            assert_eq!(CredentialRef::parse(rejected), Err(CredentialRefError::Env));
        }
    }

    #[test]
    fn an_auth_file_reference_takes_a_bounded_product_entry() {
        assert_eq!(
            CredentialRef::parse("authfile:chatgpt"),
            Ok(CredentialRef::AuthFile {
                entry: "chatgpt".to_owned(),
            })
        );
        for rejected in ["authfile:", "authfile:../chatgpt", "authfile:chat/gpt"] {
            assert_eq!(
                CredentialRef::parse(rejected),
                Err(CredentialRefError::AuthFile)
            );
        }
    }

    #[test]
    fn a_reference_serializes_as_the_string_configuration_holds() {
        let reference = CredentialRef::parse("keychain:smith/acme").expect("a reference");
        let json = serde_json::to_string(&reference).expect("serialized");
        assert_eq!(json, "\"keychain:smith/acme\"");
        assert_eq!(
            serde_json::from_str::<CredentialRef>(&json).expect("deserialized"),
            reference
        );
    }

    #[test]
    fn a_plaintext_key_is_refused_by_deserialization_without_being_quoted() {
        let err = serde_json::from_str::<CredentialRef>("\"sk-live-abcdef0123456789\"")
            .expect_err("a plaintext key must not deserialize");
        assert!(!err.to_string().contains("abcdef0123456789"), "{err}");
    }
}
