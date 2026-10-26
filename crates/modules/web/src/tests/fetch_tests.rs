use agent_runtime_core::content::ContentPart;
use url::Url;

use super::{Fixture, Reply};
use crate::{BODY_LIMIT, USER_AGENT, fetch};

fn text(outcome: &agent_runtime_core::tool::ToolOutcome) -> &str {
    match &outcome.content.as_inline().unwrap()[0] {
        ContentPart::Text { text } => text,
        _ => panic!("expected text header"),
    }
}

#[tokio::test]
async fn html_to_markdown_prefers_main_and_keeps_links_without_boilerplate() {
    let source = format!(
        "<title>Docs &amp; API</title><body><nav>BAD NAV</nav><p>OUTSIDE</p><main><h1>Guide</h1><p>{}</p><a href='/api'>API</a><script>BAD SCRIPT</script><style>BAD STYLE</style><footer>BAD FOOTER</footer><aside>BAD ASIDE</aside><noscript>BAD NOSCRIPT</noscript><svg><text>BAD SVG</text></svg></main></body>",
        "Useful documentation. ".repeat(20)
    );
    let server = Fixture::new(vec![(
        "/docs",
        Reply::new("text/html; charset=utf-8", source),
    )])
    .await;
    let outcome = fetch::fetch(&server.url("/docs"), true).await.unwrap();
    let result = text(&outcome);
    assert!(result.starts_with(&format!(
        "URL: {} | Status: 200 | Content-Type: text/html",
        server.url("/docs")
    )));
    assert!(result.contains("Title: Docs & API"));
    assert!(result.contains("(external, untrusted):"));
    assert!(result.contains("# Guide"));
    assert!(result.contains(&format!("[API]({})", server.url("/api"))));
    assert!(!result.contains("BAD"));
    assert!(!result.contains("OUTSIDE"));
    let request = server.request().await.to_lowercase();
    assert!(request.starts_with("get /docs http/1.1"));
    assert!(request.contains(&format!("user-agent: {}", USER_AGENT.to_lowercase())));
    assert!(!request.contains("cookie:"));
    assert!(!request.contains("authorization:"));
}

#[tokio::test]
async fn short_extraction_falls_back_to_whole_body() {
    let server = Fixture::new(vec![("/", Reply::new("text/html", "<body><main>Tiny</main><p>Actual content outside main</p><a href='/next'>Next</a><nav>Hidden</nav></body>"))]).await;
    let outcome = fetch::fetch(&server.url("/"), true).await.unwrap();
    assert!(text(&outcome).contains("Actual content outside main"));
    assert!(text(&outcome).contains("[Next]"));
    assert!(!text(&outcome).contains("Hidden"));
}

#[test]
fn main_precedes_article_and_article_precedes_body() {
    let base = Url::parse("https://docs.rs/guide/").unwrap();
    let article = format!("<article>ARTICLE {}</article>", "a".repeat(200));
    let main = format!("<main>MAIN {}</main>", "m".repeat(200));
    let (_, body) =
        crate::html::convert(&format!("<body>OUTSIDE{article}{main}</body>"), &base).unwrap();
    assert!(body.contains("MAIN"));
    assert!(!body.contains("ARTICLE"));
    assert!(!body.contains("OUTSIDE"));
    let (_, body) = crate::html::convert(&format!("<body>OUTSIDE{article}</body>"), &base).unwrap();
    assert!(body.contains("ARTICLE"));
    assert!(!body.contains("OUTSIDE"));
}

#[test]
fn fallback_threshold_counts_cleaned_unicode_text_and_keeps_safe_links() {
    let base = Url::parse("https://docs.rs/guide/").unwrap();
    for count in [199, 200] {
        let source = format!(
            "<head><base href='https://untrusted.example/'></head><body><main><nav>{}</nav>{}</main><p>OUTSIDE</p><a href='../next'>Next</a><a href='javascript:alert(1)'>Unsafe</a></body>",
            "BOILERPLATE".repeat(50),
            "é".repeat(count)
        );
        let (_, body) = crate::html::convert(&source, &base).unwrap();
        assert_eq!(body.contains("OUTSIDE"), count < 200);
        if count < 200 {
            assert!(body.contains("[Next](https://docs.rs/next)"));
        }
        assert!(!body.contains("BOILERPLATE"));
        assert!(!body.contains("untrusted.example"));
        assert!(!body.contains("javascript:"));
    }
}

