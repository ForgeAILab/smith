//! Strict framing around rmcp's stream transport.

use std::io;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::task::{Context, Poll};

use rmcp::RoleServer;
use rmcp::model::{ClientJsonRpcMessage, ClientRequest, ErrorData, ServerJsonRpcMessage};
use rmcp::transport::{Transport, async_rw::AsyncRwTransport};
use tokio::io::{AsyncBufRead, AsyncRead, BufReader, ReadBuf};
use tokio::net::{
    UnixStream,
    unix::{OwnedReadHalf, OwnedWriteHalf},
};

use super::endpoint::Stop;
use super::server::SerialQueue;

pub(super) const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

pub(super) struct Connection {
    pub(super) stop: Stop,
    pub(super) handshaken: AtomicBool,
}

impl Connection {
    pub(super) fn new() -> Self {
        Self {
            stop: Stop::new(),
            handshaken: AtomicBool::new(false),
        }
    }

    fn abandon(&self) {
        self.stop.cancel();
    }
}

pub(super) struct BridgeTransport {
    inner: AsyncRwTransport<RoleServer, FrameReader, OwnedWriteHalf>,
    connection: Arc<Connection>,
    queue: Arc<SerialQueue>,
}

impl BridgeTransport {
    pub(super) fn new(
        stream: UnixStream,
        connection: Arc<Connection>,
        queue: Arc<SerialQueue>,
    ) -> Self {
        let (read, write) = stream.into_split();
        Self {
            inner: AsyncRwTransport::new_server(FrameReader::new(read), write),
            connection,
            queue,
        }
    }
}

impl Transport<RoleServer> for BridgeTransport {
    type Error = io::Error;

    fn send(
        &mut self,
        item: ServerJsonRpcMessage,
    ) -> impl Future<Output = Result<(), io::Error>> + Send + 'static {
        self.inner.send(item)
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        loop {
            let Some(mut message) = self.inner.receive().await else {
                self.connection.abandon();
                return None;
            };
            if let ClientJsonRpcMessage::Request(request) = &mut message {
                if !self.connection.handshaken.load(Ordering::Acquire)
                    && matches!(request.request, ClientRequest::DiscoverRequest(_))
                {
                    // rmcp treats a discover opener as an inline session even
                    // when its handler returns an error. Refuse it here so the
                    // recorded Claude fallback still negotiates via initialize.
                    let response = ServerJsonRpcMessage::error(
                        ErrorData::new(
                            rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                            "method not found: server/discover",
                            None,
                        ),
                        Some(request.id.clone()),
                    );
                    if self.inner.send(response).await.is_err() {
                        self.connection.abandon();
                        return None;
                    }
                    continue;
                }
                if let ClientRequest::CallToolRequest(call) = &mut request.request {
                    // The reservation travels with the request and drops even
                    // if rmcp refuses its metadata before calling our handler.
                    call.extensions.insert(Arc::new(self.queue.reserve()));
                }
            }
            return Some(message);
        }
    }

    async fn close(&mut self) -> Result<(), io::Error> {
        self.connection.abandon();
        self.inner.close().await
    }
}

impl Drop for BridgeTransport {
    fn drop(&mut self) {
        self.connection.abandon();
    }
}

struct FrameReader {
    read: BufReader<OwnedReadHalf>,
    frame: Vec<u8>,
    offset: usize,
    ready: bool,
}

impl FrameReader {
    fn new(read: OwnedReadHalf) -> Self {
        Self {
            read: BufReader::new(read),
            frame: Vec::new(),
            offset: 0,
            ready: false,
        }
    }
}

impl AsyncRead for FrameReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        while !this.ready {
            let bytes = match Pin::new(&mut this.read).poll_fill_buf(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(bytes)) => bytes,
            };
            if bytes.is_empty() {
                if this.frame.is_empty() {
                    return Poll::Ready(Ok(()));
                }
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "bridge frame ended before newline",
                )));
            }
            let newline = bytes.iter().position(|byte| *byte == b'\n');
            let count = newline
                .map_or(bytes.len(), |index| index + 1)
                .min(MAX_FRAME_BYTES + 1 - this.frame.len());
            this.frame.extend_from_slice(&bytes[..count]);
            Pin::new(&mut this.read).consume(count);
            let complete = this.frame.last() == Some(&b'\n');
            let payload_len = this.frame.len() - usize::from(complete);
            if payload_len > MAX_FRAME_BYTES {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "bridge frame exceeds 8 MiB",
                )));
            }
            if complete {
                // rmcp ignores syntax errors. Validate before exposing a byte
                // so a malformed line terminates only this connection.
                if let Err(error) = serde_json::from_slice::<serde_json::Value>(&this.frame) {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("malformed bridge frame: {error}"),
                    )));
                }
                this.ready = true;
            }
        }
        let count = output.remaining().min(this.frame.len() - this.offset);
        output.put_slice(&this.frame[this.offset..this.offset + count]);
        this.offset += count;
        if this.offset == this.frame.len() {
            this.frame.clear();
            this.offset = 0;
            this.ready = false;
        }
        Poll::Ready(Ok(()))
    }
}
