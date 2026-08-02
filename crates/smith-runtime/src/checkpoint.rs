//! Smith's protected implementation of Agent Runtime turn checkpoints.
//!
//! The runtime checkpoint contains exact provider requests, prepared tool
//! arguments, interaction answers, and committed results. It is intentionally
//! not written to Smith's redacted JSONL journal. This store encrypts the exact
//! serialized [`TurnCheckpoint`] with XChaCha20-Poly1305, keeps the 256-bit
//! user key in the operating-system credential service, and atomically replaces
//! one owner-only latest-checkpoint record per session.
//!
//! The clear envelope carries only schema, algorithm, session/turn identity,
//! and a fresh 192-bit nonce. All of it, plus the project identity, is bound as
//! authenticated additional data. Moving a record across projects or sessions,
//! editing its header, using another key, or corrupting ciphertext therefore
//! produces the same non-secret protected-state diagnostic.

use std::fmt;
use std::sync::Arc;

use agent_runtime_core::checkpoint::{
    CHECKPOINT_SCHEMA_VERSION, CheckpointStore, TurnCheckpoint, TurnState,
};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::store::Secret;
use async_trait::async_trait;
use chacha20poly1305::aead::{Aead, Generate, Payload};
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use smith_config::credential::{CredentialRef, CredentialResolver};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::private_storage::acquire_private_lock_blocking;
use crate::private_storage::{
    acquire_private_lock, read_private_bounded, write_private_atomically,
};
use crate::session::SessionPaths;

/// Version of Smith's encrypted outer envelope.
pub const PROTECTED_CHECKPOINT_SCHEMA_VERSION: u32 = 1;

/// Largest exact checkpoint Smith accepts before authenticated encryption.
pub const MAX_CHECKPOINT_PLAINTEXT_BYTES: usize = 32 * 1024 * 1024;

/// Fixed operating-system credential-service address for the per-user key.
pub const CHECKPOINT_KEY_SERVICE: &str = "dev.smith-agent.checkpoints";
/// Versioned account name so a future key format cannot reinterpret this key.
pub const CHECKPOINT_KEY_ACCOUNT: &str = "checkpoint-protection-v1";

const MAGIC: &[u8; 8] = b"SMITHCP1";
const ALGORITHM_XCHACHA20_POLY1305: u8 = 1;
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
const TAG_BYTES: usize = 16;
const LENGTH_BYTES: usize = 2;
const FIXED_HEADER_BYTES: usize = MAGIC.len() + 4 + 1 + LENGTH_BYTES + LENGTH_BYTES + NONCE_BYTES;
const MAX_ID_BYTES: usize = 4 * 1024;
const MAX_ENVELOPE_BYTES: usize =
    MAX_CHECKPOINT_PLAINTEXT_BYTES + FIXED_HEADER_BYTES + (MAX_ID_BYTES * 2) + TAG_BYTES;
const PROTECTED_STATE_DIAGNOSTIC: &str = "protected checkpoint state is unavailable or invalid";

/// A 256-bit checkpoint key retained in zeroizing memory.
///
/// The bytes are deliberately not serializable, printable, or exposed through
/// accessors. Injectable providers create this value so tests never open the
/// developer's login keychain.
pub struct CheckpointKey(Zeroizing<[u8; KEY_BYTES]>);

impl CheckpointKey {
    /// Wraps exact key bytes supplied by a protected backend.
    pub fn new(bytes: [u8; KEY_BYTES]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    fn from_secret(secret: Vec<u8>) -> Result<Self, CheckpointProtectionError> {
        let secret = Zeroizing::new(secret);
        if secret.len() != KEY_BYTES {
            return Err(CheckpointProtectionError::unavailable());
        }
        let mut bytes = Zeroizing::new([0; KEY_BYTES]);
        bytes.copy_from_slice(&secret);
        Ok(Self(bytes))
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        let key: &Key = self
            .0
            .as_slice()
            .try_into()
            .expect("CheckpointKey always contains exactly 32 bytes");
        XChaCha20Poly1305::new(key)
    }
}

impl fmt::Debug for CheckpointKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CheckpointKey([REDACTED])")
    }
}

/// Opaque failure to initialize protected checkpoint storage.
///
/// Keychain denial, missing platform support, invalid key bytes, and secure
/// entropy failure intentionally render alike. A caller may report that
/// mid-turn durability is unavailable, but cannot turn the error into a key or
/// corruption oracle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CheckpointProtectionError {
    _private: (),
}

impl CheckpointProtectionError {
    /// Creates the fail-closed protected-state diagnostic for an injected
    /// provider.
    pub fn unavailable() -> Self {
        Self { _private: () }
    }
}

impl fmt::Debug for CheckpointProtectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CheckpointProtectionError([REDACTED])")
    }
}

impl fmt::Display for CheckpointProtectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(PROTECTED_STATE_DIAGNOSTIC)
    }
}

impl std::error::Error for CheckpointProtectionError {}

impl From<CheckpointProtectionError> for RuntimeError {
    fn from(_: CheckpointProtectionError) -> Self {
        protected_state_error()
    }
}

