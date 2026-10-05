use super::provider::adapt_request;
use super::*;
use agent_runtime_core::catalog::{ModelLimits, ResolvedModelProfile};
use agent_runtime_core::content::Message;
use smith_config::resolve::{ResolveRequest, resolve};

fn request(effort: &str) -> ProviderRequest {
    let mut request = ProviderRequest::new(
        ModelId::new("example-model"),
        vec![Message::user("counted input")],
    );
    request.reasoning = Some(ReasoningConfig {
        effort: Some(effort.to_owned()),
        max_tokens: None,
    });
    request
}

fn resolved_config() -> ResolvedConfig {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
    std::fs::write(
        project.path().join(".smith/config.toml"),
        r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#,
    )
    .expect("config");
    resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved config")
        .config
}

fn model_profile(reasoning: ReasoningSupport) -> ResolvedModelProfile {
    let mut profile = ResolvedModelProfile::explicit(
        "local",
        ModelId::new("example-model"),
        ModelLimits::new(128_000, 124_000, 4_096),
    );
    profile.capabilities.reasoning = reasoning;
    profile
}

#[test]
fn openai_effort_stays_normalized_for_the_openai_adapter() {
    let mut request = request("high");
    adapt_request(&mut request, ReasoningDialect::OpenaiEffort).expect("adapted");
    assert_eq!(
        request.reasoning.and_then(|reasoning| reasoning.effort),
        Some("high".to_owned())
    );
    assert!(request.vendor_extensions.is_null());
}

#[test]
fn anthropic_effort_stays_neutral_for_the_native_adapter() {
    let mut request = request("low");
    request.vendor_extensions = json!({"existing": {"owner": "adapter"}});
    adapt_request(&mut request, ReasoningDialect::AnthropicEffort).expect("adapted");
    assert_eq!(
        request.reasoning.and_then(|reasoning| reasoning.effort),
        Some("low".to_owned())
    );
    assert_eq!(
        request.vendor_extensions,
        json!({"existing": {"owner": "adapter"}})
    );
}

#[test]
fn openrouter_effort_becomes_only_the_unified_reasoning_object() {
    let mut request = request("low");
    let messages = request.messages.clone();
    adapt_request(&mut request, ReasoningDialect::Openrouter).expect("adapted");
    assert!(request.reasoning.is_none());
    assert_eq!(
        request.vendor_extensions,
        json!({"reasoning": {"effort": "low"}})
    );
    assert_eq!(
        request.messages, messages,
        "context fields remain immutable"
    );
    assert_eq!(request.model.as_str(), "example-model");
}

#[test]
fn openrouter_explicit_off_is_not_misrouted_as_reasoning_effort() {
    let mut request = request(SENTINEL_DISABLED);
    adapt_request(&mut request, ReasoningDialect::Openrouter).expect("adapted");
    assert!(request.reasoning.is_none());
    assert_eq!(
        request.vendor_extensions,
        json!({"reasoning": {"enabled": false}})
    );
}

#[test]
fn explicit_off_takes_precedence_over_a_retained_effort() {
    let policy = ReasoningRuntimePolicy {
        selected_enabled: Some(false),
        selected_effort: Some("high".to_owned()),
        dialect: Some(ReasoningDialect::Openrouter),
        ..ReasoningRuntimePolicy::default()
    };

    assert_eq!(policy.effective_state(), "off");
    assert_eq!(
        policy
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some(SENTINEL_DISABLED.to_owned())
    );
}

#[test]
fn presence_only_or_unknown_models_reject_controls_instead_of_guessing_a_dialect() {
    for support in [ReasoningSupport::Fixed, ReasoningSupport::Unsupported] {
        let mut config = resolved_config();
        config.reasoning.effort = Some(Sourced::new(
            "high".to_owned(),
            Source::session("reasoning.effort"),
        ));
        let error = resolve_reasoning_policy(&config, &model_profile(support), None, None)
            .expect_err("presence-only metadata cannot expose controls");
        assert!(error.contains("not adjustable"), "{error}");
        assert!(error.contains("presence only"), "{error}");
    }
}

