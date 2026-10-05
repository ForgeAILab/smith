use super::*;

#[test]
fn quick_start_menu_and_review_use_supplied_values() {
    let mut data = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new()).quick_start;
    data.model = "glm-reviewed".into();
    data.model_label = "GLM Reviewed".into();
    data.catalog_revision = 99;
    data.limits.context_tokens = 123_456;
    data.request_output_tokens = 4_000;
    data.output_reserve = 5_000;
    let entries = setup_entries(&data);
    let mut app = SetupApp::new(
        SetupMode::FirstRun,
        Vec::new(),
        Vec::new(),
        data,
        entries,
        setup_prompts(),
    );
    let menu = render_setup(&app, 110, 34);
    assert!(menu.contains("Z.AI · GLM Reviewed"), "{menu}");
    choose(&mut app, "glm");
    choose(&mut app, "existing-keychain");
    let review = app.review_lines().join("\n");
    for value in [
        "glm-reviewed",
        "trusted catalog v99",
        "123.4k context",
        "4k output",
        "5k reserved",
    ] {
        assert!(review.contains(value), "missing {value}: {review}");
    }
}

#[test]
fn anthropic_reuses_credentials_limits_and_review_with_its_native_kind() {
    for resolved in [
        None,
        Some(ResolvedModelLimits {
            context_tokens: 200_000,
            max_input_tokens: 190_000,
            max_output_tokens: 10_000,
            source: "trusted catalog match anthropic/claude-reviewed".into(),
        }),
    ] {
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        choose(&mut app, "anthropic-messages");
        assert_eq!(app.step, Step::CredentialMethod);
        assert_eq!(app.provider, "anthropic");
        choose(&mut app, "config");
        app.on_paste("sk-anthropic-test-only");
        assert!(!render_setup(&app, 110, 34).contains("sk-anthropic-test-only"));
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.step, Step::ModelName);
        app.on_paste("claude-reviewed");
        let SetupEffect::ResolveModelLimits { request } = app.on_key(key(KeyCode::Enter)) else {
            panic!("Anthropic uses the shared model-limit resolver");
        };
        assert!(!request.use_endpoint_listing);
        app.apply_resolved_limits(resolved);
        if app.step == Step::ContextTokens {
            app.on_paste("64000");
            app.on_key(key(KeyCode::Enter));
        }
        assert_eq!(app.step, Step::DefaultChoice);
        choose(&mut app, "yes");
        let review = app.review_lines().join("\n");
        assert!(
            review
                .lines()
                .any(|line| line.starts_with("Connection")
                    && line.contains("Anthropic Messages API")),
            "{review}"
        );
        assert!(review.contains("https://api.anthropic.com/v1"), "{review}");
        assert!(review.contains("anthropic/claude-reviewed"), "{review}");
        assert!(!review.contains("sk-anthropic-test-only"), "{review}");
        app.on_key(key(KeyCode::BackTab));
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.step, Step::ContextTokens);
        assert!(!app.input.is_empty(), "Back retains non-secret context");
        app.on_key(key(KeyCode::Enter));
        choose(&mut app, "yes");
        assert!(matches!(
            app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            SetupEffect::Cancel
        ));
        assert!(matches!(
            app.on_key(key(KeyCode::Enter)),
            SetupEffect::Submit {
                submission: SetupSubmission::AddProvider {
                    kind: SetupProviderKind::AnthropicMessages,
                    credential: SetupCredential::StoreInConfig(_),
                    reasoning_only_text: false,
                    make_default: true,
                    ..
                },
                allow_collisions: false,
            }
        ));
    }
}