#[test]
fn page_chrome_hidden_elements_and_image_urls_are_left_out() {
    let base = Url::parse("https://docs.rs/guide/").unwrap();
    let source = format!(
        "<body><header><a href='/'>BAD SITE</a></header>\
         <div role='navigation'>BAD ROLE</div><div role='note search'>BAD SEARCH</div>\
         <div hidden>BAD HIDDEN</div><p aria-hidden='true'>BAD ARIA</p>\
         <p style='color: red; DISPLAY : none !important'>BAD STYLE</p>\
         <button>BAD BUTTON</button><iframe>BAD FRAME</iframe><dialog>BAD DIALOG</dialog>\
         <div><header><h1>Post title</h1><p>By the team</p></header>\
         <section><header>Section header</header></section>\
         <p>{}</p><img alt=' A  diagram of the flow ' src='data:image/png;base64,AAAA'>\
         <img src='/decor.png'></div></body>",
        "Useful prose. ".repeat(20)
    );
    let (_, body) = crate::html::convert(&source, &base).unwrap();
    assert!(!body.contains("BAD"), "{body}");
    for kept in [
        "# Post title",
        "By the team",
        "Section header",
        "A diagram of the flow",
    ] {
        assert!(body.contains(kept), "{kept}: {body}");
    }
    assert!(!body.contains("data:"));
    assert!(!body.contains("decor.png"));
}

#[test]
fn links_without_text_go_and_links_into_the_page_keep_their_text() {
    let base = Url::parse("https://docs.rs/guide/").unwrap();
    let source = format!(
        "<main><h2 id='new'><a href='#new'></a>What is new</h2>\
         <h3 id='z'><a href='#z' aria-label='Link'>\u{200b}</a>Zero width</h3>\
         <p>{} <a href='#toc'>Contents</a> <a href='https://docs.rs/guide/'>This page</a> \
         <a href='javascript:x()'>Unsafe</a> <a name='here'>Named</a> \
         <a href='/other#part'>Other page</a></p></main>",
        "Useful prose. ".repeat(20)
    );
    let (_, body) = crate::html::convert(&source, &base).unwrap();
    assert!(body.contains("## What is new"), "{body}");
    assert!(body.contains("### Zero width"), "{body}");
    assert!(!body.contains("#new") && !body.contains("#z") && !body.contains("[]"));
    for text in ["Contents", "This page", "Unsafe", "Named"] {
        assert!(body.contains(text), "{text}: {body}");
        assert!(!body.contains(&format!("[{text}]")), "{text}: {body}");
    }
    assert!(
        body.contains("[Other page](https://docs.rs/other#part)"),
        "{body}"
    );
}

#[test]
fn a_page_wrapped_in_one_form_keeps_its_content() {
    let base = Url::parse("https://docs.rs/guide/").unwrap();
    let source = format!(
        "<body><form id='page'><main><p>{}</p><input type='hidden' value='BAD STATE'></main></form></body>",
        "Useful prose. ".repeat(20)
    );
    let (_, body) = crate::html::convert(&source, &base).unwrap();
    assert!(body.contains("Useful prose."), "{body}");
    assert!(!body.contains("BAD"), "{body}");
}

#[tokio::test]
async fn truncation_stops_at_five_mib_and_keeps_the_prefix() {
    let mut body = vec![b'x'; BODY_LIMIT];
    body.extend_from_slice(b"SHOULD NOT BE READ");
    let server = Fixture::new(vec![("/", Reply::new("text/plain", body))]).await;
    let outcome = fetch::fetch(&server.url("/"), true).await.unwrap();
    assert_eq!(outcome.value["truncated"], true);
    assert!(text(&outcome).contains("Truncated at the 5 MiB"));
    assert!(!text(&outcome).contains("SHOULD NOT"));
    let body = text(&outcome)
        .split_once("(external, untrusted):\n")
        .unwrap()
        .1;
    assert!(
        body.as_bytes()[..BODY_LIMIT]
            .iter()
            .all(|byte| *byte == b'x')
    );
    assert!(body[BODY_LIMIT..].starts_with("\n[Truncated"));
}