#[test]
fn openrouter_endpoint_grants_default_controls_to_catalog_reasoning_models() {
    let mut config = resolved_config();
    config.reasoning.effort = Some(Sourced::new(
        "medium".to_owned(),
        Source::session("reasoning.effort"),
    ));

    let policy = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENROUTER_ENDPOINT),
        None,
    )
    .expect("the unified OpenRouter reasoning API is controllable");
    assert_eq!(policy.support, ReasoningSupport::Controllable);
    assert_eq!(policy.switch, ReasoningSwitch::Optional);
    assert_eq!(policy.efforts, ["low", "medium", "high"]);
    assert_eq!(policy.dialect, Some(ReasoningDialect::Openrouter));
    assert!(policy.capability_source.contains("OpenRouter"));
    assert_eq!(
        policy
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some("medium".to_owned())
    );

    config.reasoning.effort = None;
    config.reasoning.enabled = Some(Sourced::new(false, Source::session("reasoning.enabled")));
    let off = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENROUTER_ENDPOINT),
        None,
    )
    .expect("the unified API exposes an on/off switch");
    assert_eq!(
        off.request_config().and_then(|reasoning| reasoning.effort),
        Some(SENTINEL_DISABLED.to_owned())
    );
}

#[test]
fn openai_endpoint_grants_the_effort_ladder_without_an_off_switch() {
    let mut config = resolved_config();
    config.reasoning.effort = Some(Sourced::new(
        "high".to_owned(),
        Source::session("reasoning.effort"),
    ));

    let policy = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENAI_ENDPOINT),
        None,
    )
    .expect("OpenAI reasoning models share the reasoning_effort control");
    assert_eq!(policy.support, ReasoningSupport::Controllable);
    assert_eq!(policy.switch, ReasoningSwitch::MandatoryOn);
    assert_eq!(policy.efforts, ["low", "medium", "high"]);
    assert_eq!(policy.dialect, Some(ReasoningDialect::OpenaiEffort));
    assert_eq!(
        policy
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some("high".to_owned())
    );

    // Off has no universal `reasoning_effort` spelling, so it stays a
    // local failure until per-model metadata advertises `none`.
    config.reasoning.effort = None;
    config.reasoning.enabled = Some(Sourced::new(false, Source::session("reasoning.enabled")));
    let error = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENAI_ENDPOINT),
        None,
    )
    .expect_err("off is not representable without an advertised `none`");
    assert!(error.contains("mandatory-on"), "{error}");
}

#[test]
fn xai_endpoint_grants_the_effort_ladder_without_an_off_switch() {
    let mut config = resolved_config();
    config.reasoning.effort = Some(Sourced::new(
        "high".to_owned(),
        Source::session("reasoning.effort"),
    ));
    let grok_ladder = CatalogReasoningControls {
        toggle: false,
        efforts: ["low", "medium", "high"].map(str::to_owned).to_vec(),
    };

    let policy = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(XAI_CATALOG_ENDPOINT),
        Some(&grok_ladder),
    )
    .expect("xAI reasoning models share the OpenAI-effort control");
    assert_eq!(policy.support, ReasoningSupport::Controllable);
    assert_eq!(policy.switch, ReasoningSwitch::MandatoryOn);
    assert_eq!(policy.efforts, grok_ladder.efforts);
    assert_eq!(policy.dialect, Some(ReasoningDialect::OpenaiEffort));
    assert!(policy.capability_source.contains("xAI"), "{policy:?}");
    assert_eq!(
        policy
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some("high".to_owned())
    );

    // Without catalog annotation the endpoint still exposes the universal
    // ladder so a Grok reasoning model is controllable immediately.
    let fallback = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(XAI_CATALOG_ENDPOINT),
        None,
    )
    .expect("xAI endpoint fallback ladder");
    assert_eq!(fallback.efforts, ["low", "medium", "high"]);
    assert!(
        fallback.capability_source.contains("xAI Responses"),
        "{fallback:?}"
    );

    config.reasoning.effort = None;
    config.reasoning.enabled = Some(Sourced::new(false, Source::session("reasoning.enabled")));
    let error = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(XAI_CATALOG_ENDPOINT),
        Some(&grok_ladder),
    )
    .expect_err("off is not representable without an advertised `none`");
    assert!(error.contains("mandatory-on"), "{error}");
}