/// Supplies one stable per-user key to a [`SmithCheckpointStore`].
pub trait CheckpointKeyProvider: Send + Sync + fmt::Debug {
    /// Loads the existing key or enrolls one when none exists.
    fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError>;
}

/// Explicit no-prompt checkpoint key resolved from owner-controlled config or
/// the process environment.
pub struct ConfiguredCheckpointKeyProvider {
    bytes: Zeroizing<[u8; KEY_BYTES]>,
}

impl ConfiguredCheckpointKeyProvider {
    /// Decodes a redaction-safe 64-character hexadecimal secret.
    pub fn new(secret: &Secret) -> Result<Self, CheckpointProtectionError> {
        decode_checkpoint_key(secret).map(|bytes| Self { bytes })
    }
}

impl fmt::Debug for ConfiguredCheckpointKeyProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConfiguredCheckpointKeyProvider([REDACTED])")
    }
}

impl CheckpointKeyProvider for ConfiguredCheckpointKeyProvider {
    fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
        let mut bytes = Zeroizing::new([0; KEY_BYTES]);
        bytes.copy_from_slice(self.bytes.as_slice());
        Ok(CheckpointKey(bytes))
    }
}

/// Explicit protected credential reference for a checkpoint key.
///
/// Resolution runs on the same blocking pool as the platform provider. The
/// selected reference is the only backend consulted; an `env:` reference, for
/// example, never constructs or calls the keychain path.
pub struct CredentialCheckpointKeyProvider {
    resolver: CredentialResolver,
    reference: CredentialRef,
}

impl CredentialCheckpointKeyProvider {
    /// Binds one already-validated resolver to an exact reference.
    pub fn new(
        resolver: CredentialResolver,
        reference: &str,
    ) -> Result<Self, CheckpointProtectionError> {
        let reference = CredentialRef::parse(reference)
            .map_err(|_| CheckpointProtectionError::unavailable())?;
        Ok(Self {
            resolver,
            reference,
        })
    }
}

impl fmt::Debug for CredentialCheckpointKeyProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialCheckpointKeyProvider")
            .field("reference", &self.reference)
            .finish_non_exhaustive()
    }
}

impl CheckpointKeyProvider for CredentialCheckpointKeyProvider {
    fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
        let secret = self
            .resolver
            .resolve_blocking(&self.reference)
            .map_err(|_| CheckpointProtectionError::unavailable())?;
        decode_checkpoint_key(&secret).map(CheckpointKey)
    }
}

fn decode_checkpoint_key(
    secret: &Secret,
) -> Result<Zeroizing<[u8; KEY_BYTES]>, CheckpointProtectionError> {
    let encoded = secret.expose().as_bytes();
    if encoded.len() != KEY_BYTES * 2 {
        return Err(CheckpointProtectionError::unavailable());
    }
    let mut bytes = Zeroizing::new([0; KEY_BYTES]);
    for (index, pair) in encoded.chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0]).ok_or_else(CheckpointProtectionError::unavailable)?;
        let low = hex_nibble(pair[1]).ok_or_else(CheckpointProtectionError::unavailable)?;
        bytes[index] = (high << 4) | low;
    }
    Ok(bytes)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// A host-owned durability boundary that must complete before a checkpoint is
/// published.
///
/// Smith's standard host uses this to flush every redacted journal event that
/// precedes the checkpoint watermark. Keeping the barrier separate from the
/// encrypted store lets deterministic embeddings omit the journal entirely
/// while preventing a standard host from claiming a durable event boundary
/// that is still only buffered in memory.
#[async_trait]
pub trait CheckpointBarrier: Send + Sync + fmt::Debug {
    /// Makes all state preceding `checkpoint` durable.
    async fn before_checkpoint(&self, checkpoint: &TurnCheckpoint) -> Result<(), RuntimeError>;
}

/// Production key provider backed by macOS Keychain or Linux Secret Service.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsCheckpointKeyProvider;

impl CheckpointKeyProvider for OsCheckpointKeyProvider {
    fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            return Err(CheckpointProtectionError::unavailable());
        }

        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let home = dirs::home_dir().ok_or_else(CheckpointProtectionError::unavailable)?;
            let _enrollment_guard =
                acquire_private_lock_blocking(&home.join(".smith/.checkpoint-key-v1.lock"))
                    .map_err(|_| CheckpointProtectionError::unavailable())?;
            let entry = keyring::Entry::new(CHECKPOINT_KEY_SERVICE, CHECKPOINT_KEY_ACCOUNT)
                .map_err(|_| CheckpointProtectionError::unavailable())?;
            match entry.get_secret() {
                Ok(secret) => CheckpointKey::from_secret(secret),
                Err(keyring::Error::NoEntry) => {
                    let generated = Zeroizing::new(
                        <[u8; KEY_BYTES]>::try_generate()
                            .map_err(|_| CheckpointProtectionError::unavailable())?,
                    );
                    entry
                        .set_secret(generated.as_slice())
                        .map_err(|_| CheckpointProtectionError::unavailable())?;

                    // The per-user advisory lease serializes the absent/read/
                    // create/read sequence across Smith processes. Re-reading
                    // uses the backend's canonical bytes and verifies length.
                    entry
                        .get_secret()
                        .map_err(|_| CheckpointProtectionError::unavailable())
                        .and_then(CheckpointKey::from_secret)
                }
                Err(_) => Err(CheckpointProtectionError::unavailable()),
            }
        }
    }
}