#[test]
fn chatgpt_confirmation_requests_the_existing_connection_handoff() {
    let mut app =
        setup_app(SetupMode::FirstRun, Vec::new(), Vec::new()).with_provider_actions(vec![
            SetupEntry {
                id: "chatgpt".into(),
                label: "Connect ChatGPT".into(),
                detail: "OAuth".into(),
                flow: SetupFlow::OAuth {
                    busy_note: "Opening ChatGPT sign-in…".into(),
                },
            },
        ]);
    assert!(matches!(
        app.on_key(key(KeyCode::Enter)),
        SetupEffect::ConnectChatGpt
    ));
    assert!(app.is_busy());
    assert!(!app.is_choosing_action());
    assert!(app.busy_note().is_some_and(|note| note.contains("ChatGPT")));
    let mut methods = ResourcePicker::choices(
        "Connect ChatGPT",
        vec![ResourceEntry::new("browser", "Browser login", "")],
        "empty",
    )
    .with_back(true);
    assert_eq!(
        methods.on_event(ScreenEvent::Key(key(KeyCode::Esc))),
        ScreenStep::Outcome(crate::screen::FlowOutcome::Back)
    );
    app.back_from_chatgpt();
    assert!(app.is_choosing_action());
    assert_eq!(
        app.picker
            .as_ref()
            .and_then(ResourcePicker::selected_entry)
            .map(|entry| entry.id.as_str()),
        Some("chatgpt")
    );
}

#[test]
fn an_arbitrary_entry_id_dispatches_its_required_flow() {
    let mut app =
        setup_app(SetupMode::FirstRun, Vec::new(), Vec::new()).with_provider_actions(vec![
            SetupEntry {
                id: "future-provider".into(),
                label: "Future provider".into(),
                detail: String::new(),
                flow: SetupFlow::AddModel,
            },
        ]);
    assert!(matches!(app.on_key(key(KeyCode::Enter)), SetupEffect::None));
    assert!(!app.is_choosing_action());
    assert_eq!(app.step, Step::ProviderChoice);
    assert!(matches!(
        app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        SetupEffect::Cancel
    ));
}

#[test]
fn openrouter_mode_fixes_identity_and_endpoint_before_authentication() {
    let model = "openai/gpt-reviewed";
    let limits = SetupModelLimits {
        context_tokens: 128_000,
        max_input_tokens: 120_000,
        max_output_tokens: 8_000,
    };
    let mut app = setup_app(
        direct_provider_mode("openrouter"),
        Vec::new(),
        vec![ResourceEntry::new(
            model,
            "Reviewed model",
            "catalog limits",
        )],
    )
    .with_catalog_model_limits(BTreeMap::from([(model.to_owned(), limits)]));
    assert_eq!(app.step, Step::CredentialMethod);
    assert_eq!(app.provider, "openrouter");
    assert_eq!(app.endpoint, "https://openrouter.ai/api/v1");

    choose(&mut app, "environment");
    for character in "OPENROUTER_API_KEY".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.step, Step::ModelChoice);
    assert!(app.secret.is_empty());

    choose(&mut app, model);
    assert_eq!(app.step, Step::DefaultChoice);
    assert_eq!(app.model, model);
    assert_eq!(app.context_tokens, Some(limits.context_tokens));
    assert_eq!(app.max_input_tokens, Some(limits.max_input_tokens));
    assert_eq!(app.max_output_tokens, Some(limits.max_output_tokens));
}

#[test]
fn google_mode_chooses_a_catalog_model_without_collecting_an_endpoint() {
    let model = "gemini-3.6-flash";
    let limits = SetupModelLimits {
        context_tokens: 1_048_576,
        max_input_tokens: 1_048_576,
        max_output_tokens: 65_536,
    };
    let mut app = setup_app(
        direct_provider_mode("google"),
        Vec::new(),
        vec![ResourceEntry::new(
            model,
            "Gemini 3.6 Flash",
            "catalog limits",
        )],
    )
    .with_catalog_model_limits(BTreeMap::from([(model.to_owned(), limits)]));
    assert_eq!(app.step, Step::CredentialMethod);
    assert_eq!(app.provider, "google");
    assert!(app.endpoint.ends_with("/v1beta"));

    choose(&mut app, "environment");
    for character in "GEMINI_API_KEY".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.step, Step::ModelChoice);
    choose(&mut app, model);
    choose(&mut app, "yes");
    assert_eq!(app.step, Step::Review);
    let review = app.review_lines().join("\n");
    assert!(review.contains("Gemini API"));
    assert!(review.contains(&app.endpoint), "{review}");
    assert!(review.contains("Models.dev frozen catalog"));
    assert!(!review.contains("API base URL"));
    assert!(matches!(
        app.on_key(key(KeyCode::Enter)),
        SetupEffect::Submit {
            submission: SetupSubmission::QuickGoogle { model: selected, credential: SetupCredential::Environment(variable) },
            allow_collisions: false,
        } if selected == model && variable == "GEMINI_API_KEY"
    ));
}