#[test]
fn gemini_catalog_levels_are_sent_as_native_thinking_effort() {
    let mut config = resolved_config();
    config.reasoning.effort = Some(Sourced::new(
        "high".to_owned(),
        Source::session("reasoning.effort"),
    ));
    let controls = CatalogReasoningControls {
        toggle: false,
        efforts: ["minimal", "low", "medium", "high"]
            .map(str::to_owned)
            .to_vec(),
    };
    let policy = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(GEMINI_ENDPOINT),
        Some(&controls),
    )
    .expect("Gemini thinking levels are catalog-backed");
    assert_eq!(policy.support, ReasoningSupport::Controllable);
    assert_eq!(policy.switch, ReasoningSwitch::MandatoryOn);
    assert_eq!(policy.efforts, controls.efforts);
    assert_eq!(policy.dialect, Some(ReasoningDialect::GeminiThinking));
    assert_eq!(
        policy
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some("high".to_owned())
    );

    config.reasoning.effort = None;
    config.reasoning.enabled = Some(Sourced::new(false, Source::session("reasoning.enabled")));
    let error = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(GEMINI_ENDPOINT),
        Some(&controls),
    )
    .expect_err("native Gemini thinking is mandatory-on");
    assert!(error.contains("mandatory-on"), "{error}");
}

#[test]
fn anthropic_effort_metadata_resolves_mandatory_ladder_and_provenance() {
    let mut config = resolved_config();
    config.model = Sourced::new("claude-fable-5-1".to_owned(), Source::built_in("model"));
    let metadata_source = Source::built_in("models.dddai/claude-fable-5-1.reasoning");
    config.model_reasoning = ResolvedModelReasoning {
        toggle: None,
        mandatory: Some(Sourced::new(true, metadata_source.clone())),
        efforts: Some(Sourced::new(
            ["low", "medium", "high", "xhigh", "max"]
                .map(str::to_owned)
                .to_vec(),
            metadata_source.clone(),
        )),
        default_enabled: Some(Sourced::new(true, metadata_source.clone())),
        default_effort: Some(Sourced::new("high".to_owned(), metadata_source.clone())),
        dialect: Some(Sourced::new(
            ReasoningDialect::AnthropicEffort,
            metadata_source,
        )),
    };
    config.reasoning.effort = Some(Sourced::new(
        "low".to_owned(),
        Source::flag("reasoning.effort"),
    ));

    let policy =
        resolve_reasoning_policy(&config, &model_profile(ReasoningSupport::Fixed), None, None)
            .expect("explicit Anthropic controls are trusted");
    assert_eq!(policy.support, ReasoningSupport::Controllable);
    assert_eq!(policy.switch, ReasoningSwitch::MandatoryOn);
    assert_eq!(policy.efforts, ["low", "medium", "high", "xhigh", "max"]);
    assert_eq!(policy.default_enabled, Some(true));
    assert_eq!(policy.default_effort.as_deref(), Some("high"));
    assert_eq!(policy.dialect, Some(ReasoningDialect::AnthropicEffort));
    assert!(
        policy
            .capability_source
            .contains("models.dddai/claude-fable-5-1")
    );
    assert_eq!(
        policy
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some("low".to_owned())
    );

    config.reasoning.effort = None;
    config.reasoning.enabled = Some(Sourced::new(true, Source::session("reasoning.enabled")));
    let defaulted =
        resolve_reasoning_policy(&config, &model_profile(ReasoningSupport::Fixed), None, None)
            .expect("enabled Anthropic reasoning has a trusted default effort");
    assert_eq!(
        defaulted
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some("high".to_owned())
    );

    config.model_reasoning.default_effort = None;
    let error =
        resolve_reasoning_policy(&config, &model_profile(ReasoningSupport::Fixed), None, None)
            .expect_err("enabled Anthropic reasoning needs a selected/default effort");
    assert!(error.contains("Anthropic-effort"), "{error}");

    config.reasoning.effort = None;
    config.reasoning.enabled = Some(Sourced::new(false, Source::session("reasoning.enabled")));
    let error =
        resolve_reasoning_policy(&config, &model_profile(ReasoningSupport::Fixed), None, None)
            .expect_err("mandatory adaptive thinking cannot be disabled");
    assert!(error.contains("mandatory-on"), "{error}");

    config.reasoning.enabled = None;
    config.reasoning.effort = Some(Sourced::new(
        "ultra".to_owned(),
        Source::flag("reasoning.effort"),
    ));
    let error =
        resolve_reasoning_policy(&config, &model_profile(ReasoningSupport::Fixed), None, None)
            .expect_err("an unadvertised Anthropic effort is refused locally");
    assert!(error.contains("`ultra`"), "{error}");
    assert!(
        error.contains("supported values: low, medium, high, xhigh, max"),
        "{error}"
    );
}

