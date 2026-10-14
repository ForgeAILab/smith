use crate::WasmError;
use sha2::{Digest, Sha256};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;
use wasmtime::component::Component;

/// Shared component engine with one background epoch ticker.
#[derive(Debug, Clone)]
pub struct WasmEngine(Arc<wasmtime::Engine>);
impl WasmEngine {
    /// Creates the engine and a single millisecond epoch clock.
    pub fn new() -> Result<Self, WasmError> {
        let engine =
            Arc::new(super::diagnostic_engine().map_err(|e| WasmError::Load(e.to_string()))?);
        let weak = Arc::downgrade(&engine);
        std::thread::Builder::new()
            .name("smith-wasm-epochs".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    let Some(engine) = weak.upgrade() else {
                        break;
                    };
                    engine.increment_epoch();
                }
            })
            .map_err(|e| WasmError::Load(e.to_string()))?;
        Ok(Self(engine))
    }
    pub(crate) fn raw(&self) -> &wasmtime::Engine {
        &self.0
    }
    /// Loads a component using a private, engine- and content-specific disk cache.
    pub(crate) fn compile(&self, bytes: &[u8], cache_dir: &Path) -> Result<Component, WasmError> {
        std::fs::create_dir_all(cache_dir).map_err(|e| WasmError::Load(e.to_string()))?;
        let metadata =
            std::fs::symlink_metadata(cache_dir).map_err(|e| WasmError::Load(e.to_string()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(WasmError::Load(
                "cache must be a user-owned directory".into(),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.uid() != rustix::process::getuid().as_raw() {
                return Err(WasmError::Load("cache owner differs from parent".into()));
            }
            std::fs::set_permissions(cache_dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| WasmError::Load(e.to_string()))?;
        }
        let mut compatibility = std::collections::hash_map::DefaultHasher::new();
        self.0
            .precompile_compatibility_hash()
            .hash(&mut compatibility);
        let key = format!(
            "{:x}-36.0.18-{:016x}",
            Sha256::digest(bytes),
            compatibility.finish()
        );
        let path = cache_dir.join(format!("{key}.cwasm"));
        // Checksums detect corruption before entering Wasmtime's unsafe decoder.
        if std::fs::symlink_metadata(&path)
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
            && let Ok(cached) = std::fs::read(&path)
            && cached.len() >= 64
            && Sha256::digest(key.as_bytes())[..] == cached[..32]
            && Sha256::digest(&cached[64..])[..] == cached[32..64]
            && let Ok(component) = deserialize(self.raw(), &cached[64..])
        {
            return Ok(component);
        }
        let _ = std::fs::remove_file(&path);
        let component =
            Component::new(self.raw(), bytes).map_err(|e| WasmError::Load(e.to_string()))?;
        let compiled = component
            .serialize()
            .map_err(|e| WasmError::Load(e.to_string()))?;
        let mut entry = Sha256::digest(key.as_bytes()).to_vec();
        entry.extend(Sha256::digest(&compiled));
        entry.extend(compiled);
        // Atomic replacement avoids partial entries and concurrent writer races.
        use std::io::Write;
        let mut temporary = tempfile::NamedTempFile::new_in(cache_dir)
            .map_err(|e| WasmError::Load(e.to_string()))?;
        temporary
            .write_all(&entry)
            .map_err(|e| WasmError::Load(e.to_string()))?;
        temporary
            .persist(&path)
            .map_err(|e| WasmError::Load(e.to_string()))?;
        Ok(component)
    }
}
#[allow(unsafe_code)]
fn deserialize(engine: &wasmtime::Engine, bytes: &[u8]) -> wasmtime::Result<Component> {
    // SAFETY: entries are emitted by this binary into a user-owned 0700 cache;
    // the adapter confines the cache to reserved host state, excluded from
    // workspace grants and plugin-data aliases. The filename includes content digest, pinned Wasmtime version and engine
    // compatibility fingerprint. The checksum rejects accidental corruption.
    // Like other executable caches, a malicious same-user writer is outside the
    // sandbox threat boundary. Wasmtime also checks engine compatibility.
    unsafe { Component::deserialize(engine, bytes) }
}
