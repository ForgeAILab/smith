//! MCP handlers and endpoint-wide arrival ordering.

use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use agent_runtime_core::{
    content::{ContentPart, ToolResultBlock},
    error::RuntimeError,
};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, DiscoverRequestMethod,
    DiscoverResult, ErrorData, Implementation, InitializeRequestParams, InitializeResult,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler};
use tokio::sync::Notify;

use super::endpoint::{BridgeDispatch, Stop};
use super::transport::Connection;
use super::{BRIDGE_SERVER_NAME, BridgedSet};

#[derive(Default)]
pub(super) struct SerialQueue {
    next: AtomicU64,
    order: Mutex<VecDeque<u64>>,
    changed: Notify,
}

impl SerialQueue {
    pub(super) fn reserve(self: &Arc<Self>) -> QueuePermit {
        // Reservation happens in Transport::receive, before rmcp spawns a
        // handler, so executor scheduling cannot reorder arrived calls.
        let mut order = self.order.lock().expect("bridge call order");
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        order.push_back(id);
        QueuePermit {
            queue: self.clone(),
            id,
        }
    }
}

pub(super) struct QueuePermit {
    queue: Arc<SerialQueue>,
    id: u64,
}

impl QueuePermit {
    async fn ready(&self) {
        loop {
            let changed = self.queue.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.queue.order.lock().expect("bridge call order").front() == Some(&self.id) {
                return;
            }
            changed.await;
        }
    }
}

impl Drop for QueuePermit {
    fn drop(&mut self) {
        // Removing a cancelled queued call preserves its predecessor's place;
        // releasing a semaphore here would let a later call bypass that one.
        self.queue
            .order
            .lock()
            .expect("bridge call order")
            .retain(|id| *id != self.id);
        self.queue.changed.notify_waiters();
    }
}

#[derive(Clone)]
pub(super) struct BridgeServer {
    tools: Arc<Vec<Tool>>,
    dispatch: Arc<dyn BridgeDispatch>,
    info: ServerInfo,
    connected: Arc<AtomicBool>,
    stop: Stop,
    queue: Arc<SerialQueue>,
    connection: Arc<Connection>,
}

impl BridgeServer {
    pub(super) fn tools(bridged: &BridgedSet) -> Result<Vec<Tool>, RuntimeError> {
        bridged
            .iter()
            .map(|spec| {
                let schema = spec.input_schema.as_object().ok_or_else(|| {
                    RuntimeError::config(format!(
                        "bridge tool {} input schema is not an MCP schema object",
                        spec.name
                    ))
                })?;
                Ok(Tool::new(
                    spec.name.clone(),
                    spec.description.clone(),
                    schema.clone(),
                ))
            })
            .collect()
    }

    pub(super) fn new(
        tools: Vec<Tool>,
        dispatch: Arc<dyn BridgeDispatch>,
        guidance: String,
        connected: Arc<AtomicBool>,
        stop: Stop,
    ) -> Self {
        let mut info =
            ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
                Implementation::new(BRIDGE_SERVER_NAME, env!("CARGO_PKG_VERSION")),
            );
        if !guidance.is_empty() {
            info.instructions = Some(guidance);
        }
        Self {
            tools: Arc::new(tools),
            dispatch,
            info,
            connected,
            stop,
            queue: Arc::new(SerialQueue::default()),
            connection: Arc::new(Connection::new()),
        }
    }

    pub(super) fn queue(&self) -> Arc<SerialQueue> {
        self.queue.clone()
    }

    pub(super) fn for_connection(&self, connection: Arc<Connection>) -> Self {
        Self {
            connection,
            ..self.clone()
        }
    }
}

impl ServerHandler for BridgeServer {
    fn get_info(&self) -> ServerInfo {
        self.info.clone()
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request);
        self.connection.handshaken.store(true, Ordering::Release);
        Ok(self.get_info())
    }

    async fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> Result<DiscoverResult, ErrorData> {
        Err(ErrorData::method_not_found::<DiscoverRequestMethod>())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if self.connection.handshaken.load(Ordering::Acquire) {
            self.connected.store(true, Ordering::Release);
        }
        Ok(ListToolsResult::with_all_items(self.tools.as_ref().clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        mut context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let permit = context
            .extensions
            .remove::<Arc<QueuePermit>>()
            .ok_or_else(|| ErrorData::invalid_request("bridge call is no longer pending", None))?;
        if !self.tools.iter().any(|tool| tool.name == request.name) {
            return Err(ErrorData::invalid_params(
                format!("unknown bridge tool: {}", request.name),
                None,
            ));
        }
        let arguments = serde_json::Value::Object(request.arguments.unwrap_or_default());
        // Smith owns schema validation. Keeping get_tool's default bypasses
        // rmcp's schema validator and lets Smith return its ordinary refusal.
        let call = async {
            permit.ready().await;
            self.dispatch
                .call(request.name.into_owned(), arguments)
                .await
        };
        let result = tokio::select! {
            biased;
            _ = self.stop.cancelled() => return Err(ErrorData::internal_error("bridge turn ended", None)),
            _ = self.connection.stop.cancelled() => return Err(ErrorData::internal_error("bridge peer closed", None)),
            _ = context.ct.cancelled() => return Err(ErrorData::internal_error("bridge request cancelled", None)),
            result = call => result,
        };
        let result = match result {
            Ok(block) => map_result(block),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.message)]),
        };
        Ok(result.into())
    }
}

pub(super) fn map_result(block: ToolResultBlock) -> CallToolResult {
    let content = block
        .content
        .into_iter()
        .map(|part| match part {
            ContentPart::Text { text } => ContentBlock::text(text),
            ContentPart::Image { url, .. } => {
                if let Some((mime, data)) = url
                    .strip_prefix("data:")
                    .and_then(|value| value.split_once(";base64,"))
                {
                    ContentBlock::image(data, mime)
                } else {
                    ContentBlock::text(
                        "[Smith omitted image content: MCP requires an inline base64 image.]",
                    )
                }
            }
            ContentPart::Reasoning { .. } => {
                ContentBlock::text("[Smith omitted reasoning content.]")
            }
            ContentPart::ToolCall(_) => {
                ContentBlock::text("[Smith omitted nested tool call content.]")
            }
            ContentPart::ToolResult(_) => {
                ContentBlock::text("[Smith omitted nested tool result content.]")
            }
        })
        .collect();
    if block.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}