/// Project-scoped, authenticated latest-turn checkpoint storage.
pub struct SmithCheckpointStore {
    paths: SessionPaths,
    key: CheckpointKey,
    save_gate: Mutex<()>,
}

/// Deferred factory input for Smith's protected store.
///
/// The runtime factory resolves provider and credential configuration before
/// calling this setup, so an invalid run never prompts for checkpoint-key
/// access or creates protected state.
#[derive(Clone)]
pub struct SmithCheckpointSetup {
    paths: SessionPaths,
    provider: Arc<dyn CheckpointKeyProvider>,
}

impl SmithCheckpointSetup {
    /// Uses the operating-system credential service for `paths`.
    pub fn platform(paths: SessionPaths) -> Self {
        Self {
            paths,
            provider: Arc::new(OsCheckpointKeyProvider),
        }
    }

    /// Uses an injected key provider, primarily for deterministic hosts and
    /// tests that must never access the developer's credential service.
    pub fn with_provider(paths: SessionPaths, provider: Arc<dyn CheckpointKeyProvider>) -> Self {
        Self { paths, provider }
    }

    /// Initializes the exact runtime store.
    pub async fn initialize(&self) -> Result<Arc<dyn CheckpointStore>, CheckpointProtectionError> {
        SmithCheckpointStore::initialize_with(self.paths.clone(), self.provider.clone())
            .await
            .map(|store| Arc::new(store) as Arc<dyn CheckpointStore>)
    }
}

impl fmt::Debug for SmithCheckpointSetup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SmithCheckpointSetup")
            .field("paths", &self.paths)
            .field("provider", &self.provider)
            .finish()
    }
}

impl SmithCheckpointStore {
    /// Initializes a store with the operating-system credential service.
    pub async fn initialize(paths: SessionPaths) -> Result<Self, CheckpointProtectionError> {
        Self::initialize_with(paths, Arc::new(OsCheckpointKeyProvider)).await
    }

    /// Initializes a store through an injected protected-key provider.
    ///
    /// The provider is synchronous because platform keyring APIs are
    /// synchronous; it is always run on Tokio's blocking pool.
    pub async fn initialize_with(
        paths: SessionPaths,
        provider: Arc<dyn CheckpointKeyProvider>,
    ) -> Result<Self, CheckpointProtectionError> {
        if !cfg!(any(target_os = "macos", target_os = "linux")) {
            return Err(CheckpointProtectionError::unavailable());
        }
        let key = tokio::task::spawn_blocking(move || provider.load_or_create())
            .await
            .map_err(|_| CheckpointProtectionError::unavailable())??;
        Ok(Self {
            paths,
            key,
            save_gate: Mutex::new(()),
        })
    }

    /// The project-scoped paths this store uses.
    pub fn paths(&self) -> &SessionPaths {
        &self.paths
    }

    async fn load_latest_inner(
        &self,
        session: &SessionId,
    ) -> Result<Option<TurnCheckpoint>, RuntimeError> {
        let path = self.paths.checkpoint(session)?;
        let Some(envelope) = read_private_bounded(&path, MAX_ENVELOPE_BYTES).await? else {
            return Ok(None);
        };
        let parsed = parse_envelope(&envelope)?;
        if parsed.session != session.as_str() {
            return Err(protected_state_error());
        }
        let aad = authenticated_data(self.paths.project().as_str(), parsed.header);
        let plaintext = Zeroizing::new(
            self.key
                .cipher()
                .decrypt(
                    parsed.nonce,
                    Payload {
                        msg: parsed.ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| protected_state_error())?,
        );
        if plaintext.len() > MAX_CHECKPOINT_PLAINTEXT_BYTES {
            return Err(protected_state_error());
        }
        let checkpoint: TurnCheckpoint =
            serde_json::from_slice(&plaintext).map_err(|_| protected_state_error())?;
        checkpoint.validate()?;
        if checkpoint.schema_version != CHECKPOINT_SCHEMA_VERSION
            || checkpoint.session != *session
            || checkpoint.turn.as_str() != parsed.turn
        {
            return Err(protected_state_error());
        }
        Ok(Some(checkpoint))
    }

    fn seal(&self, checkpoint: &TurnCheckpoint) -> Result<Vec<u8>, RuntimeError> {
        let plaintext =
            Zeroizing::new(serde_json::to_vec(checkpoint).map_err(|_| protected_state_error())?);
        if plaintext.len() > MAX_CHECKPOINT_PLAINTEXT_BYTES {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                "protected checkpoint exceeds Smith's size limit",
            ));
        }
        let nonce = XNonce::try_generate().map_err(|_| CheckpointProtectionError::unavailable())?;
        let header = encode_header(&checkpoint.session, checkpoint.turn.as_str(), &nonce)?;
        let aad = authenticated_data(self.paths.project().as_str(), &header);
        let ciphertext = self
            .key
            .cipher()
            .encrypt(
                &nonce,
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| protected_state_error())?;
        let mut envelope = header;
        envelope.extend_from_slice(&ciphertext);
        Ok(envelope)
    }
}

