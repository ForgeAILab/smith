use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

mod flows;
mod input;
mod rendering;

fn setup_app(
    mode: SetupMode,
    providers: Vec<ResourceEntry>,
    models: Vec<ResourceEntry>,
) -> SetupApp {
    let quick_start = SetupQuickStart {
        provider: "zai".into(),
        endpoint: "https://api.z.ai/api/coding/paas/v4".into(),
        model: "glm-5.2".into(),
        model_label: "GLM-5.2".into(),
        limits: SetupModelLimits {
            context_tokens: 1_000_000,
            max_input_tokens: 1_000_000,
            max_output_tokens: 131_072,
        },
        request_output_tokens: 32_768,
        output_reserve: 32_768,
        profile: "glm".into(),
        catalog_revision: 5,
    };
    let entries = setup_entries(&quick_start);
    SetupApp::new(
        mode,
        providers,
        models,
        quick_start,
        entries,
        setup_prompts(),
    )
}

fn setup_prompts() -> SetupPrompts {
    SetupPrompts {
        provider_name_help: "A stable local name, for example openrouter".into(),
        endpoint_help: "OpenAI-compatible base, for example https://openrouter.ai/api/v1".into(),
        environment_variable_error:
            "Use an environment variable such as ZAI_API_KEY (letters, digits, underscore).".into(),
    }
}

fn setup_entries(quick_start: &SetupQuickStart) -> Vec<SetupEntry> {
    vec![
        SetupEntry {
            id: "glm".into(),
            label: "Quick start with GLM".into(),
            detail: format!("Z.AI · {}", quick_start.model_label),
            flow: SetupFlow::QuickKey {
                kind: SetupQuickKey::Glm,
                provider: quick_start.provider.clone(),
                endpoint: quick_start.endpoint.clone(),
                review: SetupKeyReview {
                    action: "action: Quick start with GLM".into(),
                    provider: format!("provider: {} (openai-compatible)", quick_start.provider),
                    endpoint: format!("endpoint: {}", quick_start.endpoint),
                    profile: format!("default profile: {}", quick_start.profile),
                    reasoning: None,
                },
                catalog_models: false,
            },
        },
        custom_entry(
            "add-provider",
            "Add provider",
            None,
            None,
            SetupProviderKind::OpenAiCompatible,
            false,
        ),
        SetupEntry {
            id: "google".into(),
            label: "Connect Google Gemini".into(),
            detail: "AI Studio API key · native Gemini Interactions".into(),
            flow: SetupFlow::QuickKey {
                kind: SetupQuickKey::Google,
                provider: "google".into(),
                endpoint: "https://generativelanguage.googleapis.com/v1beta".into(),
                review: SetupKeyReview {
                    action: "action: Connect Google Gemini".into(),
                    provider: "provider: google (native gemini-interactions)".into(),
                    endpoint: "endpoint: fixed Google Gemini Interactions endpoint".into(),
                    profile: "default profile: gemini".into(),
                    reasoning: Some(
                        "reasoning: native Gemini thinking levels from the selected catalog model"
                            .into(),
                    ),
                },
                catalog_models: false,
            },
        },
        custom_entry(
            "anthropic-messages",
            "Anthropic Messages API",
            Some("anthropic"),
            Some("https://api.anthropic.com/v1"),
            SetupProviderKind::AnthropicMessages,
            false,
        ),
        custom_entry(
            "openrouter",
            "Connect OpenRouter",
            Some("openrouter"),
            Some("https://openrouter.ai/api/v1"),
            SetupProviderKind::OpenAiCompatible,
            false,
        ),
    ]
}

fn custom_entry(
    id: &str,
    label: &str,
    provider: Option<&str>,
    endpoint: Option<&str>,
    kind: SetupProviderKind,
    catalog_models: bool,
) -> SetupEntry {
    let (review_action, adapter) = match kind {
        SetupProviderKind::OpenAiCompatible => (
            "action: Add OpenAI-compatible provider",
            "openai-compatible",
        ),
        SetupProviderKind::AnthropicMessages => (
            "action: Add Anthropic Messages provider",
            "anthropic-messages",
        ),
    };
    SetupEntry {
        id: id.into(),
        label: label.into(),
        detail: String::new(),
        flow: SetupFlow::CustomEndpoint {
            kind,
            provider: provider.map(str::to_owned),
            endpoint: endpoint.map(str::to_owned),
            review_action: review_action.into(),
            adapter: adapter.into(),
            catalog_models,
        },
    }
}

fn direct_provider_mode(id: &str) -> SetupMode {
    let app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    let mut flow = app
        .provider_actions
        .into_iter()
        .find(|entry| entry.id == id)
        .expect("test entry")
        .flow;
    match &mut flow {
        SetupFlow::QuickKey { catalog_models, .. }
        | SetupFlow::CustomEndpoint { catalog_models, .. } => *catalog_models = true,
        _ => panic!("the direct test mode needs a catalog model"),
    }
    SetupMode::Provider { flow }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn choose(app: &mut SetupApp, id: &str) {
    app.select_picker(id.to_owned());
}

fn render_setup(app: &SetupApp, width: u16, height: u16) -> String {
    setup_screen(&render_setup_buffer(
        app,
        width,
        height,
        Theme::new().without_color().without_motion(),
    ))
}

fn render_setup_buffer(
    app: &SetupApp,
    width: u16,
    height: u16,
    theme: Theme,
) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| draw_setup(frame, app, theme))
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn setup_screen(buffer: &ratatui::buffer::Buffer) -> String {
    // Read glyphs, not cells: the trailing cell of a wide character is
    // stored blank, so cell-by-cell collection garbles Chinese content.
    (0..buffer.area.height)
        .map(|y| {
            crate::selection::glyph_bounds(buffer, buffer.area, y)
                .map(|(x, _)| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn setup_body_rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    let mut rows = setup_screen(buffer)
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    while rows.last().is_some_and(|row| row.trim().is_empty()) {
        rows.pop();
    }
    rows
}

fn glm_environment_review() -> SetupApp {
    let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new())
        .with_destination("/tmp/smith-home/.smith/config.toml");
    choose(&mut app, "glm");
    choose(&mut app, "environment");
    for character in "ZAI_API_KEY".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    app
}
