use crate::bindings::smith::module::{host, types as w};
use crate::{CapabilityGrant, WasmError, WasmLimits};
use cap_std::fs::{Dir, OpenOptions};
use smith_module::{ModuleCapability as C, ModuleContext};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Component, Path};
use std::time::{Duration, Instant};
use wasmtime::{ResourceLimiter, StoreLimits, StoreLimitsBuilder};

pub(crate) struct Broker {
    pub context: Option<ModuleContext>,
    pub grants: Vec<CapabilityGrant>,
    pub data_dir: std::path::PathBuf,
    host_state_dir: std::path::PathBuf,
    pub limits: WasmLimits,
    resources: StoreLimits,
    pub memory_denied: bool,
    pub failure: Option<WasmError>,
    pub session: Option<agent_runtime_core::ids::SessionId>,
    pub runtime: Option<tokio::runtime::Handle>,
    pub deadline: Instant,
}
impl Broker {
    pub(crate) fn new(config: &crate::WasmModuleConfig, context: Option<ModuleContext>) -> Self {
        Self {
            context,
            grants: config.granted_capabilities.clone(),
            data_dir: config.data_dir.clone(),
            host_state_dir: config.host_state_dir.clone(),
            resources: StoreLimitsBuilder::new()
                .memory_size(config.limits.memory_bytes)
                .table_elements(4096)
                .instances(16)
                .tables(16)
                .memories(1)
                .trap_on_grow_failure(true)
                .build(),
            limits: config.limits.clone(),
            memory_denied: false,
            failure: None,
            session: None,
            runtime: tokio::runtime::Handle::try_current().ok(),
            deadline: Instant::now(),
        }
    }
    fn service_call<T: Send + 'static>(
        &mut self,
        operation: impl FnOnce(&mut Broker) -> Result<T, w::HostError> + Send + 'static,
    ) -> Result<T, w::HostError> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.failure = Some(WasmError::Deadline);
            return Err(w::HostError::Unavailable(
                "host service deadline exceeded".into(),
            ));
        }
        let mut service = Self {
            context: self.context.clone(),
            grants: self.grants.clone(),
            data_dir: self.data_dir.clone(),
            host_state_dir: self.host_state_dir.clone(),
            limits: self.limits.clone(),
            resources: StoreLimitsBuilder::new().build(),
            memory_denied: false,
            failure: None,
            session: self.session.clone(),
            runtime: None,
            deadline: self.deadline,
        };
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("smith-wasm-service".into())
            .spawn(move || {
                let result = operation(&mut service);
                let _ = sender.send((result, service.failure));
            })
            .map_err(|_| w::HostError::Unavailable("host service worker unavailable".into()))?;
        match receiver.recv_timeout(remaining) {
            Ok((result, failure)) => {
                if failure.is_some() {
                    self.failure = failure;
                }
                result
            }
            Err(_) => {
                self.failure = Some(WasmError::Deadline);
                Err(w::HostError::Unavailable(
                    "host service deadline exceeded".into(),
                ))
            }
        }
    }
    pub(crate) fn granted(&self, capability: C, root: &str) -> bool {
        self.grants
            .iter()
            .any(|g| g.capability == capability && g.roots.iter().any(|r| r == root))
    }
    fn deny(&mut self, capability: C) -> w::HostError {
        self.failure = Some(WasmError::CapabilityDenied(format!("{capability:?}")));
        w::HostError::CapabilityDenied(crate::projection::capability(capability))
    }
    fn check<T: serde::Serialize>(&mut self, value: &T) -> Result<(), w::HostError> {
        crate::limits::bounded(value, &self.limits).map_err(|error| {
            let host_error = match &error {
                WasmError::OversizedPayload(field) => {
                    let maximum = match field.as_str() {
                        "aggregate" => self.limits.payload_bytes,
                        "body" | "bytes" => 1024 * 1024,
                        "value" => 64 * 1024,
                        "id" | "name" | "root" | "key" => 128,
                        "headers" => 64,
                        _ => 4096,
                    };
                    w::HostError::PayloadTooLarge(w::PayloadLimit {
                        field: if field.is_empty() {
                            "message".into()
                        } else {
                            field.clone()
                        },
                        maximum: maximum as u32,
                    })
                }
                WasmError::MalformedResult(reason) => w::HostError::InvalidInput(reason.clone()),
                _ => w::HostError::Unavailable("protocol validation failed".into()),
            };
            self.failure = Some(error);
            host_error
        })
    }
    fn root(&mut self, file: &w::FilePath, write: bool) -> Result<Dir, w::HostError> {
        self.check(file)?;
        let capability = match (file.root.as_str(), write) {
            ("workspace", false) => C::WorkspaceRead,
            ("workspace", true) => C::WorkspaceWrite,
            ("plugin-data", false) => C::HostRead,
            ("plugin-data", true) => C::HostWrite,
            _ => return Err(w::HostError::InvalidInput("unknown root alias".into())),
        };
        if !self.granted(capability, &file.root) {
            return Err(self.deny(capability));
        }
        if Path::new(&file.path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
            || file.path.is_empty()
        {
            return Err(w::HostError::InvalidInput(
                "relative confined path required".into(),
            ));
        }
        let root = if file.root == "plugin-data" {
            &self.data_dir
        } else {
            self.context
                .as_ref()
                .and_then(|c| c.workspace_root.as_ref())
                .ok_or_else(|| w::HostError::Unavailable("workspace unavailable".into()))?
        };
        let dir = Dir::open_ambient_dir(root, cap_std::ambient_authority()).map_err(io)?;
        // Refuse symlinks at every existing path component, including dangling
        // final links. cap-std's descriptor-relative traversal confines races too.
        let mut relative = std::path::PathBuf::new();
        for component in Path::new(&file.path).components() {
            relative.push(component);
            match dir.symlink_metadata(&relative) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(w::HostError::InvalidInput("symlink refused".into()));
                }
                Ok(meta) if relative == Path::new(&file.path) && !meta.is_file() => {
                    return Err(w::HostError::InvalidInput("regular file required".into()));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && write => {}
                Err(error) => return Err(io(error)),
            }
        }
        if file.root == "workspace" {
            let parent = root
                .join(&file.path)
                .parent()
                .ok_or_else(|| w::HostError::InvalidInput("invalid file parent".into()))?
                .canonicalize()
                .map_err(io)?;
            if parent.starts_with(&self.host_state_dir) {
                return Err(self.deny(capability));
            }
        }
        Ok(dir)
    }
}
impl ResourceLimiter for Broker {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let result = self.resources.memory_growing(current, desired, maximum);
        if !matches!(result, Ok(true)) {
            self.memory_denied = true;
        }
        result
    }
    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let result = self.resources.table_growing(current, desired, maximum);
        if !matches!(result, Ok(true)) {
            self.memory_denied = true;
        }
        result
    }
    fn instances(&self) -> usize {
        16
    }
    fn tables(&self) -> usize {
        16
    }
    fn memories(&self) -> usize {
        1
    }
}
impl w::Host for Broker {}
impl Broker {
    fn log(&mut self, level: w::LogLevel, message: String) -> Result<(), w::HostError> {
        self.check(&message)?;
        if message.len() > 4096 {
            return Err(w::HostError::PayloadTooLarge(w::PayloadLimit {
                field: "message".into(),
                maximum: 4096,
            }));
        }
        // ModuleContext has no redactor: do not emit untrusted message contents
        // into host diagnostics. Length-only logging is bounded and secret-safe.
        tracing_log(level, message.len());
        Ok(())
    }
    fn fs_read(&mut self, file: w::FilePath) -> Result<Vec<u8>, w::HostError> {
        let dir = self.root(&file, false)?;
        let mut bytes = Vec::new();
        open_regular(&dir, &file.path, false)?
            .take((self.limits.payload_bytes + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(io)?;
        self.check(&w::FileWrite {
            file: w::FilePath {
                root: file.root,
                path: "response".into(),
            },
            bytes: bytes.clone(),
        })?;
        Ok(bytes)
    }
    fn fs_write(&mut self, write: w::FileWrite) -> Result<(), w::HostError> {
        self.check(&write)?;
        let dir = self.root(&write.file, true)?;
        open_regular(&dir, &write.file.path, true)?
            .write_all(&write.bytes)
            .map_err(io)
    }
    fn http(&mut self, request: w::HttpRequest) -> Result<w::HttpResponse, w::HostError> {
        self.check(&request)?;
        if request.url.len() > 4096 {
            return Err(w::HostError::PayloadTooLarge(w::PayloadLimit {
                field: "url".into(),
                maximum: 4096,
            }));
        }
        let url = url::Url::parse(&request.url)
            .map_err(|_| w::HostError::InvalidInput("invalid URL".into()))?;
        let permitted = |g: &CapabilityGrant| {
            g.hosts.iter().any(|allowed| {
                (url.scheme() == "https" && url.host_str() == Some(allowed.as_str()))
                    || (g.allow_http
                        && url.scheme() == "http"
                        && url.origin().ascii_serialization() == *allowed)
            })
        };
        let grant = self
            .grants
            .iter()
            .find(|g| g.capability == C::Network && permitted(g));
        if grant.is_none() {
            return Err(self.deny(C::Network));
        }
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return Err(w::HostError::InvalidInput(
                "URL credentials/fragments refused".into(),
            ));
        }
        if !self
            .grants
            .iter()
            .any(|g| g.capability == C::DataEgress && permitted(g))
        {
            return Err(self.deny(C::DataEgress));
        }
        let transport = self
            .context
            .as_ref()
            .and_then(|context| context.http.clone())
            .ok_or_else(|| w::HostError::Unavailable("module HTTP service unavailable".into()))?;
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| w::HostError::Unavailable("async runtime unavailable".into()))?;
        let maximum = self.limits.payload_bytes.min(1024 * 1024);
        let remaining = self
            .deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(5));
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let task = runtime.spawn(async move {
            let result = tokio::time::timeout(remaining, async move {
                let response = transport
                    .request(smith_module::ModuleHttpRequest {
                        method: match request.method {
                            w::HttpMethod::Get => smith_module::ModuleHttpMethod::Get,
                            w::HttpMethod::Head => smith_module::ModuleHttpMethod::Head,
                            w::HttpMethod::Post => smith_module::ModuleHttpMethod::Post,
                            w::HttpMethod::Put => smith_module::ModuleHttpMethod::Put,
                            w::HttpMethod::Patch => smith_module::ModuleHttpMethod::Patch,
                            w::HttpMethod::Delete => smith_module::ModuleHttpMethod::Delete,
                            w::HttpMethod::Options => smith_module::ModuleHttpMethod::Options,
                        },
                        url: request.url,
                        headers: request
                            .headers
                            .into_iter()
                            .map(|h| (h.name, h.value))
                            .collect(),
                        body: request.body,
                        max_response_bytes: maximum,
                        timeout: remaining,
                    })
                    .await
                    .map_err(|error| {
                        if error.kind
                            == agent_runtime_core::provider::ProviderErrorKind::MalformedStream
                        {
                            return w::HostError::PayloadTooLarge(w::PayloadLimit {
                                field: "body".into(),
                                maximum: maximum as u32,
                            });
                        }
                        w::HostError::Http(w::HttpFailure {
                            status: None,
                            message: "module HTTP transport failed".into(),
                        })
                    })?;
                if (300..400).contains(&response.status) {
                    return Err(w::HostError::Http(w::HttpFailure {
                        status: Some(response.status),
                        message: "redirect refused".into(),
                    }));
                }
                Ok(w::HttpResponse {
                    status: response.status,
                    headers: response
                        .headers
                        .into_iter()
                        .map(|(name, value)| w::HttpHeader { name, value })
                        .collect(),
                    body: response.body,
                })
            })
            .await
            .unwrap_or_else(|_| Err(w::HostError::Unavailable("HTTP deadline exceeded".into())));
            let _ = sender.send(result);
        });
        // Unlike Handle::block_on, this deadline also works when synchronous
        // mount blocks the caller's single-thread Tokio reactor. Abort prevents
        // a queued request from running after that mount has already failed.
        let response = match receiver.recv_timeout(remaining) {
            Ok(result) => result?,
            Err(_) => {
                task.abort();
                self.failure = Some(WasmError::Deadline);
                return Err(w::HostError::Unavailable("HTTP deadline exceeded".into()));
            }
        };
        self.check(&response)?;
        Ok(response)
    }
    fn kv_get(&mut self, key: String) -> Result<Option<Vec<u8>>, w::HostError> {
        if !self.granted(C::HostRead, "plugin-data") {
            return Err(self.deny(C::HostRead));
        }
        crate::limits::identifier(&key)
            .map_err(|_| w::HostError::InvalidInput("invalid key".into()))?;
        let file = w::FilePath {
            root: "plugin-data".into(),
            path: ".smith-kv.json".into(),
        };
        let bytes = match self.fs_read(file) {
            Ok(bytes) => bytes,
            Err(w::HostError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let values: BTreeMap<String, Vec<u8>> = serde_json::from_slice(&bytes)
            .map_err(|_| w::HostError::Io("invalid key-value store".into()))?;
        let result = values.get(&key).cloned();
        if result.as_ref().is_some_and(|v| v.len() > 64 * 1024) {
            return Err(w::HostError::PayloadTooLarge(w::PayloadLimit {
                field: "value".into(),
                maximum: 64 * 1024,
            }));
        }
        Ok(result)
    }
    fn kv_set(&mut self, entry: w::KvEntry) -> Result<(), w::HostError> {
        if !self.granted(C::HostWrite, "plugin-data") {
            return Err(self.deny(C::HostWrite));
        }
        self.check(&entry)?;
        crate::limits::identifier(&entry.key)
            .map_err(|_| w::HostError::InvalidInput("invalid key".into()))?;
        // HostWrite grants internal read-modify-write but never exposes read data.
        let file = w::FilePath {
            root: "plugin-data".into(),
            path: ".smith-kv.json".into(),
        };
        let dir = self.root(&file, true)?;
        let mut values: BTreeMap<String, Vec<u8>> =
            match read_store(&dir, &file.path, self.limits.payload_bytes) {
                Ok(bytes) if bytes.len() <= self.limits.payload_bytes => {
                    serde_json::from_slice(&bytes)
                        .map_err(|_| w::HostError::Io("invalid key-value store".into()))?
                }
                Ok(_) => {
                    return Err(w::HostError::PayloadTooLarge(w::PayloadLimit {
                        field: "store".into(),
                        maximum: self.limits.payload_bytes as u32,
                    }));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
                Err(e) => return Err(io(e)),
            };
        if values.len() >= 256 && !values.contains_key(&entry.key) {
            return Err(w::HostError::InvalidInput("key-value store full".into()));
        }
        values.insert(entry.key, entry.value);
        let bytes =
            serde_json::to_vec(&values).map_err(|_| w::HostError::Io("encoding failed".into()))?;
        self.fs_write(w::FileWrite { file, bytes })
    }
    fn session_history(&mut self, limit: u32) -> Result<w::HistorySnapshot, w::HostError> {
        if !self.granted(C::HostRead, "session-history") {
            return Err(self.deny(C::HostRead));
        }
        if !(1..=256).contains(&limit) {
            return Err(w::HostError::InvalidInput(
                "history limit must be 1..256".into(),
            ));
        }
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| w::HostError::Unavailable("session unavailable".into()))?;
        let history = self
            .context
            .as_ref()
            .and_then(|c| c.session_history.as_ref())
            .ok_or_else(|| w::HostError::Unavailable("history unavailable".into()))?;
        let mut result = Ok(Vec::new());
        history
            .with_history(session, &mut |messages| {
                result = crate::projection::messages(
                    &messages[messages.len().saturating_sub(limit as usize)..],
                );
            })
            .map_err(|_| w::HostError::Unavailable("history unavailable".into()))?;
        let snapshot = w::HistorySnapshot {
            messages: result
                .map_err(|_| w::HostError::InvalidInput("unrepresentable history".into()))?,
        };
        self.check(&snapshot)?;
        Ok(snapshot)
    }
}
fn io(error: std::io::Error) -> w::HostError {
    if error.kind() == std::io::ErrorKind::NotFound {
        w::HostError::NotFound
    } else {
        w::HostError::Io("confined file operation failed".into())
    }
}
fn tracing_log(level: w::LogLevel, message_bytes: usize) {
    match level {
        w::LogLevel::Trace => {
            tracing::trace!(message_bytes, "sandboxed module log (content withheld)")
        }
        w::LogLevel::Debug => {
            tracing::debug!(message_bytes, "sandboxed module log (content withheld)")
        }
        w::LogLevel::Info => {
            tracing::info!(message_bytes, "sandboxed module log (content withheld)")
        }
        w::LogLevel::Warning => {
            tracing::warn!(message_bytes, "sandboxed module log (content withheld)")
        }
        w::LogLevel::Error => {
            tracing::error!(message_bytes, "sandboxed module log (content withheld)")
        }
    }
}

fn open_regular(dir: &Dir, path: &str, write: bool) -> Result<cap_std::fs::File, w::HostError> {
    let mut options = OpenOptions::new();
    if write {
        options.write(true).create(true);
    } else {
        options.read(true);
    }
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(
            (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOFOLLOW).bits() as i32,
        );
    }
    let mut file = dir.open_with(path, &options).map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(w::HostError::InvalidInput("regular file required".into()));
    }
    if write {
        file.set_len(0).map_err(io)?;
        file.flush().map_err(io)?;
    }
    Ok(file)
}
fn read_store(dir: &Dir, path: &str, maximum: usize) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::new();
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(
            (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOFOLLOW).bits() as i32,
        );
    }
    let file = dir.open_with(path, &options)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("regular file required"));
    }
    file.take((maximum + 1) as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

impl Broker {
    fn result<T>(&mut self, result: Result<T, w::HostError>) -> Result<T, w::HostError> {
        if let Err(w::HostError::PayloadTooLarge(limit)) = &result {
            self.failure = Some(WasmError::OversizedPayload(limit.field.clone()));
        }
        result
    }
}
impl host::Host for Broker {
    fn log(&mut self, level: w::LogLevel, message: String) -> Result<(), w::HostError> {
        let result = Broker::log(self, level, message);
        self.result(result)
    }
    fn fs_read(&mut self, file: w::FilePath) -> Result<Vec<u8>, w::HostError> {
        let result = self.service_call(move |broker| Broker::fs_read(broker, file));
        self.result(result)
    }
    fn fs_write(&mut self, write: w::FileWrite) -> Result<(), w::HostError> {
        let result = self.service_call(move |broker| Broker::fs_write(broker, write));
        self.result(result)
    }
    fn http(&mut self, request: w::HttpRequest) -> Result<w::HttpResponse, w::HostError> {
        let result = Broker::http(self, request);
        self.result(result)
    }
    fn kv_get(&mut self, key: String) -> Result<Option<Vec<u8>>, w::HostError> {
        let result = self.service_call(move |broker| Broker::kv_get(broker, key));
        self.result(result)
    }
    fn kv_set(&mut self, entry: w::KvEntry) -> Result<(), w::HostError> {
        let result = self.service_call(move |broker| Broker::kv_set(broker, entry));
        self.result(result)
    }
    fn session_history(&mut self, limit: u32) -> Result<w::HistorySnapshot, w::HostError> {
        let result = self.service_call(move |broker| Broker::session_history(broker, limit));
        self.result(result)
    }
}