impl fmt::Debug for SmithCheckpointStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SmithCheckpointStore")
            .field("paths", &self.paths)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// Orders a host-owned durability barrier before the exact encrypted write.
pub(crate) struct BarrierCheckpointStore {
    inner: Arc<dyn CheckpointStore>,
    barrier: Arc<dyn CheckpointBarrier>,
}

impl BarrierCheckpointStore {
    pub(crate) fn new(
        inner: Arc<dyn CheckpointStore>,
        barrier: Arc<dyn CheckpointBarrier>,
    ) -> Self {
        Self { inner, barrier }
    }
}

impl fmt::Debug for BarrierCheckpointStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BarrierCheckpointStore")
            .field("inner", &self.inner)
            .field("barrier", &self.barrier)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl CheckpointStore for BarrierCheckpointStore {
    async fn load_latest(
        &self,
        session: &SessionId,
    ) -> Result<Option<TurnCheckpoint>, RuntimeError> {
        self.inner.load_latest(session).await
    }

    async fn save(&self, checkpoint: &TurnCheckpoint) -> Result<(), RuntimeError> {
        self.barrier.before_checkpoint(checkpoint).await?;
        self.inner.save(checkpoint).await
    }
}

#[async_trait]
impl CheckpointStore for SmithCheckpointStore {
    async fn load_latest(
        &self,
        session: &SessionId,
    ) -> Result<Option<TurnCheckpoint>, RuntimeError> {
        self.load_latest_inner(session).await
    }

    async fn save(&self, checkpoint: &TurnCheckpoint) -> Result<(), RuntimeError> {
        checkpoint.validate()?;
        let _guard = self.save_gate.lock().await;
        let lock_path = self.paths.checkpoint_lock(&checkpoint.session)?;
        let _writer_lease = acquire_private_lock(&lock_path).await?;
        if let Some(existing) = self.load_latest_inner(&checkpoint.session).await? {
            match compare_checkpoint(&existing, checkpoint)? {
                SaveDecision::AlreadyStored => return Ok(()),
                SaveDecision::Replace => {}
            }
        } else if checkpoint.state_revision != 0
            || checkpoint.watermark.checkpoint_sequence != 1
            || !matches!(
                checkpoint.state,
                TurnState::Accepted { .. }
                    | TurnState::InternalAccepted { .. }
                    | TurnState::LocalActionAccepted { .. }
            )
        {
            return Err(RuntimeError::conflict(
                "the first protected checkpoint must be an accepted turn at revision zero and sequence one",
            ));
        }

        let path = self.paths.checkpoint(&checkpoint.session)?;
        let envelope = self.seal(checkpoint)?;
        write_private_atomically(&path, &envelope).await
    }
}

enum SaveDecision {
    AlreadyStored,
    Replace,
}

fn compare_checkpoint(
    existing: &TurnCheckpoint,
    next: &TurnCheckpoint,
) -> Result<SaveDecision, RuntimeError> {
    if existing.session != next.session {
        return Err(RuntimeError::conflict(
            "protected checkpoint belongs to another session",
        ));
    }
    if existing.turn == next.turn {
        if next.state_revision < existing.state_revision {
            return Err(RuntimeError::conflict(
                "protected checkpoint revision cannot move backwards",
            ));
        }
        if next.state_revision == existing.state_revision {
            return if next == existing {
                Ok(SaveDecision::AlreadyStored)
            } else {
                Err(RuntimeError::conflict(
                    "protected checkpoint revision aliases different state",
                ))
            };
        }
        existing.validate_successor(next)?;
        return Ok(SaveDecision::Replace);
    }

    if matches!(existing.state, TurnState::Terminal { .. })
        && next.state_revision == 0
        && matches!(
            next.state,
            TurnState::Accepted { .. }
                | TurnState::InternalAccepted { .. }
                | TurnState::LocalActionAccepted { .. }
        )
        && next.watermark.checkpoint_sequence
            == existing.watermark.checkpoint_sequence.saturating_add(1)
        && next.watermark.event_sequence >= existing.watermark.event_sequence
    {
        Ok(SaveDecision::Replace)
    } else {
        Err(RuntimeError::conflict(
            "a new protected turn cannot replace unfinished checkpoint state",
        ))
    }
}

