//! Owner-only, session-authorized artifact storage.
//!
//! Tool output artifacts live in Smith's user state, never in the project
//! workspace. Opaque references are content-addressed for integrity but do not
//! grant access: every read compares the runtime-owned requesting session with
//! the owner stored in protected metadata.

use std::fmt;
use std::path::{Path, PathBuf};

use agent_runtime_core::artifact::{
    ArtifactChunk, ArtifactDigest, ArtifactError, ArtifactId, ArtifactRead, ArtifactRef,
    ArtifactStore, ArtifactWrite, MAX_ARTIFACT_MEDIA_TYPE_CHARS,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::private_storage::{
    ensure_private_directory, read_private_bounded, write_private_atomically,
};
use crate::session::SessionPaths;

/// Hard upper bound for one exact stored artifact.
///
/// The standard offloader runs before model-facing truncation, so this cap is
/// a host resource boundary rather than a context-window policy.
pub const MAX_SMITH_ARTIFACT_BYTES: usize = 32 * 1024 * 1024;

const ARTIFACT_SCHEMA_VERSION: u32 = 1;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 1_024;

/// Project-scoped storage whose records remain private to their owning
/// runtime session.
pub struct SmithArtifactStore {
    paths: SessionPaths,
    write_gate: Mutex<()>,
}

impl SmithArtifactStore {
    /// Creates a lazy store below `paths`.
    ///
    /// No directory is created until the first artifact is written, preserving
    /// Smith's no-side-effects-before-valid-startup rule.
    pub fn new(paths: SessionPaths) -> Self {
        Self {
            paths,
            write_gate: Mutex::new(()),
        }
    }

    /// Project-scoped owner-only artifact directory.
    pub fn directory(&self) -> PathBuf {
        self.paths.artifacts_directory()
    }

    fn record_paths(&self, id: &ArtifactId) -> (PathBuf, PathBuf) {
        // ArtifactId is intentionally host-neutral and may contain path
        // separators. Hashing the opaque value keeps model input out of paths.
        let stem = sha256_hex(id.as_str().as_bytes());
        let directory = self.directory();
        (
            directory.join(format!("{stem}.metadata.json")),
            directory.join(format!("{stem}.content")),
        )
    }

    fn reference(write: &ArtifactWrite) -> Result<ArtifactRef, ArtifactError> {
        validate_write(write)?;
        let content_digest = sha256_hex(&write.bytes);
        let id_digest = digest_fields(&[
            b"smith-artifact-id-v1",
            write.provenance.session.as_str().as_bytes(),
            write.idempotency_key.as_bytes(),
        ]);
        let reference = ArtifactRef {
            id: ArtifactId::new(format!("a-{id_digest}"))?,
            digest: ArtifactDigest::new("sha256", content_digest)?,
            media_type: write.media_type.clone(),
            byte_length: u64::try_from(write.bytes.len()).unwrap_or(u64::MAX),
            sensitivity: write.sensitivity,
            retention: write.retention,
            provenance: write.provenance.clone(),
        };
        reference.validate()?;
        Ok(reference)
    }

    async fn load_record(
        &self,
        metadata_path: &Path,
    ) -> Result<Option<StoredArtifact>, ArtifactError> {
        let Some(bytes) = read_private_bounded(metadata_path, MAX_METADATA_BYTES)
            .await
            .map_err(unavailable)?
        else {
            return Ok(None);
        };
        let record: StoredArtifact =
            serde_json::from_slice(&bytes).map_err(|_| ArtifactError::Integrity {
                detail: "artifact metadata is malformed".into(),
            })?;
        if record.schema_version != ARTIFACT_SCHEMA_VERSION {
            return Err(ArtifactError::Integrity {
                detail: "artifact metadata schema is unsupported".into(),
            });
        }
        record.reference.validate()?;
        Ok(Some(record))
    }

    async fn read_verified_bytes(
        &self,
        content_path: &Path,
        reference: &ArtifactRef,
    ) -> Result<Vec<u8>, ArtifactError> {
        let bytes = read_private_bounded(content_path, MAX_SMITH_ARTIFACT_BYTES)
            .await
            .map_err(unavailable)?
            .ok_or_else(|| ArtifactError::Integrity {
                detail: "artifact content is missing".into(),
            })?;
        if bytes.len() as u64 != reference.byte_length
            || reference.digest.algorithm != "sha256"
            || sha256_hex(&bytes) != reference.digest.hex
        {
            return Err(ArtifactError::Integrity {
                detail: "artifact content does not match its protected metadata".into(),
            });
        }
        Ok(bytes)
    }
}

impl fmt::Debug for SmithArtifactStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SmithArtifactStore")
            .field("directory", &self.directory())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredArtifact {
    schema_version: u32,
    reference: ArtifactRef,
}

#[async_trait]
impl ArtifactStore for SmithArtifactStore {
    async fn put(&self, write: ArtifactWrite) -> Result<ArtifactRef, ArtifactError> {
        let reference = Self::reference(&write)?;
        let (metadata_path, content_path) = self.record_paths(&reference.id);
        let _guard = self.write_gate.lock().await;
        ensure_private_directory(&self.directory())
            .await
            .map_err(unavailable)?;

        if let Some(existing) = self.load_record(&metadata_path).await? {
            if existing.reference != reference {
                return Err(ArtifactError::Integrity {
                    detail:
                        "artifact idempotency key was reused with different content or metadata"
                            .into(),
                });
            }
            self.read_verified_bytes(&content_path, &existing.reference)
                .await?;
            return Ok(existing.reference);
        }

        write_private_atomically(&content_path, &write.bytes)
            .await
            .map_err(unavailable)?;
        let metadata = serde_json::to_vec(&StoredArtifact {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            reference: reference.clone(),
        })
        .map_err(|_| ArtifactError::Unavailable {
            detail: "artifact metadata could not be encoded".into(),
        })?;
        if metadata.len() > MAX_METADATA_BYTES {
            return Err(ArtifactError::Unavailable {
                detail: "artifact metadata exceeds Smith's size limit".into(),
            });
        }
        write_private_atomically(&metadata_path, &metadata)
            .await
            .map_err(unavailable)?;
        Ok(reference)
    }

    async fn read(&self, read: ArtifactRead) -> Result<ArtifactChunk, ArtifactError> {
        read.validate()?;
        let (metadata_path, content_path) = self.record_paths(&read.id);
        let record = self
            .load_record(&metadata_path)
            .await?
            .ok_or(ArtifactError::NotFound)?;
        if record.reference.id != read.id {
            return Err(ArtifactError::Integrity {
                detail: "artifact metadata identifies a different reference".into(),
            });
        }
        if record.reference.provenance.session != read.session {
            return Err(ArtifactError::AccessDenied);
        }
        let bytes = self
            .read_verified_bytes(&content_path, &record.reference)
            .await?;
        let start = usize::try_from(read.offset).map_err(|_| ArtifactError::InvalidRange {
            detail: "artifact offset exceeds this platform's addressable range".into(),
        })?;
        if start > bytes.len() {
            return Err(ArtifactError::InvalidRange {
                detail: "artifact offset is beyond end-of-file".into(),
            });
        }
        let end = start.saturating_add(read.limit as usize).min(bytes.len());
        let next_offset = (end < bytes.len()).then_some(end as u64);
        let chunk = ArtifactChunk {
            reference: record.reference,
            bytes: bytes[start..end].to_vec(),
            offset: read.offset,
            next_offset,
        };
        chunk.validate_for(&read)?;
        Ok(chunk)
    }
}

fn validate_write(write: &ArtifactWrite) -> Result<(), ArtifactError> {
    if write.bytes.len() > MAX_SMITH_ARTIFACT_BYTES {
        return Err(ArtifactError::Unavailable {
            detail: format!("artifact exceeds Smith's {MAX_SMITH_ARTIFACT_BYTES} byte hard limit"),
        });
    }
    if write.media_type.is_empty()
        || write.media_type.chars().count() > MAX_ARTIFACT_MEDIA_TYPE_CHARS
    {
        return Err(ArtifactError::InvalidReference {
            detail: format!(
                "artifact media type must contain 1..={MAX_ARTIFACT_MEDIA_TYPE_CHARS} characters"
            ),
        });
    }
    if write.idempotency_key.is_empty() || write.idempotency_key.len() > MAX_IDEMPOTENCY_KEY_BYTES {
        return Err(ArtifactError::InvalidReference {
            detail: format!(
                "artifact idempotency key must contain 1..={MAX_IDEMPOTENCY_KEY_BYTES} bytes"
            ),
        });
    }
    if write.provenance.session.as_str().is_empty() {
        return Err(ArtifactError::InvalidReference {
            detail: "artifact provenance has an empty owning session".into(),
        });
    }
    if write.sensitivity == agent_runtime_core::artifact::ArtifactSensitivity::Secret {
        return Err(ArtifactError::Unavailable {
            detail: "secret artifact persistence requires a stronger Smith storage policy".into(),
        });
    }
    if write.retention != agent_runtime_core::artifact::ArtifactRetention::Session {
        return Err(ArtifactError::Unavailable {
            detail: "Smith currently supports session-retained artifacts only".into(),
        });
    }
    Ok(())
}

fn digest_fields(fields: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    hex_digest(digest.finalize().as_slice())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes).as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn unavailable(_error: agent_runtime_core::error::RuntimeError) -> ArtifactError {
    ArtifactError::Unavailable {
        detail: "protected Smith artifact storage is unavailable".into(),
    }
}

#[cfg(test)]
mod tests {
    use agent_runtime_core::artifact::{
        ArtifactProvenance, ArtifactRetention, ArtifactSensitivity,
    };
    use agent_runtime_core::ids::{SessionId, ToolCallId, TurnId};

    use super::*;
    use crate::session::ProjectId;

    fn store(root: &Path) -> SmithArtifactStore {
        SmithArtifactStore::new(SessionPaths::new(
            root,
            &ProjectId::new("artifact-tests").expect("project id"),
        ))
    }

    fn write(session: &str, bytes: &[u8]) -> ArtifactWrite {
        ArtifactWrite {
            bytes: bytes.to_vec(),
            media_type: "text/plain".into(),
            sensitivity: ArtifactSensitivity::Sensitive,
            retention: ArtifactRetention::Session,
            provenance: ArtifactProvenance::new(SessionId::new(session), "tool-output")
                .with_turn(TurnId::new("turn-1"))
                .with_call(ToolCallId::new("call-1")),
            idempotency_key: "stable-write".into(),
        }
    }

    #[tokio::test]
    async fn writes_idempotently_and_reads_bounded_verified_pages() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let first = store.put(write("session-1", b"abcdef")).await.unwrap();
        let second = store.put(write("session-1", b"abcdef")).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first.digest.hex,
            "bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721"
        );

        let page = store
            .read(ArtifactRead {
                session: SessionId::new("session-1"),
                id: first.id,
                offset: 1,
                limit: 3,
            })
            .await
            .unwrap();
        assert_eq!(page.bytes, b"bcd");
        assert_eq!(page.next_offset, Some(4));
    }

    #[tokio::test]
    async fn an_exact_reference_never_grants_cross_session_access() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let reference = store.put(write("owner", b"private")).await.unwrap();

        assert_eq!(
            store
                .read(ArtifactRead {
                    session: SessionId::new("different-session"),
                    id: reference.id,
                    offset: 0,
                    limit: 4,
                })
                .await,
            Err(ArtifactError::AccessDenied)
        );
    }

    #[tokio::test]
    async fn an_idempotency_key_cannot_alias_different_content() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        store.put(write("session-1", b"first")).await.unwrap();

        assert!(matches!(
            store.put(write("session-1", b"second")).await,
            Err(ArtifactError::Integrity { .. })
        ));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[tokio::test]
    async fn artifact_files_and_directory_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        store.put(write("session-1", b"private")).await.unwrap();

        assert_eq!(
            tokio::fs::metadata(store.directory())
                .await
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let mut entries = tokio::fs::read_dir(store.directory()).await.unwrap();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            assert_eq!(
                entry.metadata().await.unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