#[test]
fn anthropic_named_models_do_not_infer_an_effort_dialect() {
    let mut config = resolved_config();
    config.model = Sourced::new("fable-5-1".to_owned(), Source::built_in("model"));
    config.reasoning.effort = Some(Sourced::new(
        "low".to_owned(),
        Source::flag("reasoning.effort"),
    ));
    let error =
        resolve_reasoning_policy(&config, &model_profile(ReasoningSupport::Fixed), None, None)
            .expect_err("model names do not establish a trusted dialect");
    assert!(error.contains("not adjustable"), "{error}");
    assert!(error.contains("presence only"), "{error}");
}

#[test]
fn catalog_advertised_ladders_refine_the_endpoint_defaults() {
    // A gpt-5.x-style ladder advertises `none`, which unlocks off.
    let mut config = resolved_config();
    config.reasoning.enabled = Some(Sourced::new(false, Source::session("reasoning.enabled")));
    let gpt_ladder = CatalogReasoningControls {
        toggle: false,
        efforts: ["none", "low", "medium", "high", "xhigh"]
            .map(str::to_owned)
            .to_vec(),
    };
    let policy = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENAI_ENDPOINT),
        Some(&gpt_ladder),
    )
    .expect("an advertised `none` unlocks off");
    assert_eq!(policy.switch, ReasoningSwitch::Optional);
    assert_eq!(policy.efforts, gpt_ladder.efforts);
    assert!(policy.capability_source.contains("Models.dev"));
    assert_eq!(
        policy
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some("none".to_owned())
    );

    // An o3-style ladder without `none` keeps off unrepresentable.
    let o3_ladder = CatalogReasoningControls {
        toggle: false,
        efforts: ["low", "medium", "high"].map(str::to_owned).to_vec(),
    };
    let error = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENAI_ENDPOINT),
        Some(&o3_ladder),
    )
    .expect_err("no advertised `none` means no off");
    assert!(error.contains("mandatory-on"), "{error}");

    // A toggle-only OpenRouter model keeps the unified switch and
    // advertises no ladder, so an effort selection fails locally.
    config.reasoning.enabled = None;
    config.reasoning.effort = Some(Sourced::new(
        "high".to_owned(),
        Source::session("reasoning.effort"),
    ));
    let toggle_only = CatalogReasoningControls {
        toggle: true,
        efforts: Vec::new(),
    };
    let error = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENROUTER_ENDPOINT),
        Some(&toggle_only),
    )
    .expect_err("a toggle-only model advertises no efforts");
    assert!(error.contains("no effort levels are advertised"), "{error}");
}

#[test]
fn openrouter_non_reasoning_models_stay_fail_closed() {
    let mut config = resolved_config();
    config.reasoning.enabled = Some(Sourced::new(true, Source::session("reasoning.enabled")));
    let error = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Unsupported),
        Some(OPENROUTER_ENDPOINT),
        None,
    )
    .expect_err("a non-reasoning model gains no switch from the endpoint");
    assert!(error.contains("not adjustable"), "{error}");
}

