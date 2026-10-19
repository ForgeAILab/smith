use super::{append_connectable_connections, connection_entries, model_entries};
use smith_config::inventory::{
    InventoryLimit, ModelInventoryEntry, ModelLimitOrigin, ProviderInventoryEntry,
};

fn advisor_test_agent() -> smith_config::resolve::ResolvedAgent {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join(".smith")).unwrap();
    std::fs::write(
        project.path().join(".smith/config.toml"),
        r#"
default_profile = "main"
[profiles.main]
provider = "local"
model = "main"
[providers.local]
kind = "fake"
"#,
    )
    .unwrap();
    smith_config::resolve::resolve(
        &smith_config::resolve::ResolveRequest::new(project.path()).with_home_dir(home.path()),
    )
    .unwrap()
    .config
    .agent
}

#[test]
fn advisor_model_rows_share_labels_path_checks_and_override_selection() {
    use smith_config::resolve::{AdvisorTarget, Source, Sourced};
    let mut agent = advisor_test_agent();
    agent.advisor_overridden = true;
    agent.profile.advisor = Some(Sourced::new(
        AdvisorTarget::parse("cli/claude-code/opus").unwrap(),
        Source::session("advisor"),
    ));
    let mut models =
        super::append_cli_agent_models_with_availability(Vec::new(), None, |program| {
            program == "claude"
        });
    models.push(smith_tui::ResourceEntry::new("acme/main", "Main", "main binding").active(true));
    let advisors = super::advisor_entries(&agent, &[], &models);
    assert!(!advisors.iter().any(|entry| entry.id == "acme/main"));
    for model in models.iter().filter(|entry| entry.id.starts_with("cli/")) {
        let advisor = advisors.iter().find(|entry| entry.id == model.id).unwrap();
        assert_eq!(advisor.label, model.label);
        assert_eq!(advisor.description, model.description);
        assert_eq!(advisor.detail, model.detail);
        assert_eq!(advisor.disabled_reason, model.disabled_reason);
        assert_eq!(advisor.active, advisor.id == "cli/claude-code/opus");
        if model.id.starts_with("cli/codex/") {
            assert_eq!(
                advisor.disabled_reason.as_deref(),
                Some("`codex` is not on PATH")
            );
        } else {
            assert!(advisor.disabled_reason.is_none());
        }
    }
    // The active CLI binding is dropped even when another model of that
    // same kind is the current advisor override.
    models
        .iter_mut()
        .find(|row| row.id == "cli/claude-code/sonnet")
        .unwrap()
        .active = true;
    let advisors = super::advisor_entries(&agent, &[], &models);
    assert!(
        !advisors
            .iter()
            .any(|row| row.id == "cli/claude-code/sonnet")
    );
    assert!(
        advisors
            .iter()
            .any(|row| row.id == "cli/claude-code/opus" && row.active)
    );
}

#[test]
fn setup_default_rows_list_agents_as_the_model_picker_does() {
    let provider = |name: &str, active: bool, adapter_available: bool| ProviderInventoryEntry {
        name: name.into(),
        kind: Some("fake".into()),
        adapter_available,
        selectable: adapter_available,
        model_count: 1,
        active,
        source: None,
    };
    let agent_pair = ModelInventoryEntry {
        provider: "google".into(),
        model: "cli/claude-code/sonnet".into(),
        label: "cli/claude-code/sonnet".into(),
        context_tokens: None,
        max_input_tokens: None,
        max_output_tokens: None,
        output_budget: None,
        context_windows: Vec::new(),
        tool_call: None,
        reasoning: None,
        structured_output: None,
        catalog_provider: None,
        catalog_revision: None,
        catalog_retrieved_at_ms: None,
        catalog_source: None,
        catalog_cost: None,
        profiles: vec!["cc".into()],
        selectable: true,
        disabled_reason: None,
        active: true,
    };
    let inventory = smith_config::inventory::SelectionInventory {
        models: vec![agent_pair],
        providers: vec![
            provider("google", true, true),
            provider("zeta", false, true),
        ],
        ..Default::default()
    };
    let claude_only = |program: &str| program == "claude";
    let picker = super::append_cli_agent_models_with_availability(
        Vec::new(),
        Some("cli/claude-code/sonnet"),
        claude_only,
    );
    let setup = super::default_model_entries(&inventory, claude_only);
    assert_eq!(setup.len(), picker.len(), "one row per agent model");
    for (setup, picker) in setup.iter().zip(&picker) {
        assert_eq!(setup.id, format!("google/{}", picker.id));
        assert_eq!(setup.label, picker.label);
        assert_eq!(setup.description, picker.description);
        assert_eq!(setup.detail, picker.detail);
        assert_eq!(setup.active, picker.active);
        assert_eq!(setup.disabled_reason, picker.disabled_reason);
    }
    assert!(
        setup
            .iter()
            .any(|row| row.id == "google/cli/claude-code/sonnet" && row.active)
    );
    assert!(setup.iter().any(|row| {
        row.id.starts_with("google/cli/codex/")
            && row.disabled_reason.as_deref() == Some("`codex` is not on PATH")
    }));

    // An unusable current provider hands the agent rows to the first
    // usable one; with none, setup cannot write an agent default at all.
    let mut fallback = inventory.clone();
    fallback.providers = vec![
        provider("google", true, false),
        provider("zeta", false, true),
    ];
    assert!(
        super::default_model_entries(&fallback, claude_only)
            .iter()
            .all(|row| row.id.starts_with("zeta/cli/"))
    );
    fallback.providers = vec![provider("google", true, false)];
    assert!(super::default_model_entries(&fallback, claude_only).is_empty());
}