#[tokio::test]
async fn unsupported_type_names_the_type_and_supported_images_are_rich_parts() {
    let server = Fixture::new(vec![
        ("/zip", Reply::new("application/zip", "PK")),
        ("/image", Reply::new("image/png", [137, 80, 78, 71])),
        ("/svg", Reply::new("image/svg+xml", "<svg/>")),
    ])
    .await;
    assert!(
        fetch::fetch(&server.url("/zip"), true)
            .await
            .unwrap_err()
            .contains("application/zip")
    );
    assert!(
        fetch::fetch(&server.url("/svg"), true)
            .await
            .unwrap_err()
            .contains("image/svg+xml")
    );
    let outcome = fetch::fetch(&server.url("/image"), true).await.unwrap();
    assert!(
        matches!(&outcome.content.as_inline().unwrap()[1], ContentPart::Image { url, .. } if url == "data:image/png;base64,iVBORw==")
    );
    assert!(text(&outcome).contains("external, untrusted"));
}

#[tokio::test]
async fn plain_text_json_and_xml_pass_through_even_on_http_errors() {
    for mime in ["text/plain", "application/json", "application/xml"] {
        let server = Fixture::new(vec![(
            "/",
            Reply {
                status: 404,
                ..Reply::new(mime, "exact body\n<>&")
            },
        )])
        .await;
        let outcome = fetch::fetch(&server.url("/"), true).await.unwrap();
        assert!(text(&outcome).ends_with("exact body\n<>&"));
        assert_eq!(outcome.value["status"], 404);
    }
}

#[tokio::test]
async fn the_page_body_is_carried_once() {
    let server = Fixture::new(vec![("/", Reply::new("text/plain", "only-once-marker"))]).await;
    let outcome = fetch::fetch(&server.url("/"), true).await.unwrap();
    let serialized = serde_json::to_string(&outcome).unwrap();
    assert_eq!(
        serialized.matches("only-once-marker").count(),
        1,
        "{serialized}"
    );
    assert_eq!(outcome.value["url"], server.url("/"));
    assert_eq!(outcome.value["status"], 200);
    assert_eq!(outcome.value["truncated"], false);
}

#[test]
fn url_validation_and_private_guard_cover_literal_and_mixed_dns_answers() {
    for url in [
        "file:///etc/passwd",
        "ftp://example.com/",
        "http://a:b@example.com/",
        "http://@example.com/",
        "relative",
        &format!("https://example.com/{}", "x".repeat(2048)),
    ] {
        assert!(fetch::validate_url(url).is_err(), "{url}");
    }
    for address in [
        "127.0.0.1",
        "10.0.0.5",
        "172.16.0.1",
        "192.168.1.1",
        "100.64.0.1",
        "100.127.255.255",
        "169.254.169.254",
        "0.1.2.3",
        "::",
        "::1",
        "fc00::1",
        "fdff::1",
        "fe80::1",
        "febf::1",
        "::ffff:127.0.0.1",
        "::127.0.0.1",
        "::ffff:169.254.169.254",
    ] {
        let address = std::net::SocketAddr::new(address.parse().unwrap(), 80);
        assert!(
            fetch::vet_addresses(&[address], false)
                .unwrap_err()
                .contains("private address")
        );
        assert_eq!(fetch::vet_addresses(&[address], true).unwrap(), address);
    }
    let public = "93.184.216.34:80".parse().unwrap();
    let private = "10.0.0.1:80".parse().unwrap();
    assert_eq!(fetch::vet_addresses(&[public], false).unwrap(), public);
    assert!(fetch::vet_addresses(&[public, private], false).is_err());
    assert!(fetch::vet_addresses(&[], true).is_err());
}