#[test]
fn zai_coding_plan_grants_the_thinking_switch_to_catalog_reasoning_models() {
    let mut config = resolved_config();
    config.reasoning.enabled = Some(Sourced::new(false, Source::session("reasoning.enabled")));

    // `example-model` is not in the trusted list, so this exercises the
    // catalog-advertised branch.
    let policy = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(ZAI_CODING_PLAN_ENDPOINT),
        None,
    )
    .expect("the coding-plan thinking switch is controllable");
    assert_eq!(policy.dialect, Some(ReasoningDialect::ZaiThinking));
    assert_eq!(policy.default_enabled, None);
    assert!(policy.efforts.is_empty());
    assert_eq!(
        policy
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some(SENTINEL_DISABLED.to_owned())
    );
}

#[test]
fn openai_effort_off_requires_an_advertised_none_effort() {
    let metadata_source = Source::built_in("test model metadata");
    let metadata = |efforts: Vec<String>| ResolvedModelReasoning {
        toggle: Some(Sourced::new(true, metadata_source.clone())),
        mandatory: None,
        efforts: Some(Sourced::new(efforts, metadata_source.clone())),
        default_enabled: None,
        default_effort: None,
        dialect: Some(Sourced::new(
            ReasoningDialect::OpenaiEffort,
            metadata_source.clone(),
        )),
    };

    let mut config = resolved_config();
    config.model_reasoning = metadata(vec!["low".to_owned(), "high".to_owned()]);
    config.reasoning.enabled = Some(Sourced::new(false, Source::session("reasoning.enabled")));
    let error = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Unsupported),
        None,
        None,
    )
    .expect_err("off would emit a non-advertised `none` effort");
    assert!(error.contains("non-advertised effort `none`"), "{error}");

    config.model_reasoning = metadata(vec!["none".to_owned(), "high".to_owned()]);
    let policy = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Unsupported),
        None,
        None,
    )
    .expect("an advertised `none` makes off representable");
    assert_eq!(
        policy
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some("none".to_owned())
    );
}

#[test]
fn mandatory_reasoning_rejects_off_with_a_local_alternative() {
    let mut config = resolved_config();
    let metadata_source = Source::built_in("test model metadata");
    config.model_reasoning = ResolvedModelReasoning {
        toggle: Some(Sourced::new(false, metadata_source.clone())),
        mandatory: Some(Sourced::new(true, metadata_source.clone())),
        efforts: Some(Sourced::new(
            vec!["high".to_owned()],
            metadata_source.clone(),
        )),
        default_enabled: Some(Sourced::new(true, metadata_source.clone())),
        default_effort: Some(Sourced::new("high".to_owned(), metadata_source.clone())),
        dialect: Some(Sourced::new(ReasoningDialect::Openrouter, metadata_source)),
    };
    config.reasoning.enabled = Some(Sourced::new(false, Source::session("reasoning.enabled")));

    let error = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Unsupported),
        Some(OPENROUTER_ENDPOINT),
        None,
    )
    .expect_err("mandatory reasoning cannot be disabled");
    assert!(error.contains("mandatory-on"), "{error}");
}

#[test]
fn zai_toggle_becomes_thinking_type_without_reasoning_effort() {
    let mut request = request(SENTINEL_ENABLED);
    adapt_request(&mut request, ReasoningDialect::ZaiThinking).expect("adapted");
    assert!(request.reasoning.is_none());
    assert_eq!(
        request.vendor_extensions,
        json!({"thinking": {"type": "enabled"}})
    );
}

