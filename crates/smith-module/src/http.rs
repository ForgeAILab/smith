//! Bounded HTTP service for modules; credentials and networking stay host-owned.
use agent_runtime_core::provider::ProviderError;
use std::{fmt, future::Future, pin::Pin, time::Duration};

/// Largest module HTTP body, independent of provider streaming limits.
pub const MODULE_HTTP_BODY_LIMIT: usize = 1024 * 1024;

/// Methods available through the module interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ModuleHttpMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Options,
}

/// A request already checked against the module's exact endpoint grants.
/// Debug deliberately excludes potentially secret URLs, headers and bodies.
#[derive(Clone)]
pub struct ModuleHttpRequest {
    /// Method to send.
    pub method: ModuleHttpMethod,
    /// Absolute endpoint; the sandbox checks its grant before calling.
    pub url: String,
    /// Guest-supplied headers, never provider credentials.
    pub headers: Vec<(String, String)>,
    /// Bytes to send, bounded by MODULE_HTTP_BODY_LIMIT.
    pub body: Vec<u8>,
    /// Caller may reduce the fixed body ceiling.
    pub max_response_bytes: usize,
    /// Caller may reduce the transport deadline.
    pub timeout: Duration,
}
impl fmt::Debug for ModuleHttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleHttpRequest")
            .field("method", &self.method)
            .finish_non_exhaustive()
    }
}
/// Complete bounded response. Redirects are errors, never automatically followed.
#[derive(Clone)]
pub struct ModuleHttpResponse {
    /// HTTP status, including non-success statuses.
    pub status: u16,
    /// Bounded response headers.
    pub headers: Vec<(String, String)>,
    /// Bounded response bytes.
    pub body: Vec<u8>,
}
impl fmt::Debug for ModuleHttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleHttpResponse")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}
/// Boxed request future using only standard and Agent Runtime types.
pub type ModuleHttpFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ModuleHttpResponse, ProviderError>> + Send + 'a>>;

/// Host-owned HTTP service; uses the same client as provider transport.
pub trait ModuleHttp: Send + Sync + fmt::Debug {
    /// Sends one bounded request. Implementations must refuse all redirects.
    fn request(&self, request: ModuleHttpRequest) -> ModuleHttpFuture<'_>;
}
