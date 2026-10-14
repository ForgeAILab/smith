//! Turn ownership, private socket binding, and the dispatch boundary.

use std::collections::HashMap;
use std::fs::{self, DirBuilder};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream as ShutdownStream;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use agent_runtime_core::{content::ToolResultBlock, error::RuntimeError};
use async_trait::async_trait;
use rmcp::ServiceExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use super::server::BridgeServer;
use super::transport::{BridgeTransport, Connection};
use super::{BridgedSet, instructions};

#[async_trait]
pub(crate) trait BridgeDispatch: Send + Sync + 'static {
    /// Runs one bridged call through Smith's tool path. Dropping the returned
    /// future abandons the call: nothing may execute afterwards.
    async fn call(
        &self,
        name: String,
        arguments: serde_json::Value,
    ) -> Result<ToolResultBlock, RuntimeError>;
}

/// Keeps abandonment attached to the runtime future rather than a spawned task.
pub(crate) struct RuntimeBridgeDispatch(
    pub(crate) agent_runtime::agent::external::ExternalToolBridge,
);

#[async_trait]
impl BridgeDispatch for RuntimeBridgeDispatch {
    async fn call(
        &self,
        name: String,
        arguments: serde_json::Value,
    ) -> Result<ToolResultBlock, RuntimeError> {
        self.0.call(name, arguments).await
    }
}

#[derive(Clone)]
pub(super) struct Stop(watch::Sender<bool>);

impl Stop {
    pub(super) fn new() -> Self {
        Self(watch::channel(false).0)
    }

    pub(super) fn cancel(&self) {
        self.0.send_replace(true);
    }

    pub(super) async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|stopped| *stopped).await;
    }
}

type Sockets = Arc<Mutex<HashMap<u64, ShutdownStream>>>;
type OwnUid = Arc<dyn Fn() -> u32 + Send + Sync>;

pub(crate) struct BridgeEndpoint {
    directory: PrivateDirectory,
    socket: PathBuf,
    connected: Arc<AtomicBool>,
    stop: Stop,
    sockets: Sockets,
    task: Option<JoinHandle<()>>,
}