#[test]
fn an_invocation_effort_is_validated_by_the_one_refusal_path() {
    // The flag reaches the resolver as an ordinary command-line-layer
    // value, so it is refused by exactly the rule an unsupported profile
    // effort meets — same wording, same place, before any credential
    // lookup or provider request.
    let mut config = resolved_config();
    config.reasoning.effort = Some(Sourced::new(
        "ludicrous".to_owned(),
        Source::flag("reasoning.effort"),
    ));

    let error = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENAI_ENDPOINT),
        None,
    )
    .expect_err("an unadvertised effort is refused");
    assert!(error.contains("`ludicrous`"), "{error}");
    assert!(
        error.contains("supported values: low, medium, high"),
        "{error}"
    );
    assert!(error.contains("example-model"), "{error}");

    // A supported one resolves, and says the command line chose it rather
    // than a session or a profile.
    config.reasoning.effort = Some(Sourced::new(
        "medium".to_owned(),
        Source::flag("reasoning.effort"),
    ));
    let policy = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENAI_ENDPOINT),
        None,
    )
    .expect("an advertised effort is selectable from the command line");
    assert_eq!(policy.selected_effort.as_deref(), Some("medium"));
    assert_eq!(policy.selection_source, "command-line flag `--effort`");
    assert_eq!(
        policy
            .request_config()
            .and_then(|reasoning| reasoning.effort),
        Some("medium".to_owned())
    );
}

#[test]
fn an_invocation_effort_on_an_uncontrollable_binding_refuses_rather_than_degrades() {
    // Nothing adjustable at all: presence in the catalog is not a dialect.
    let mut config = resolved_config();
    config.reasoning.effort = Some(Sourced::new(
        "high".to_owned(),
        Source::flag("reasoning.effort"),
    ));
    let error =
        resolve_reasoning_policy(&config, &model_profile(ReasoningSupport::Fixed), None, None)
            .expect_err("an unknown endpoint grants no reasoning controls");
    assert!(error.contains("not adjustable"), "{error}");
    assert!(error.contains("presence only"), "{error}");

    // Controllable, but with an on/off switch and no ladder to pick from.
    let toggle_only = CatalogReasoningControls {
        toggle: true,
        efforts: Vec::new(),
    };
    let error = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENROUTER_ENDPOINT),
        Some(&toggle_only),
    )
    .expect_err("a toggle-only binding advertises no efforts");
    assert!(error.contains("no effort levels are advertised"), "{error}");
}

#[test]
fn a_shadowed_persisted_effort_leaves_the_invocation_effort_in_place() {
    // What resume does when `--effort` was supplied: the saved override is
    // not reinstated over the command-line layer, and the flag's own
    // thinking state is untouched.
    let saved = PersistedReasoningOverride {
        enabled: Some(true),
        effort: Some("low".to_owned()),
        context_window: None,
    };

    let mut config = resolved_config();
    config.reasoning.effort = Some(Sourced::new(
        "high".to_owned(),
        Source::flag("reasoning.effort"),
    ));
    saved.apply(&mut config, false, true);

    let effort = config.reasoning.effort.expect("the invocation effort");
    assert_eq!(effort.value, "high");
    assert_eq!(effort.source.layer, Layer::CommandLine);
    assert_eq!(
        config.reasoning.enabled.map(|value| value.value),
        Some(true),
        "shadowing an effort must not touch the saved thinking state"
    );
}

#[test]
fn persisted_override_is_versioned_redaction_safe_and_additive() {
    let override_value = PersistedReasoningOverride {
        enabled: Some(false),
        effort: Some("high".to_owned()),
        context_window: Some("872k".to_owned()),
    };
    let state = override_value.versioned().expect("versioned state");
    assert_eq!(state.sensitivity, SessionStateSensitivity::RedactionSafe);
    assert_eq!(
        PersistedReasoningOverride::restore(&state).expect("restored"),
        override_value
    );
}

