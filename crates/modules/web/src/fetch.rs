use std::net::{IpAddr, SocketAddr};

use agent_runtime_core::content::ContentPart;
use agent_runtime_core::tool::{ToolContent, ToolOutcome};
use base64::Engine as _;
use futures_util::StreamExt;
use reqwest::{Client, Response, redirect::Policy};
use url::{Host, Url};

use crate::{BODY_LIMIT, TIMEOUT, USER_AGENT};

pub(crate) fn validate_url(raw: &str) -> Result<Url, String> {
    if raw.len() > 2048 {
        return Err("URL must be at most 2048 bytes".into());
    }
    let mut url = Url::parse(raw).map_err(|_| "invalid absolute HTTP URL".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err("URL must use http or https with a host".into());
    }
    let authority = raw
        .split("://")
        .nth(1)
        .unwrap_or("")
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    if !url.username().is_empty() || url.password().is_some() || authority.contains('@') {
        return Err("URL must not contain userinfo".into());
    }
    url.set_fragment(None);
    if url.as_str().len() > 2048 {
        return Err("URL must be at most 2048 bytes".into());
    }
    Ok(url)
}

pub(crate) fn is_private(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || a == 0
                || (a == 100 && (64..=127).contains(&b))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4() {
                return is_private(IpAddr::V4(v4));
            }
            let first = ip.segments()[0];
            ip.is_loopback()
                || ip.is_unspecified()
                || first & 0xfe00 == 0xfc00
                || first & 0xffc0 == 0xfe80
        }
    }
}

pub(crate) fn vet_addresses(
    addresses: &[SocketAddr],
    allow_private: bool,
) -> Result<SocketAddr, String> {
    if !allow_private && addresses.iter().any(|address| is_private(address.ip())) {
        return Err("destination is a private address; allow_private_networks is disabled".into());
    }
    // Prefer IPv4 when present so IPv4-only localhost servers remain useful
    // on hosts whose resolver lists ::1 first. Every answer was checked above.
    addresses
        .iter()
        .find(|address| address.is_ipv4())
        .or_else(|| addresses.first())
        .copied()
        .ok_or_else(|| "DNS returned no addresses".into())
}

#[async_trait::async_trait]
pub(crate) trait Resolver: Send + Sync {
    async fn resolve(&self, url: &Url, allow_private: bool) -> Result<SocketAddr, String>;
}

struct SystemResolver;

#[async_trait::async_trait]
impl Resolver for SystemResolver {
    async fn resolve(&self, url: &Url, allow_private: bool) -> Result<SocketAddr, String> {
        let port = url.port_or_known_default().ok_or("URL has no HTTP port")?;
        let addresses = match url.host().ok_or("URL has no host")? {
            Host::Ipv4(ip) => vec![SocketAddr::new(ip.into(), port)],
            Host::Ipv6(ip) => vec![SocketAddr::new(ip.into(), port)],
            Host::Domain(host) => tokio::net::lookup_host((host, port))
                .await
                .map_err(|_| "destination DNS resolution failed".to_owned())?
                .collect(),
        };
        vet_addresses(&addresses, allow_private)
    }
}

pub(crate) fn client() -> reqwest::ClientBuilder {
    client_without_user_agent().user_agent(USER_AGENT)
}

pub(crate) fn client_without_user_agent() -> reqwest::ClientBuilder {
    Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .timeout(TIMEOUT)
}

pub(crate) async fn read_body(response: Response) -> Result<(Vec<u8>, bool), String> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "HTTP body read failed".to_owned())?;
        let remaining = BODY_LIMIT - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if body.len() == BODY_LIMIT {
            // Stop immediately at the cap, even if EOF would follow it.
            return Ok((body, true));
        }
    }
    Ok((body, false))
}

pub(crate) async fn fetch(raw: &str, allow_private: bool) -> Result<ToolOutcome, String> {
    tokio::time::timeout(TIMEOUT, fetch_inner(raw, allow_private, &SystemResolver))
        .await
        .map_err(|_| "web_fetch timed out after 30 seconds".to_owned())?
}

pub(crate) async fn fetch_inner(
    raw: &str,
    allow_private: bool,
    resolver: &dyn Resolver,
) -> Result<ToolOutcome, String> {
    let mut url = validate_url(raw)?;
    for hop in 0..=5 {
        let address = resolver.resolve(&url, allow_private).await?;
        // One client per hop prevents pool reuse and pins only the vetted DNS
        // answer. IP literals connect directly to the same checked address.
        let client = client()
            .resolve(url.host_str().ok_or("URL has no host")?, address)
            .build()
            .map_err(|_| "could not create web HTTP client".to_owned())?;
        let response = client.get(url.clone())
            .header(reqwest::header::ACCEPT, "text/html, text/*, application/json, application/xml, image/png, image/jpeg, image/gif, image/webp")
            .send().await.map_err(|_| "web_fetch HTTP request failed".to_owned())?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            if hop == 5 {
                return Err("web_fetch exceeded 5 redirects".into());
            }
            url = redirect_url(&url, &response)?;
            continue;
        }
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let is_image = matches!(
            mime.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        );
        if !is_image
            && !mime.starts_with("text/")
            && !matches!(mime.as_str(), "application/json" | "application/xml")
        {
            return Err(format!("unsupported content type: {content_type}"));
        }
        let (bytes, truncated) = read_body(response).await?;
        return render(&url, status, &content_type, &mime, bytes, truncated);
    }
    unreachable!("redirect limit returns before the last iteration ends")
}

pub(crate) fn redirect_url(url: &Url, response: &Response) -> Result<Url, String> {
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or("redirect has no valid Location")?;
    if location.starts_with("//") {
        return validate_url(&format!("{}:{location}", url.scheme()));
    }
    if Url::parse(location).is_ok() {
        return validate_url(location);
    }
    let next = url
        .join(location)
        .map_err(|_| "redirect has an invalid URL".to_owned())?;
    validate_url(next.as_str())
}

fn render(
    url: &Url,
    status: u16,
    content_type: &str,
    mime: &str,
    bytes: Vec<u8>,
    truncated: bool,
) -> Result<ToolOutcome, String> {
    let is_image = mime.starts_with("image/");
    let (title, body) = if is_image {
        (String::new(), String::new())
    } else {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if mime == "text/html" {
            crate::html::convert(&text, url)?
        } else {
            (String::new(), text)
        }
    };
    let note = if truncated {
        "\n[Truncated at the 5 MiB download limit.]"
    } else {
        ""
    };
    let header = format!(
        "URL: {url} | Status: {status} | Content-Type: {content_type} | Title: {title}\n\nContent from {url} (external, untrusted):\n"
    );
    let text = format!("{header}{body}{note}");
    let value = serde_json::json!({"url":url.as_str(), "status":status, "content_type":content_type, "title":title, "truncated":truncated, "text":text});
    let mut content = vec![ContentPart::text(text)];
    if is_image {
        content.push(ContentPart::Image {
            url: format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
            detail: None,
        });
    }
    Ok(ToolOutcome {
        value,
        content: ToolContent::inline(content),
        is_error: false,
    })
}
