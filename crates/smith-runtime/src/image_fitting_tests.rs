use super::*;

use agent_runtime::provider::fake::{FakeProvider, ScriptedStream};
use agent_runtime_core::content::{Message, ToolResultBlock};
use agent_runtime_core::ids::ToolCallId;
use agent_runtime_testkit::conformance::provider::call_ctx;
use agent_runtime_testkit::scenarios::stop_events;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use image::{DynamicImage, ImageFormat};

fn url(format: ImageFormat, width: u32) -> String {
    let mut bytes = std::io::Cursor::new(Vec::new());
    DynamicImage::new_rgb8(width, 20)
        .write_to(&mut bytes, format)
        .unwrap();
    format!(
        "data:{};base64,{}",
        format.to_mime_type(),
        STANDARD.encode(bytes.into_inner())
    )
}

fn image(url: &str) -> ContentPart {
    ContentPart::Image {
        url: url.into(),
        detail: Some("high".into()),
    }
}

fn provider(attempts: usize) -> (Arc<FakeProvider>, ImageFittingProvider) {
    let inner = Arc::new(FakeProvider::new(
        "m",
        Capabilities::basic_streaming(),
        (0..attempts)
            .map(|_| ScriptedStream::new(stop_events("seen")))
            .collect(),
    ));
    let fitting = ImageFittingProvider::new(
        inner.clone(),
        crate::factory::image_limits_for_provider_kind("xai-responses"),
    );
    (inner, fitting)
}

async fn send(provider: &ImageFittingProvider, request: ProviderRequest) {
    use futures_util::StreamExt;
    let (ctx, _cancel) = call_ctx();
    let events: Vec<_> = provider.stream(request, ctx).await.unwrap().collect().await;
    assert!(!events.is_empty());
}

#[tokio::test]
async fn image_fitting_xai_fits_messages_and_tool_results_preserving_history() {
    let (inner, fitting) = provider(1);
    let webp = url(ImageFormat::WebP, 100);
    let large = url(ImageFormat::Png, 3000);
    let small = url(ImageFormat::Png, 50);
    let remote = "https://example.com/image.webp";
    let messages = vec![
        Message::assistant(vec![image(&webp), image(&small), image(remote)]),
        Message::tool_result(ToolResultBlock {
            call_id: ToolCallId::new("read"),
            name: "read".into(),
            content: vec![image(&large)],
            is_error: false,
        }),
    ];
    send(
        &fitting,
        ProviderRequest::new(ModelId::new("m"), messages.clone()),
    )
    .await;
    let requests = inner.requests();
    let ContentPart::ToolResult(tool) = &requests[0].messages[1].content[0] else {
        panic!("tool result")
    };
    for part in [&requests[0].messages[0].content[0], &tool.content[0]] {
        let ContentPart::Image { url, detail } = part else {
            panic!("image")
        };
        assert_eq!(detail.as_deref(), Some("high"));
        assert!(url.starts_with("data:image/png;base64,"));
        let bytes = STANDARD.decode(url.split_once(',').unwrap().1).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Png);
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert!(decoded.width().max(decoded.height()) <= 2048);
    }
    assert_eq!(requests[0].messages[0].content[1], image(&small));
    assert_eq!(requests[0].messages[0].content[2], image(remote));
    assert_eq!(messages[0].content[0], image(&webp));
    let ContentPart::ToolResult(original) = &messages[1].content[0] else {
        panic!("tool result")
    };
    assert_eq!(original.content[0], image(&large));
}

#[tokio::test]
async fn image_fitting_repeated_requests_prepare_once() {
    let (inner, fitting) = provider(3);
    let request = ProviderRequest::new(
        ModelId::new("m"),
        vec![Message::assistant(vec![image(&url(
            ImageFormat::Png,
            3000,
        ))])],
    );
    for _ in 0..3 {
        send(&fitting, request.clone()).await;
    }
    assert_eq!(fitting.cache.lock().unwrap().preparations, 1);
    let requests = inner.requests();
    assert_eq!(requests[0].messages, requests[1].messages);
    assert_eq!(requests[1].messages, requests[2].messages);
}

#[tokio::test]
async fn image_fitting_corrupt_image_is_omitted_and_request_is_sent() {
    let (inner, fitting) = provider(2);
    let mut corrupt_png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    corrupt_png.extend(3000u32.to_be_bytes());
    corrupt_png.extend(20u32.to_be_bytes());
    let corrupt = format!("data:image/png;base64,{}", STANDARD.encode(corrupt_png));
    let request = ProviderRequest::new(
        ModelId::new("m"),
        vec![Message::assistant(vec![
            image(&corrupt),
            image("data:image/png;base64,!!!"),
            ContentPart::text("continue"),
        ])],
    );
    for _ in 0..2 {
        send(&fitting, request.clone()).await;
    }
    let requests = inner.requests();
    assert_eq!(requests.len(), 2);
    for part in &requests[0].messages[0].content[..2] {
        let text = part.as_text().unwrap();
        assert!(
            text.starts_with("[image omitted: unsupported or corrupt image:"),
            "{text}"
        );
        assert!(text.ends_with(']'));
    }
    assert_eq!(
        requests[0].messages[0].content[2],
        ContentPart::text("continue")
    );
    assert_eq!(fitting.cache.lock().unwrap().preparations, 2);
}