#[test]
fn masked_key_never_appears_in_debug_or_review() {
    let secret = "sk-do-not-render";
    let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    choose(&mut app, "glm");
    choose(&mut app, "keychain");
    for character in secret.chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    let rendered = format!("{app:?}\n{}", app.review_lines().join("\n"));
    assert!(!rendered.contains(secret), "{rendered}");
    assert!(
        app.secret
            .0
            .display_text()
            .chars()
            .all(|character| character == '•')
    );
}

#[test]
fn config_storage_is_masked_warned_and_submitted_as_a_secret() {
    let secret = "sk-config-input-must-not-render";
    let mut app = setup_app(
        SetupMode::Credential {
            provider: "zai".into(),
        },
        Vec::new(),
        Vec::new(),
    )
    .with_destination("/tmp/smith-home/.smith/config.toml");
    choose(&mut app, "config");
    for character in secret.chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    let input_render = render_setup(&app, 92, 20);
    assert!(!input_render.contains(secret), "{input_render}");
    assert!(input_render.contains('•'), "{input_render}");

    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.step, Step::Review);
    let review = app.review_lines().join("\n");
    assert!(review.contains("api_key = [redacted]"), "{review}");
    assert!(review.contains("plaintext at rest"), "{review}");
    assert!(review.contains("same-user processes"), "{review}");
    assert!(review.contains("Backups"), "{review}");
    assert!(!review.contains(secret), "{review}");
    assert!(!format!("{app:?}").contains(secret));

    assert!(matches!(
        app.on_key(key(KeyCode::Enter)),
        SetupEffect::Submit {
            submission: SetupSubmission::ChangeCredential {
                provider,
                credential: SetupCredential::StoreInConfig(value),
            },
            allow_collisions: false,
        } if provider == "zai" && value.expose() == secret
    ));
}

#[test]
fn an_unknown_custom_model_asks_only_for_context_and_derives_ceilings() {
    let mut app = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());
    for character in "router".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    for character in "https://example.test/v1".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    choose(&mut app, "existing-keychain");
    for character in "model".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    // The driver feeds a failed resolution back before any numeric value
    // is requested.
    assert!(matches!(
        app.on_key(key(KeyCode::Enter)),
        SetupEffect::ResolveModelLimits { .. }
    ));
    assert_eq!(app.step, Step::Busy);
    app.apply_resolved_limits(None);
    assert_eq!(app.step, Step::ContextTokens);
    let rendered = render_setup(&app, 92, 20);
    assert!(rendered.contains("Model context window"), "{rendered}");
    assert!(
        rendered.contains("input/output ceilings are derived"),
        "{rendered}"
    );
    assert!(!rendered.contains("Maximum input tokens"), "{rendered}");
    assert!(!rendered.contains("Maximum output tokens"), "{rendered}");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.step, Step::ContextTokens);
    assert!(app.error.is_some(), "an empty context window was accepted");
    for character in "64000".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.step, Step::ResponseBehavior);
    assert_eq!(app.context_tokens, Some(64_000));
    assert_eq!(app.max_input_tokens, Some(64_000));
    assert_eq!(app.max_output_tokens, Some(16_000));
    assert!(
        app.review_lines()
            .iter()
            .any(|line| line.contains("context entered manually · ceilings derived"))
    );
}