fn encode_header(session: &SessionId, turn: &str, nonce: &XNonce) -> Result<Vec<u8>, RuntimeError> {
    let session = session.as_str().as_bytes();
    let turn = turn.as_bytes();
    if session.len() > MAX_ID_BYTES || turn.len() > MAX_ID_BYTES {
        return Err(RuntimeError::new(
            ErrorKind::Limit,
            "protected checkpoint identity exceeds Smith's size limit",
        ));
    }
    let session_len = u16::try_from(session.len()).map_err(|_| {
        RuntimeError::new(
            ErrorKind::Limit,
            "protected checkpoint session identity is too long",
        )
    })?;
    let turn_len = u16::try_from(turn.len()).map_err(|_| {
        RuntimeError::new(
            ErrorKind::Limit,
            "protected checkpoint turn identity is too long",
        )
    })?;
    let mut header = Vec::with_capacity(FIXED_HEADER_BYTES + session.len() + turn.len());
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&PROTECTED_CHECKPOINT_SCHEMA_VERSION.to_be_bytes());
    header.push(ALGORITHM_XCHACHA20_POLY1305);
    header.extend_from_slice(&session_len.to_be_bytes());
    header.extend_from_slice(&turn_len.to_be_bytes());
    header.extend_from_slice(session);
    header.extend_from_slice(turn);
    header.extend_from_slice(nonce);
    Ok(header)
}

struct ParsedEnvelope<'a> {
    header: &'a [u8],
    session: &'a str,
    turn: &'a str,
    nonce: &'a XNonce,
    ciphertext: &'a [u8],
}

fn parse_envelope(envelope: &[u8]) -> Result<ParsedEnvelope<'_>, RuntimeError> {
    if envelope.len() < FIXED_HEADER_BYTES + TAG_BYTES || envelope.get(..MAGIC.len()) != Some(MAGIC)
    {
        return Err(protected_state_error());
    }
    let schema = u32::from_be_bytes(
        envelope[MAGIC.len()..MAGIC.len() + 4]
            .try_into()
            .map_err(|_| protected_state_error())?,
    );
    if schema != PROTECTED_CHECKPOINT_SCHEMA_VERSION {
        return Err(RuntimeError::new(
            ErrorKind::Serialization,
            format!(
                "protected checkpoint envelope uses schema version {schema}; \
                 this build reads version {PROTECTED_CHECKPOINT_SCHEMA_VERSION}"
            ),
        ));
    }
    let algorithm_offset = MAGIC.len() + 4;
    if envelope[algorithm_offset] != ALGORITHM_XCHACHA20_POLY1305 {
        return Err(protected_state_error());
    }
    let session_len = usize::from(u16::from_be_bytes(
        envelope[algorithm_offset + 1..algorithm_offset + 3]
            .try_into()
            .map_err(|_| protected_state_error())?,
    ));
    let turn_len = usize::from(u16::from_be_bytes(
        envelope[algorithm_offset + 3..algorithm_offset + 5]
            .try_into()
            .map_err(|_| protected_state_error())?,
    ));
    if session_len > MAX_ID_BYTES || turn_len > MAX_ID_BYTES {
        return Err(protected_state_error());
    }
    let identities_start = algorithm_offset + 5;
    let session_end = identities_start
        .checked_add(session_len)
        .ok_or_else(protected_state_error)?;
    let turn_end = session_end
        .checked_add(turn_len)
        .ok_or_else(protected_state_error)?;
    let header_end = turn_end
        .checked_add(NONCE_BYTES)
        .ok_or_else(protected_state_error)?;
    if envelope.len() < header_end + TAG_BYTES {
        return Err(protected_state_error());
    }
    let session = std::str::from_utf8(&envelope[identities_start..session_end])
        .map_err(|_| protected_state_error())?;
    let turn = std::str::from_utf8(&envelope[session_end..turn_end])
        .map_err(|_| protected_state_error())?;
    let nonce: &XNonce = (&envelope[turn_end..header_end])
        .try_into()
        .map_err(|_| protected_state_error())?;
    Ok(ParsedEnvelope {
        header: &envelope[..header_end],
        session,
        turn,
        nonce,
        ciphertext: &envelope[header_end..],
    })
}

fn authenticated_data(project: &str, header: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(64 + project.len() + header.len());
    append_aad_field(&mut aad, b"smith.protected-checkpoint");
    append_aad_field(&mut aad, project.as_bytes());
    append_aad_field(&mut aad, header);
    aad
}

fn append_aad_field(aad: &mut Vec<u8>, field: &[u8]) {
    let length = u64::try_from(field.len()).unwrap_or(u64::MAX);
    aad.extend_from_slice(&length.to_be_bytes());
    aad.extend_from_slice(field);
}