#[tokio::test]
async fn metadata_and_local_servers_are_refused_before_connecting_unless_allowed() {
    let server = Fixture::new(vec![("/", Reply::new("text/plain", "local docs"))]).await;
    assert!(
        fetch::fetch(&server.url("/"), false)
            .await
            .unwrap_err()
            .contains("private address")
    );
    assert!(
        fetch::fetch("http://169.254.169.254/latest/meta-data/", false)
            .await
            .unwrap_err()
            .contains("private address")
    );
    assert!(text(&fetch::fetch(&server.url("/"), true).await.unwrap()).contains("local docs"));
    let localhost = server.url("/").replace("127.0.0.1", "localhost");
    assert!(
        fetch::fetch(&localhost, false)
            .await
            .unwrap_err()
            .contains("private address")
    );
    assert!(text(&fetch::fetch(&localhost, true).await.unwrap()).contains("local docs"));
}

struct RedirectResolver(std::net::SocketAddr);

#[async_trait::async_trait]
impl fetch::Resolver for RedirectResolver {
    async fn resolve(&self, url: &Url, allow: bool) -> Result<std::net::SocketAddr, String> {
        if url.host_str() == Some("public.fixture") {
            // Deterministic public first hop, served by a loopback fixture.
            fetch::vet_addresses(&["93.184.216.34:80".parse().unwrap()], allow)?;
            Ok(self.0)
        } else {
            let address = std::net::SocketAddr::new(url.host_str().unwrap().parse().unwrap(), 80);
            fetch::vet_addresses(&[address], allow)
        }
    }
}

#[tokio::test]
async fn redirect_from_public_first_hop_is_rechecked_and_private_target_refused() {
    let server = Fixture::new(vec![("/", Reply::redirect("http://10.0.0.5/admin"))]).await;
    let url = format!("http://public.fixture:{}/", server.address.port());
    let error = fetch::fetch_inner(&url, false, &RedirectResolver(server.address))
        .await
        .unwrap_err();
    assert!(error.contains("private address"));
    let request = server.request().await;
    assert!(request.starts_with("GET / HTTP/1.1"));
    assert!(
        request
            .to_lowercase()
            .contains(&format!("host: public.fixture:{}", server.address.port()))
    );
}

#[tokio::test]
async fn redirects_are_manual_bounded_and_validate_each_location() {
    let server = Fixture::new(vec![
        ("/loop", Reply::redirect("/loop")),
        ("/bad", Reply::redirect("file:///etc/passwd")),
        ("/good", Reply::redirect("/done")),
        ("/done", Reply::new("text/plain", "done")),
    ])
    .await;
    assert!(
        fetch::fetch(&server.url("/loop"), true)
            .await
            .unwrap_err()
            .contains("5 redirects")
    );
    assert!(
        fetch::fetch(&server.url("/bad"), true)
            .await
            .unwrap_err()
            .contains("http or https")
    );
    let result = fetch::fetch(&server.url("/good"), true).await.unwrap();
    assert_eq!(result.value["url"], server.url("/done"));
    assert!(text(&result).ends_with("done"));
}

#[tokio::test]
async fn exactly_five_redirects_are_allowed_and_empty_userinfo_redirects_are_refused() {
    let server = Fixture::new(vec![
        ("/0", Reply::redirect("/1")),
        ("/1", Reply::redirect("/2")),
        ("/2", Reply::redirect("/3")),
        ("/3", Reply::redirect("/4")),
        ("/4", Reply::redirect("/5")),
        ("/5", Reply::new("text/plain", "five redirects")),
        ("/userinfo", Reply::redirect("http://@example.com/")),
        ("/relative-userinfo", Reply::redirect("//@example.com/")),
    ])
    .await;
    let result = fetch::fetch(&server.url("/0"), true).await.unwrap();
    assert_eq!(result.value["url"], server.url("/5"));
    for path in ["/userinfo", "/relative-userinfo"] {
        assert!(
            fetch::fetch(&server.url(path), true)
                .await
                .unwrap_err()
                .contains("userinfo")
        );
    }
}