#[test]
fn a_model_step_resolution_request_carries_the_flow_facts() {
    let mut app = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());
    for character in "router".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    for character in "https://example.test/v1".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    choose(&mut app, "keychain");
    for character in "sk-test".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    for character in "model".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    let SetupEffect::ResolveModelLimits { request } = app.on_key(key(KeyCode::Enter)) else {
        panic!("the custom model step requests resolution");
    };
    assert_eq!(request.endpoint.as_deref(), Some("https://example.test/v1"));
    assert_eq!(request.provider.as_deref(), Some("router"));
    assert_eq!(request.model, "model");
    assert!(request.bearer.is_some(), "the typed key rides along");
    assert_eq!(request.environment_variable, None);
    // The busy note is the reason the surface is blocked, and no key
    // escapes it while the driver works.
    assert!(
        app.busy_note()
            .is_some_and(|note| note.contains("Resolving"))
    );
    assert!(matches!(app.on_key(key(KeyCode::Esc)), SetupEffect::None));
}

#[test]
fn resolved_limits_skip_input_and_only_context_is_editable() {
    let mut app = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());
    app.action = Some(SetupAction::AddProvider);
    app.endpoint = "https://example.test/v1".into();
    app.enter(Step::ModelName, false);
    for character in "model".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    app.apply_resolved_limits(Some(ResolvedModelLimits {
        context_tokens: 200_000,
        max_input_tokens: 200_000,
        max_output_tokens: 32_768,
        source: "endpoint /models listing".to_owned(),
    }));
    assert_eq!(app.step, Step::ResponseBehavior);
    assert!(
        app.review_lines()
            .iter()
            .any(|line| line.contains("(endpoint /models listing)")),
        "{:?}",
        app.review_lines()
    );
    // Back walks into the one context-window fallback with its value
    // prefilled, not into the busy step the resolution passed through.
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.step, Step::ContextTokens);
    assert_eq!(app.input, "200000");
    let rendered = render_setup(&app, 92, 20);
    assert!(
        rendered.contains("Resolved from endpoint /models listing"),
        "{rendered}"
    );
    app.input.clear();
    for character in "100000".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.step, Step::ResponseBehavior);
    assert_eq!(app.max_input_tokens, Some(100_000));
    assert_eq!(app.max_output_tokens, Some(25_000));
    assert!(
        app.review_lines()
            .iter()
            .any(|line| line.contains("(context entered manually · ceilings derived)")),
        "{:?}",
        app.review_lines()
    );
}

#[test]
fn unchanged_resolved_context_preserves_published_ceilings() {
    let mut app = setup_app(
        SetupMode::AddModel { provider: None },
        Vec::new(),
        Vec::new(),
    );
    app.action = Some(SetupAction::AddModel);
    app.provider = "local".into();
    app.enter(Step::ModelName, false);
    for character in "m".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    app.apply_resolved_limits(Some(ResolvedModelLimits {
        context_tokens: 64_000,
        max_input_tokens: 60_000,
        max_output_tokens: 4_000,
        source: "endpoint /models listing".to_owned(),
    }));
    assert_eq!(app.step, Step::DefaultChoice);
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.step, Step::ContextTokens);
    assert_eq!(app.input, "64000");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.step, Step::DefaultChoice);
    assert_eq!(app.max_input_tokens, Some(60_000));
    assert_eq!(app.max_output_tokens, Some(4_000));
    assert_eq!(
        app.limits_source.as_deref(),
        Some("endpoint /models listing")
    );
}

#[test]
fn escape_on_first_step_cancels_without_an_effectful_submission() {
    let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    assert!(matches!(app.on_key(key(KeyCode::Esc)), SetupEffect::Cancel));
}

#[test]
fn credential_service_failure_returns_to_authentication_with_environment_available() {
    let secret = "sk-must-be-forgotten";
    let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    choose(&mut app, "glm");
    choose(&mut app, "keychain");
    for character in secret.chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.fail(
        "protected storage unavailable; choose the environment-variable option",
        true,
    );
    assert_eq!(app.step, Step::CredentialMethod);
    let picker = app.picker.as_ref().expect("authentication picker");
    assert!(picker.entries.iter().any(|entry| entry.id == "environment"));
    assert!(picker.entries.iter().any(|entry| entry.id == "config"));

    choose(&mut app, "environment");
    assert!(app.secret.is_empty(), "stale key material was retained");
    let rendered = format!("{app:?}\n{}", app.review_lines().join("\n"));
    assert!(!rendered.contains(secret), "{rendered}");
}