fn protected_state_error() -> RuntimeError {
    RuntimeError::new(ErrorKind::Serialization, PROTECTED_STATE_DIAGNOSTIC)
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use agent_runtime::registry::RegistryRevision;
    use agent_runtime_core::checkpoint::CheckpointWatermark;
    use agent_runtime_core::clock::{Deadline, Timestamp};
    use agent_runtime_core::content::UserInput;
    use agent_runtime_core::event::TurnFinish;
    use agent_runtime_core::ids::TurnId;
    use agent_runtime_core::store::{SessionIdentityState, SessionSnapshot};
    use agent_runtime_core::usage::UsageLedger;
    use smith_config::credential::{Environment, Keychain, KeychainError};

    use super::*;
    use crate::session::ProjectId;

    #[derive(Debug)]
    struct FixedKeyProvider([u8; KEY_BYTES]);

    impl CheckpointKeyProvider for FixedKeyProvider {
        fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
            Ok(CheckpointKey::new(self.0))
        }
    }

    #[derive(Debug)]
    struct UnavailableKeyProvider;

    impl CheckpointKeyProvider for UnavailableKeyProvider {
        fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
            Err(CheckpointProtectionError::unavailable())
        }
    }

    #[derive(Debug)]
    struct PanicsIfKeychainCalled;

    impl Keychain for PanicsIfKeychainCalled {
        fn secret(&self, _service: &str, _account: &str) -> Result<Secret, KeychainError> {
            panic!("an environment checkpoint key consulted the keychain")
        }
    }

    #[derive(Debug)]
    struct FixedCheckpointEnvironment(&'static str);

    impl Environment for FixedCheckpointEnvironment {
        fn value(&self, _name: &str) -> Option<Secret> {
            Some(Secret::new(self.0))
        }
    }

    fn paths(root: &std::path::Path, project: &str) -> SessionPaths {
        SessionPaths::new(root, &ProjectId::new(project).unwrap())
    }

    async fn store(
        root: &std::path::Path,
        project: &str,
        key: [u8; KEY_BYTES],
    ) -> SmithCheckpointStore {
        SmithCheckpointStore::initialize_with(paths(root, project), Arc::new(FixedKeyProvider(key)))
            .await
            .unwrap()
    }

    fn snapshot(session: &SessionId) -> SessionSnapshot {
        SessionSnapshot {
            id: session.clone(),
            history: Vec::new(),
            usage: UsageLedger::new(),
            identity: SessionIdentityState::default(),
            manifests: Vec::new(),
            extension_state: Default::default(),
            updated: Timestamp::ZERO,
        }
    }

    fn accepted(session: &str, turn: &str) -> TurnCheckpoint {
        let session = SessionId::new(session);
        let input = UserInput::text("secret prompt");
        let mut snapshot = snapshot(&session);
        snapshot.history.push(input.clone().into_message());
        TurnCheckpoint::accepted(
            TurnId::new(turn),
            input,
            snapshot,
            0,
            Deadline::never(),
            1,
            7,
            Timestamp::ZERO,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn round_trip_is_encrypted_owner_only_and_key_is_redacted() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path(), "project-a", [7; KEY_BYTES]).await;
        let mut checkpoint = accepted("session-a", "turn-a");
        let sensitive_state = "sensitive-extension-state-93bd";
        checkpoint.snapshot.extension_state.insert(
            "smith.memory".into(),
            agent_runtime_core::store::VersionedSessionState::new(
                RegistryRevision::new("memory-state-1"),
                serde_json::json!({"content": sensitive_state}),
            ),
        );

        store.save(&checkpoint).await.unwrap();
        let restored = store
            .load_latest(&checkpoint.session)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(restored, checkpoint);
        assert_eq!(
            restored.snapshot.extension_state["smith.memory"].value,
            serde_json::json!({"content": sensitive_state})
        );
        let path = store.paths().checkpoint(&checkpoint.session).unwrap();
        let bytes = tokio::fs::read(&path).await.unwrap();
        assert!(bytes.starts_with(MAGIC));
        assert!(
            !bytes
                .windows(b"secret prompt".len())
                .any(|window| { window == b"secret prompt" })
        );
        assert!(
            !bytes
                .windows(sensitive_state.len())
                .any(|window| { window == sensitive_state.as_bytes() })
        );
        assert_eq!(
            tokio::fs::metadata(path)
                .await
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let debug = format!("{store:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("070707"));
    }

    #[tokio::test]
    async fn configured_hex_key_round_trips_without_platform_storage() {
        const ENCODED: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let root = tempfile::tempdir().unwrap();
        let provider = Arc::new(
            ConfiguredCheckpointKeyProvider::new(&Secret::new(ENCODED)).expect("a configured key"),
        );
        let store = SmithCheckpointStore::initialize_with(
            paths(root.path(), "configured-project"),
            provider,
        )
        .await
        .expect("a configured store");
        let checkpoint = accepted("configured-session", "configured-turn");
        store.save(&checkpoint).await.expect("encrypted save");
        assert_eq!(
            store
                .load_latest(&checkpoint.session)
                .await
                .expect("encrypted load"),
            Some(checkpoint.clone())
        );
        let envelope = tokio::fs::read(
            store
                .paths()
                .checkpoint(&checkpoint.session)
                .expect("checkpoint path"),
        )
        .await
        .expect("checkpoint envelope");
        assert!(envelope.starts_with(MAGIC));
        assert!(
            !envelope
                .windows(ENCODED.len())
                .any(|window| window == ENCODED.as_bytes())
        );
        let debug = format!(
            "{:?}",
            ConfiguredCheckpointKeyProvider::new(&Secret::new(ENCODED)).unwrap()
        );
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains(ENCODED));
    }

    #[test]
    fn configured_key_rejects_invalid_material_with_one_opaque_error() {
        let invalid_hex = "g".repeat(KEY_BYTES * 2);
        for invalid in ["short", invalid_hex.as_str()] {
            let error = ConfiguredCheckpointKeyProvider::new(&Secret::new(invalid))
                .expect_err("invalid checkpoint key material");
            assert_eq!(error.to_string(), PROTECTED_STATE_DIAGNOSTIC);
            assert!(!format!("{error:?}").contains(invalid));
        }
    }

    #[test]
    fn environment_reference_never_calls_the_keychain_backend() {
        const ENCODED: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let resolver = CredentialResolver::new("/nonexistent-user-state")
            .with_keychain(Arc::new(PanicsIfKeychainCalled))
            .with_environment(Arc::new(FixedCheckpointEnvironment(ENCODED)));
        let provider =
            CredentialCheckpointKeyProvider::new(resolver, "env:SMITH_CHECKPOINT_SECRET")
                .expect("an environment checkpoint provider");
        provider
            .load_or_create()
            .expect("the environment key is decoded without keychain access");
    }

    #[tokio::test]
    async fn every_seal_uses_a_fresh_nonce() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path(), "project-a", [8; KEY_BYTES]).await;
        let checkpoint = accepted("session-a", "turn-a");

        let first = store.seal(&checkpoint).unwrap();
        let second = store.seal(&checkpoint).unwrap();

        assert_ne!(first, second);
        assert_ne!(
            parse_envelope(&first).unwrap().nonce,
            parse_envelope(&second).unwrap().nonce
        );
    }

    #[tokio::test]
    async fn project_and_session_identity_are_authenticated() {
        let root = tempfile::tempdir().unwrap();
        let first = store(root.path(), "project-a", [9; KEY_BYTES]).await;
        let second = store(root.path(), "project-b", [9; KEY_BYTES]).await;
        let checkpoint = accepted("session-a", "turn-a");
        first.save(&checkpoint).await.unwrap();
        second.paths.ensure_directory().await.unwrap();
        let source = first.paths().checkpoint(&checkpoint.session).unwrap();
        let moved = second.paths().checkpoint(&checkpoint.session).unwrap();
        tokio::fs::copy(&source, &moved).await.unwrap();

        assert_eq!(
            second
                .load_latest(&checkpoint.session)
                .await
                .unwrap_err()
                .message,
            PROTECTED_STATE_DIAGNOSTIC
        );

        let other_session = SessionId::new("session-b");
        let renamed = first.paths().checkpoint(&other_session).unwrap();
        tokio::fs::copy(source, renamed).await.unwrap();
        assert_eq!(
            first.load_latest(&other_session).await.unwrap_err().message,
            PROTECTED_STATE_DIAGNOSTIC
        );
    }

    #[tokio::test]
    async fn corruption_wrong_key_and_unavailable_key_share_one_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        let writer = store(root.path(), "project-a", [10; KEY_BYTES]).await;
        let checkpoint = accepted("session-a", "turn-a");
        writer.save(&checkpoint).await.unwrap();

        let wrong_key = store(root.path(), "project-a", [11; KEY_BYTES]).await;
        let wrong_key_error = wrong_key
            .load_latest(&checkpoint.session)
            .await
            .unwrap_err();

        let path = writer.paths().checkpoint(&checkpoint.session).unwrap();
        let mut bytes = tokio::fs::read(&path).await.unwrap();
        let last = bytes.last_mut().unwrap();
        *last ^= 0x80;
        tokio::fs::write(&path, bytes).await.unwrap();
        let corruption_error = writer.load_latest(&checkpoint.session).await.unwrap_err();
        let unavailable = SmithCheckpointStore::initialize_with(
            paths(root.path(), "project-a"),
            Arc::new(UnavailableKeyProvider),
        )
        .await
        .unwrap_err();

        assert_eq!(wrong_key_error.message, PROTECTED_STATE_DIAGNOSTIC);
        assert_eq!(corruption_error.message, PROTECTED_STATE_DIAGNOSTIC);
        assert_eq!(unavailable.to_string(), PROTECTED_STATE_DIAGNOSTIC);
    }

    #[tokio::test]
    async fn envelope_version_is_checked_before_decryption() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path(), "project-a", [12; KEY_BYTES]).await;
        let checkpoint = accepted("session-a", "turn-a");
        store.save(&checkpoint).await.unwrap();
        let path = store.paths().checkpoint(&checkpoint.session).unwrap();
        let mut bytes = tokio::fs::read(&path).await.unwrap();
        bytes[MAGIC.len()..MAGIC.len() + 4].copy_from_slice(&2u32.to_be_bytes());
        tokio::fs::write(path, bytes).await.unwrap();

        let error = store.load_latest(&checkpoint.session).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Serialization);
        assert!(error.message.contains("schema version 2"));
    }

    #[tokio::test]
    async fn torn_or_malformed_envelopes_fail_closed_without_parse_details() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path(), "project-a", [17; KEY_BYTES]).await;
        let checkpoint = accepted("session-a", "turn-a");
        store.save(&checkpoint).await.unwrap();
        let path = store.paths().checkpoint(&checkpoint.session).unwrap();
        let original = tokio::fs::read(&path).await.unwrap();

        for length in [
            0,
            MAGIC.len(),
            FIXED_HEADER_BYTES.saturating_sub(1),
            original.len().saturating_sub(1),
        ] {
            tokio::fs::write(&path, &original[..length]).await.unwrap();
            let error = store.load_latest(&checkpoint.session).await.unwrap_err();
            assert_eq!(error.message, PROTECTED_STATE_DIAGNOSTIC);
        }

        let mut malformed = original;
        let lengths = MAGIC.len() + 5;
        malformed[lengths..lengths + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        tokio::fs::write(path, malformed).await.unwrap();
        assert_eq!(
            store
                .load_latest(&checkpoint.session)
                .await
                .unwrap_err()
                .message,
            PROTECTED_STATE_DIAGNOSTIC
        );
    }

    #[tokio::test]
    async fn save_is_idempotent_and_rejects_backward_or_aliased_revision() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path(), "project-a", [13; KEY_BYTES]).await;
        let accepted = accepted("session-a", "turn-a");
        store.save(&accepted).await.unwrap();
        store.save(&accepted).await.unwrap();

        let planning = accepted
            .transition(
                TurnState::Planning { step: 0 },
                accepted.snapshot.clone(),
                8,
                Timestamp(1),
            )
            .unwrap();
        store.save(&planning).await.unwrap();
        assert_eq!(
            store.save(&accepted).await.unwrap_err().kind,
            ErrorKind::Conflict
        );

        let mut alias = planning.clone();
        alias.watermark = CheckpointWatermark::new(99, 99);
        assert_eq!(
            store.save(&alias).await.unwrap_err().kind,
            ErrorKind::Conflict
        );
    }

    #[tokio::test]
    async fn a_new_turn_cannot_replace_unfinished_state() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path(), "project-a", [14; KEY_BYTES]).await;
        let first = accepted("session-a", "turn-a");
        let mut second = accepted("session-a", "turn-b");
        store.save(&first).await.unwrap();

        assert_eq!(
            store.save(&second).await.unwrap_err().kind,
            ErrorKind::Conflict
        );

        let completing = first
            .transition(
                TurnState::Completing {
                    finish: TurnFinish::Completed,
                    visible_output: false,
                    provider_error_kind: None,
                },
                first.snapshot.clone(),
                8,
                Timestamp(1),
            )
            .unwrap();
        store.save(&completing).await.unwrap();
        let publishing = completing
            .transition(
                TurnState::PublishingTerminal {
                    finish: TurnFinish::Completed,
                    visible_output: false,
                },
                first.snapshot.clone(),
                9,
                Timestamp(2),
            )
            .unwrap();
        store.save(&publishing).await.unwrap();
        let terminal = publishing
            .transition(
                TurnState::Terminal {
                    finish: TurnFinish::Completed,
                    visible_output: false,
                },
                first.snapshot.clone(),
                10,
                Timestamp(3),
            )
            .unwrap();
        store.save(&terminal).await.unwrap();
        second.watermark = CheckpointWatermark::new(
            terminal.watermark.checkpoint_sequence.saturating_add(1),
            terminal.watermark.event_sequence.saturating_add(1),
        );
        store.save(&second).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_store_instances_share_one_cross_process_writer_lease() {
        let root = tempfile::tempdir().unwrap();
        let first = store(root.path(), "project-a", [15; KEY_BYTES]).await;
        let second = store(root.path(), "project-a", [15; KEY_BYTES]).await;
        let accepted = accepted("session-a", "turn-a");
        first.save(&accepted).await.unwrap();
        let planning = accepted
            .transition(
                TurnState::Planning { step: 0 },
                accepted.snapshot.clone(),
                8,
                Timestamp(1),
            )
            .unwrap();

        let lock_path = first.paths().checkpoint_lock(&accepted.session).unwrap();
        let lease = acquire_private_lock(&lock_path).await.unwrap();
        let expected = planning.clone();
        let mut blocked = tokio::spawn(async move { second.save(&planning).await });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(40), &mut blocked)
                .await
                .is_err(),
            "a second store must wait for the existing writer lease"
        );
        drop(lease);
        blocked.await.unwrap().unwrap();
        assert_eq!(
            first.load_latest(&accepted.session).await.unwrap(),
            Some(expected)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_sibling_successors_cannot_both_replace_latest() {
        let root = tempfile::tempdir().unwrap();
        let first = store(root.path(), "project-a", [16; KEY_BYTES]).await;
        let second = store(root.path(), "project-a", [16; KEY_BYTES]).await;
        let accepted = accepted("session-a", "turn-a");
        first.save(&accepted).await.unwrap();
        let planning = accepted
            .transition(
                TurnState::Planning { step: 0 },
                accepted.snapshot.clone(),
                8,
                Timestamp(1),
            )
            .unwrap();
        let completing = accepted
            .transition(
                TurnState::Completing {
                    finish: TurnFinish::Completed,
                    visible_output: false,
                    provider_error_kind: None,
                },
                accepted.snapshot.clone(),
                8,
                Timestamp(1),
            )
            .unwrap();

        let (planning_result, completing_result) =
            tokio::join!(first.save(&planning), second.save(&completing));
        assert_ne!(planning_result.is_ok(), completing_result.is_ok());
        let latest = first.load_latest(&accepted.session).await.unwrap().unwrap();
        assert_eq!(
            latest,
            if planning_result.is_ok() {
                planning
            } else {
                completing
            }
        );
    }
}