impl BridgeEndpoint {
    /// Binds the socket and starts serving. Fails before anything is spawned.
    pub(crate) fn bind(
        bridged: BridgedSet,
        dispatch: Arc<dyn BridgeDispatch>,
    ) -> Result<Self, RuntimeError> {
        let candidates = [
            std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
            std::env::var_os("TMPDIR").map(PathBuf::from),
            Some(PathBuf::from("/tmp")),
        ];
        let base = shortest_base(candidates.into_iter().flatten()).ok_or_else(|| {
            RuntimeError::config("no absolute bridge socket base directory exists")
        })?;
        Self::bind_inner(
            bridged,
            dispatch,
            &base,
            Arc::new(|| rustix::process::getuid().as_raw()),
            None,
        )
    }

    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket
    }

    #[cfg(test)]
    pub(crate) fn bind_at_base(
        bridged: BridgedSet,
        dispatch: Arc<dyn BridgeDispatch>,
        base: &Path,
    ) -> Result<Self, RuntimeError> {
        Self::bind_inner(
            bridged,
            dispatch,
            base,
            Arc::new(|| rustix::process::getuid().as_raw()),
            None,
        )
    }

    /// Whether a client has completed a handshake and asked for the tools.
    pub(crate) fn connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// Stops accepting, abandons calls in flight, and removes the directory.
    pub(crate) async fn close(mut self) {
        self.stop_connections();
        if let Some(task) = self.task.as_mut() {
            let _ = task.await;
        }
        self.task = None;
        self.directory.remove();
    }

    #[cfg(test)]
    pub(super) fn bind_test(
        bridged: BridgedSet,
        dispatch: Arc<dyn BridgeDispatch>,
        base: &Path,
        own_uid: OwnUid,
        instructions: Option<String>,
    ) -> Result<Self, RuntimeError> {
        Self::bind_inner(bridged, dispatch, base, own_uid, instructions)
    }

    fn bind_inner(
        bridged: BridgedSet,
        dispatch: Arc<dyn BridgeDispatch>,
        base: &Path,
        own_uid: OwnUid,
        override_instructions: Option<String>,
    ) -> Result<Self, RuntimeError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|error| {
            RuntimeError::config(format!("bridge needs a Tokio runtime: {error}"))
        })?;
        let guidance = override_instructions.unwrap_or_else(|| instructions(&bridged).text);
        // MCP's inputSchema is an object. Refuse an unrepresentable descriptor
        // before binding rather than silently changing a registered schema.
        let tools = BridgeServer::tools(&bridged)?;
        let directory = PrivateDirectory::create(base)?;
        let socket = directory.0.join("s");
        if socket.as_os_str().as_bytes().len() > socket_path_limit() {
            return Err(RuntimeError::config(format!(
                "bridge socket path {} exceeds sun_path limit of {} bytes",
                socket.display(),
                socket_path_limit()
            )));
        }
        let listener = UnixListener::bind(&socket)
            .map_err(|error| config_io("bind socket", &socket, error))?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
            .map_err(|error| config_io("set socket permissions", &socket, error))?;
        let connected = Arc::new(AtomicBool::new(false));
        let stop = Stop::new();
        let sockets = Arc::new(Mutex::new(HashMap::new()));
        let server = BridgeServer::new(tools, dispatch, guidance, connected.clone(), stop.clone());
        let task = runtime.spawn(accept_connections(
            listener,
            server,
            own_uid,
            stop.clone(),
            sockets.clone(),
        ));
        Ok(Self {
            directory,
            socket,
            connected,
            stop,
            sockets,
            task: Some(task),
        })
    }

    fn stop_connections(&self) {
        self.stop.cancel();
        // Keeping a duplicate descriptor lets Drop shut sockets synchronously;
        // aborting a Tokio task alone only drops its stream on its next poll.
        for stream in self.sockets.lock().expect("bridge sockets").values() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Drop for BridgeEndpoint {
    fn drop(&mut self) {
        self.stop_connections();
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.directory.remove();
    }
}

async fn accept_connections(
    listener: UnixListener,
    server: BridgeServer,
    own_uid: OwnUid,
    stop: Stop,
    sockets: Sockets,
) {
    let mut tasks = JoinSet::new();
    let mut next_id = 0u64;
    loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(connection) => connection,
                    Err(error) => {
                        tracing::warn!(%error, "bridge accept failed");
                        break;
                    }
                };
                // Credentials are checked before the stream is read or given
                // to MCP, including for reconnects within the same turn.
                if !stream.peer_cred().is_ok_and(|credentials| credentials.uid() == own_uid()) {
                    continue;
                }
                let stream = match stream.into_std() {
                    Ok(stream) => stream,
                    Err(_) => continue,
                };
                let shutdown = match stream.try_clone() {
                    Ok(stream) => stream,
                    Err(_) => continue,
                };
                let stream = match UnixStream::from_std(stream) {
                    Ok(stream) => stream,
                    Err(_) => continue,
                };
                let id = next_id;
                next_id += 1;
                sockets.lock().expect("bridge sockets").insert(id, shutdown);
                let guard = SocketGuard { id, sockets: sockets.clone() };
                let connection = Arc::new(Connection::new());
                let handler = server.for_connection(connection.clone());
                let transport = BridgeTransport::new(stream, connection, server.queue());
                let stop = stop.clone();
                tasks.spawn(async move {
                    let _guard = guard;
                    let service = tokio::select! {
                        biased;
                        _ = stop.cancelled() => return,
                        result = handler.serve(transport) => match result {
                            Ok(service) => service,
                            Err(error) => {
                                tracing::debug!(%error, "bridge handshake ended");
                                return;
                            }
                        },
                    };
                    let cancellation = service.cancellation_token();
                    let mut waiting = Box::pin(service.waiting());
                    tokio::select! {
                        biased;
                        _ = stop.cancelled() => {
                            cancellation.cancel();
                            let _ = waiting.await;
                        },
                        _ = &mut waiting => {},
                    }
                });
            }
        }
    }
    drop(listener);
    stop.cancel();
    for stream in sockets.lock().expect("bridge sockets").values() {
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
    while tasks.join_next().await.is_some() {}
}

struct SocketGuard {
    id: u64,
    sockets: Sockets,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Some(stream) = self
            .sockets
            .lock()
            .expect("bridge sockets")
            .remove(&self.id)
        {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

pub(super) fn shortest_base(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates
        .into_iter()
        .filter(|path| path.is_absolute() && path.is_dir())
        .min_by_key(|path| path.as_os_str().as_bytes().len())
}

pub(super) const fn socket_path_limit() -> usize {
    if cfg!(target_os = "macos") { 103 } else { 107 }
}

struct PrivateDirectory(PathBuf);

impl PrivateDirectory {
    fn create(base: &Path) -> Result<Self, RuntimeError> {
        for _ in 0..16 {
            let random = uuid::Uuid::new_v4().simple().to_string();
            let path = base.join(format!(".sb-{}", &random[..12]));
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {
                    let directory = Self(path);
                    // An unusually restrictive umask must not prevent the
                    // owner from reaching its own socket.
                    fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o700)).map_err(
                        |error| config_io("set directory permissions", &directory.0, error),
                    )?;
                    return Ok(directory);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(config_io("create directory", &path, error)),
            }
        }
        Err(RuntimeError::config(
            "cannot create a unique bridge socket directory",
        ))
    }

    fn remove(&self) {
        if let Err(error) = fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.0.display(), %error, "bridge directory cleanup failed");
        }
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        self.remove();
    }
}

fn config_io(action: &str, path: &Path, error: std::io::Error) -> RuntimeError {
    RuntimeError::config(format!(
        "bridge cannot {action} at {}: {error}",
        path.display()
    ))
}