#[test]
fn restored_override_is_session_precedence_and_explicit_reset_skips_it() {
    let saved = PersistedReasoningOverride {
        enabled: Some(false),
        effort: Some("high".to_owned()),
        context_window: None,
    };

    let mut restored = resolved_config();
    saved.apply(&mut restored, false, false);
    assert_eq!(
        restored.reasoning.enabled.as_ref().map(|value| value.value),
        Some(false)
    );
    assert_eq!(
        restored
            .reasoning
            .effort
            .as_ref()
            .map(|value| value.value.as_str()),
        Some("high")
    );
    assert_eq!(
        restored
            .reasoning
            .effort
            .as_ref()
            .map(|value| value.source.layer),
        Some(Layer::SessionOverride)
    );

    let mut reset = resolved_config();
    saved.apply(&mut reset, true, true);
    assert!(reset.reasoning.enabled.is_none());
    assert!(reset.reasoning.effort.is_none());
}

#[test]
fn saved_context_window_restores_as_session_state_and_honors_reset_or_flag() {
    let saved = PersistedReasoningOverride {
        context_window: Some("872k".to_owned()),
        ..PersistedReasoningOverride::default()
    };

    let mut persisted_config = resolved_config();
    persisted_config.context_window = Some(Sourced::new(
        "872k".to_owned(),
        Source::session("context_window"),
    ));
    assert_eq!(
        PersistedReasoningOverride::from_config(&persisted_config).context_window,
        Some("872k".to_owned())
    );

    let mut restored = resolved_config();
    saved.apply_with_context_window(&mut restored, false, false, false, false);
    let selected = restored.context_window.expect("saved context window");
    assert_eq!(selected.value, "872k");
    assert_eq!(selected.source.layer, Layer::SessionOverride);

    let mut reset = resolved_config();
    saved.apply_with_context_window(&mut reset, false, false, true, false);
    assert!(reset.context_window.is_none());

    let mut flagged = resolved_config();
    flagged.context_window = Some(Sourced::new(
        "272k".to_owned(),
        Source::flag("context_window"),
    ));
    saved.apply_with_context_window(&mut flagged, false, false, false, true);
    let selected = flagged.context_window.expect("invocation flag remains");
    assert_eq!(selected.value, "272k");
    assert_eq!(selected.source.layer, Layer::CommandLine);

    let mut session_choice = resolved_config();
    session_choice.context_window = Some(Sourced::new(
        "272k".to_owned(),
        Source::session("context_window"),
    ));
    saved.apply_with_context_window(&mut session_choice, false, false, false, false);
    assert_eq!(
        session_choice
            .context_window
            .expect("current session choice remains")
            .value,
        "272k"
    );
}

#[test]
fn an_invocation_off_effort_is_refused_by_an_openai_ladder_without_none() {
    // `--effort off` is not a shorthand for disabling reasoning: the
    // OpenAI-effort ladder must advertise `none` before off is valid.
    let mut config = resolved_config();
    config.reasoning.effort = Some(Sourced::new(
        "off".to_owned(),
        Source::flag("reasoning.effort"),
    ));
    let controls = CatalogReasoningControls {
        toggle: false,
        efforts: ["low", "medium", "high"].map(str::to_owned).to_vec(),
    };

    let result = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENAI_ENDPOINT),
        Some(&controls),
    );
    let error = result.expect_err("an unadvertised off effort must be refused");
    assert!(error.contains("`off`"), "{error}");
    assert!(
        error.contains("supported values: low, medium, high"),
        "{error}"
    );
}

#[test]
fn an_invocation_off_effort_is_refused_by_openrouter_as_a_non_effort_value() {
    // OpenRouter sends off through its separate boolean control, so an
    // effort flag must not turn into a silently disabled reasoning policy.
    let mut config = resolved_config();
    config.reasoning.effort = Some(Sourced::new(
        "off".to_owned(),
        Source::flag("reasoning.effort"),
    ));
    let controls = CatalogReasoningControls {
        toggle: true,
        efforts: ["low", "medium", "high"].map(str::to_owned).to_vec(),
    };

    let result = resolve_reasoning_policy(
        &config,
        &model_profile(ReasoningSupport::Fixed),
        Some(OPENROUTER_ENDPOINT),
        Some(&controls),
    );
    let error = result.expect_err("an unadvertised off effort must be refused");
    assert!(error.contains("`off`"), "{error}");
    assert!(
        error.contains("supported values: low, medium, high"),
        "{error}"
    );
}