#[test]
fn model_detail_compacts_every_limit_and_keeps_provenance_off_the_row() {
    let limit = |value| {
        Some(InventoryLimit {
            value,
            origin: ModelLimitOrigin::Trusted {
                catalog: "smith-trusted-models".into(),
                revision: 5,
            },
        })
    };
    let mut model = ModelInventoryEntry {
        provider: "zai".into(),
        model: "glm-5.3".into(),
        label: "GLM-5.3".into(),
        context_tokens: limit(1_000_000),
        max_input_tokens: limit(255_616),
        max_output_tokens: limit(131_072),
        output_budget: Some(
            smith_config::output_budget::resolve_output_budget(
                1_000_000,
                131_072,
                Some(32_768),
                Some(32_768),
                0,
            )
            .expect("budget"),
        ),
        context_windows: Vec::new(),
        tool_call: Some(true),
        reasoning: Some(true),
        structured_output: Some(false),
        catalog_provider: None,
        catalog_revision: None,
        catalog_retrieved_at_ms: None,
        catalog_source: None,
        catalog_cost: None,
        profiles: vec!["glm".into()],
        selectable: true,
        disabled_reason: None,
        active: true,
    };
    let entries = model_entries(vec![model.clone()], None);
    assert_eq!(
        entries[0].detail,
        "1M context · 255.6k input · 131k output ceiling · request 32.7k configured · zai/glm-5.3 · trusted catalog r5 · tools+reasoning · profiles glm"
    );
    for raw in [
        "1000000",
        "255616",
        "131072",
        "32768",
        "limits from",
        "smith-trusted-models",
    ] {
        assert!(!entries[0].detail.contains(raw), "{}", entries[0].detail);
    }
    assert!(!entries[0].description.contains("trusted catalog"));
    assert!(entries[0].active);
    let agent = advisor_test_agent();
    let mut unusable = model.clone();
    unusable.active = false;
    unusable.context_tokens = None;
    let rows = model_entries(vec![unusable.clone()], None);
    let advisors = super::advisor_entries(&agent, &[unusable], &rows);
    let disabled = advisors
        .iter()
        .find(|entry| entry.id == model.id())
        .unwrap();
    assert_eq!(
        disabled.disabled_reason.as_deref(),
        Some("advisor requires usable context limits")
    );
    model.max_input_tokens.as_mut().expect("input").origin = ModelLimitOrigin::BuiltIn;
    let mixed = model_entries(vec![model.clone()], None);
    assert!(mixed[0].detail.contains("255.6k [built-in] input"));
    assert!(mixed[0].detail.contains("trusted catalog r5"));

    model.provider = "xiaoge".into();
    model.model = "selected-large".into();
    model.catalog_provider = Some("xiaoge".into());
    model.catalog_source = Some("https://example.test/v1/models".into());
    model.catalog_revision = Some("endpoint-revision-1".into());
    model.catalog_retrieved_at_ms = Some(1);
    let endpoint = model_entries(vec![model.clone()], None);
    assert!(
        endpoint[0]
            .detail
            .contains("https://example.test/v1/models advertised")
    );
    assert!(!endpoint[0].detail.contains("models.dev/xiaoge"));
    assert!(endpoint[0].detail.contains("price unknown"));
    model.catalog_cost = Some(smith_config::catalog::CatalogModelCost {
        input: Some(0),
        output: Some(2_000_000),
        cache_read: None,
        cache_write: None,
    });
    let priced = model_entries(vec![model], None);
    assert!(
        priced[0]
            .detail
            .contains("USD/M tokens: input $0, output $2")
    );
    assert!(!priced[0].detail.contains("price unknown"));
}

#[test]
fn connection_rows_name_plain_authentication_and_a_distinct_custom_description() {
    for (kind, description) in [
        (
            smith_config::model::KIND_CHATGPT_RESPONSES,
            "ChatGPT sign-in · experimental",
        ),
        (
            smith_config::model::KIND_GEMINI_INTERACTIONS,
            "Gemini API key",
        ),
        (smith_config::model::KIND_XAI_RESPONSES, "xAI sign-in"),
        (smith_config::model::KIND_OPENAI_RESPONSES, "API key"),
        (smith_config::model::KIND_OPENAI_COMPATIBLE, "API key"),
    ] {
        let provider = ProviderInventoryEntry {
            name: "provider".into(),
            kind: Some(kind.into()),
            adapter_available: true,
            selectable: true,
            model_count: 1,
            active: false,
            source: None,
        };
        assert_eq!(connection_entries(&[provider])[0].description, description);
    }
    let mut entries = Vec::new();
    append_connectable_connections(&mut entries);
    let custom = entries
        .iter()
        .find(|entry| entry.id == "openai-compatible")
        .expect("custom endpoint");
    assert_eq!(
        custom.description,
        "any OpenAI-compatible endpoint · API key"
    );
    for entry in entries {
        assert_ne!(entry.label, entry.description);
    }
}