#[tokio::test]
async fn image_fitting_impossible_limits_are_omitted() {
    let (inner, mut fitting) = provider(1);
    fitting.limits.max_long_edge = 0;
    let request = ProviderRequest::new(
        ModelId::new("m"),
        vec![Message::assistant(vec![image(&url(ImageFormat::Png, 50))])],
    );
    send(&fitting, request).await;
    assert_eq!(
        inner.requests()[0].messages[0].content[0],
        ContentPart::text("[image omitted: image cannot fit the provider's limits]")
    );
}

#[test]
fn image_fitting_cache_evicts_lru_and_bounds_output_bytes() {
    let mut cache = ImageCache::default();
    for n in 0..CACHE_ENTRIES {
        cache.insert([n as u8; 32], Ok("x".into()));
    }
    assert!(cache.get(&[0; 32]).is_some());
    cache.insert([32; 32], Ok("x".into()));
    assert!(cache.get(&[1; 32]).is_none());
    assert!(cache.get(&[0; 32]).is_some());
    assert_eq!(cache.entries.len(), CACHE_ENTRIES);
    cache.insert([33; 32], Ok("x".repeat(CACHE_BYTES)));
    assert_eq!(cache.bytes, CACHE_BYTES);
    assert_eq!(cache.entries.len(), 1);
    cache.insert([34; 32], Ok("x".into()));
    assert_eq!(cache.bytes, 1);
    cache.insert([35; 32], Ok("x".repeat(CACHE_BYTES + 1)));
    assert_eq!(cache.bytes, 1);
    assert_eq!(cache.entries.len(), 1);
}

#[tokio::test]
async fn image_fitting_preserves_encoded_history_after_eviction_and_restart() {
    for family in [
        smith_tools::ImageProvider::OpenAi,
        smith_tools::ImageProvider::Anthropic,
        smith_tools::ImageProvider::Gemini,
        smith_tools::ImageProvider::Xai,
        smith_tools::ImageProvider::Other,
    ] {
        let limits = ImageLimits::for_provider(family);
        let make_provider = |attempts| {
            let inner = Arc::new(FakeProvider::new(
                "m",
                Capabilities::basic_streaming(),
                (0..attempts)
                    .map(|_| ScriptedStream::new(stop_events("seen")))
                    .collect(),
            ));
            let fitting = ImageFittingProvider::new(inner.clone(), limits.clone());
            (inner, fitting)
        };
        let source = vec![
            Message {
                role: agent_runtime_core::content::Role::User,
                content: vec![image(&url(ImageFormat::Png, 3500))],
            },
            Message::tool_result(ToolResultBlock {
                call_id: ToolCallId::new("reference"),
                name: "read".into(),
                content: vec![image(&url(ImageFormat::WebP, 100))],
                is_error: false,
            }),
        ];
        let request = ProviderRequest::new(ModelId::new("m"), source.clone());
        let (inner, fitting) = make_provider(CACHE_ENTRIES + 3);
        send(&fitting, request.clone()).await;
        let expected = inner.requests()[0].messages.clone();

        for width in 10..10 + CACHE_ENTRIES as u32 + 1 {
            send(
                &fitting,
                ProviderRequest::new(
                    ModelId::new("m"),
                    vec![Message::assistant(vec![image(&url(
                        ImageFormat::Png,
                        width,
                    ))])],
                ),
            )
            .await;
        }
        let before = fitting.cache.lock().unwrap().preparations;
        let mut appended = request;
        appended.messages.push(Message::user("continue comparing"));
        send(&fitting, appended).await;
        assert_eq!(fitting.cache.lock().unwrap().preparations, before + 2);
        assert_eq!(
            &inner.requests().last().unwrap().messages[..source.len()],
            expected.as_slice(),
            "encoded history changed after eviction for {family:?}"
        );

        let (cold_inner, cold_fitting) = make_provider(1);
        send(
            &cold_fitting,
            ProviderRequest::new(ModelId::new("m"), source.clone()),
        )
        .await;
        assert_eq!(cold_inner.requests()[0].messages, expected, "{family:?}");
        assert_eq!(source[0].content[0], image(&url(ImageFormat::Png, 3500)));
    }
}

#[test]
fn image_fitting_tile_resizing_is_byte_idempotent_across_provider_families() {
    let mut bytes = std::io::Cursor::new(Vec::new());
    DynamicImage::new_rgba8(2048, 1535)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    let source = format!(
        "data:image/png;base64,{}",
        STANDARD.encode(bytes.into_inner())
    );
    for family in [
        smith_tools::ImageProvider::OpenAi,
        smith_tools::ImageProvider::Anthropic,
        smith_tools::ImageProvider::Gemini,
        smith_tools::ImageProvider::Xai,
        smith_tools::ImageProvider::Other,
    ] {
        let limits = ImageLimits::for_provider(family);
        let first = prepare_image_data_url(&source, &limits).unwrap();
        let mut repeated = first.clone();
        for _ in 0..5 {
            repeated = prepare_image_data_url(&repeated, &limits).unwrap();
            assert_eq!(
                repeated, first,
                "repeated fitting changed bytes for {family:?}"
            );
        }
        assert_eq!(
            prepare_image_data_url(&source, &limits).unwrap(),
            first,
            "a cold preparation changed bytes for {family:?}"
        );
    }
}