#[test]
fn wide_no_color_review_names_every_non_secret_boundary() {
    let app = glm_environment_review();
    let rendered = render_setup(&app, 110, 34);
    for expected in [
        "Provider     zai",
        "api.z.ai/api/coding/paas/v4",
        "env:ZAI_API_KEY",
        "glm-5.2",
        "1M context",
        "trusted catalog v5",
        "/tmp/smith-home/.smith/config.toml",
        "Writes",
        "enter confirm",
        "esc back",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected:?}\n{rendered}"
        );
    }
}

#[test]
fn narrow_validation_keeps_field_error_and_navigation_visible() {
    let mut app = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());
    app.on_key(key(KeyCode::Enter));
    let rendered = render_setup(&app, 40, 10);
    assert!(rendered.contains("Provider name"), "{rendered}");
    assert!(rendered.contains("error:"), "{rendered}");
    assert!(rendered.contains("enter continue"), "{rendered}");
    assert!(rendered.contains("esc cancel"), "{rendered}");
}

#[test]
fn masked_input_and_collision_retry_remain_secret_free() {
    let secret = "sk-render-never";
    let mut masked = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    choose(&mut masked, "glm");
    choose(&mut masked, "keychain");
    for character in secret.chars() {
        masked.on_key(key(KeyCode::Char(character)));
    }
    let rendered = render_setup(&masked, 72, 18);
    assert!(!rendered.contains(secret), "{rendered}");
    assert!(rendered.contains('•'), "{rendered}");

    let mut review = glm_environment_review();
    review.review_collisions(
        "[providers.zai]\n- credential = \"env:OLD\"\n+ credential = \"env:ZAI_API_KEY\"",
    );
    assert!(matches!(
        review.on_key(key(KeyCode::Enter)),
        SetupEffect::Submit {
            allow_collisions: true,
            ..
        }
    ));
}

#[test]
fn backing_out_of_collision_review_revokes_the_stale_approval() {
    let mut review = glm_environment_review();
    review.review_collisions(
        "[providers.zai]\n- credential = \"env:OLD\"\n+ credential = \"env:ZAI_API_KEY\"",
    );

    // Back-editing invalidates the approval: a re-entered review submits
    // without collision consent until the merge preview is shown again.
    review.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(review.input, "ZAI_API_KEY");
    review.on_key(key(KeyCode::Enter));
    assert_eq!(review.step, Step::Review);
    assert!(matches!(
        review.on_key(key(KeyCode::Enter)),
        SetupEffect::Submit {
            allow_collisions: false,
            ..
        }
    ));
}

#[test]
fn picker_step_failures_render_their_error_above_the_picker() {
    let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    choose(&mut app, "glm");
    app.fail("keychain unavailable: locked", true);
    for (width, height) in [(44, 16), (80, 24)] {
        let rendered = render_setup(&app, width, height);
        assert!(
            rendered.contains("error: keychain unavailable: locked"),
            "{rendered}"
        );
        assert!(
            rendered.contains("❯ 1. Store API key securely"),
            "{rendered}"
        );
        for control in ["enter confirm", "esc back"] {
            assert!(rendered.contains(control), "{rendered}");
        }
    }
}

#[test]
fn glm_funnel_reaches_a_non_secret_review_and_submission() {
    let mut app = glm_environment_review();
    assert_eq!(app.step, Step::Review);
    let review = app.review_lines().join("\n");
    assert!(review.contains("glm-5.2"));
    assert!(review.contains("env:ZAI_API_KEY"));
    assert!(review.contains("an answer sent only as reasoning is shown as the reply"));
    assert!(matches!(
        app.on_key(key(KeyCode::Enter)),
        SetupEffect::Submit {
            submission: SetupSubmission::QuickGlm {
                credential: SetupCredential::Environment(variable)
            },
            allow_collisions: false,
        } if variable == "ZAI_API_KEY"
    ));
}
