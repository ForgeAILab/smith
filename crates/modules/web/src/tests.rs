mod fetch_tests;
mod search_tests;
mod tool_tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn new(content_type: &str, body: impl AsRef<[u8]>) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".into(), content_type.into())],
            body: body.as_ref().to_vec(),
        }
    }

    fn redirect(location: &str) -> Self {
        Self {
            status: 302,
            headers: vec![("Location".into(), location.into())],
            body: Vec::new(),
        }
    }
}

struct Fixture {
    address: std::net::SocketAddr,
    task: JoinHandle<()>,
    requests: Mutex<mpsc::UnboundedReceiver<String>>,
}

impl Fixture {
    async fn new(routes: Vec<(&str, Reply)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let routes = Arc::new(
            routes
                .into_iter()
                .map(|(path, reply)| (path.to_owned(), reply))
                .collect::<BTreeMap<_, _>>(),
        );
        let (sender, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let routes = routes.clone();
                let sender = sender.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 4096];
                    loop {
                        let read = stream.read(&mut buffer).await.unwrap();
                        if read == 0 {
                            return;
                        }
                        request.extend_from_slice(&buffer[..read]);
                        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                        {
                            let headers = String::from_utf8_lossy(&request[..end]);
                            let length = headers
                                .lines()
                                .filter_map(|line| line.split_once(':'))
                                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                                .unwrap_or(0);
                            if request.len() >= end + 4 + length {
                                break;
                            }
                        }
                    }
                    let request = String::from_utf8(request).unwrap();
                    let path = request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .split('?')
                        .next()
                        .unwrap();
                    let reply = routes.get(path).cloned().unwrap_or_else(|| Reply {
                        status: 404,
                        ..Reply::new("text/plain", "missing fixture")
                    });
                    let _ = sender.send(request);
                    let mut response = format!(
                        "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n",
                        reply.status,
                        reply.body.len()
                    );
                    for (name, value) in reply.headers {
                        response.push_str(&format!("{name}: {value}\r\n"));
                    }
                    response.push_str("\r\n");
                    if stream.write_all(response.as_bytes()).await.is_ok() {
                        // A bounded reader may close before the fixture finishes.
                        let _ = stream.write_all(&reply.body).await;
                    }
                });
            }
        });
        Self {
            address,
            task,
            requests: Mutex::new(requests),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    async fn request(&self) -> String {
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            self.requests.lock().await.recv(),
        )
        .await
        .unwrap()
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
