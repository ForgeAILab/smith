use agent_runtime_core::provider::ProviderAttemptPurpose;
use agent_runtime_core::usage::{CounterKind, Provenance, UsageDelta, UsageRecord, UsageSource};

use super::{restore_usage_records, restore_usage_with_bindings};

fn price(provider: &str, model: &str) -> smith_client::status::PriceReference {
    smith_client::status::PriceReference {
        provenance: None,
        provider: provider.to_owned(),
        model: model.to_owned(),
        table: smith_client::status::PriceTable {
            input: Some(2_000_000),
            output: None,
            cache_read: None,
            cache_write: None,
        },
    }
}

fn root_record(tokens: u64) -> UsageRecord {
    UsageRecord {
        source: UsageSource::ProviderAttempt,
        provenance: Provenance::default(),
        delta: UsageDelta::new().with(CounterKind::InputUncached, tokens),
    }
}

#[test]
fn restored_usage_uses_single_manifest_identity_without_inventing_historical_rates() {
    let mut status = smith_client::status::Status::new("current", "project");
    status.switch_model(Some("google".into()), "current");
    status.set_price(Some(price("google", "current")));
    restore_usage_with_bindings(
        &mut status,
        &[root_record(1_000_000)],
        None,
        [("zai", "glm-5.3"), ("zai", "glm-5.3")],
        |provider, model| Some(price(provider, model)),
    );
    let usage = status.session_usage();
    assert_eq!(status.provider.as_deref(), Some("google"));
    assert_eq!(status.model, "current");
    assert_eq!(usage.total_tokens(), 1_000_000);
    assert_eq!(usage.bindings[0].provider.as_deref(), Some("zai"));
    assert_eq!(usage.bindings[0].model, "glm-5.3");
    assert!(usage.cost_price(status.price()).is_none());
    assert!(usage.bindings[0].price.is_none());
}

#[test]
fn restored_usage_with_several_bindings_stays_unpriced_and_named() {
    let mut status = smith_client::status::Status::new("current", "project");
    status.switch_model(Some("google".into()), "current");
    status.set_price(Some(price("google", "current")));
    restore_usage_with_bindings(
        &mut status,
        &[root_record(9_000_000)],
        None,
        [("zai", "glm-5.3"), ("google", "current")],
        |_, _| panic!("ambiguous records must not request a price"),
    );
    assert!(status.session_usage().cost_price(status.price()).is_none());
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1_000_000));
    let usage = status.session_usage();
    let current = usage
        .cost_price(status.price())
        .expect("new usage has a price");
    let cost = smith_client::status::SessionCost::compute(&usage, current);
    assert_eq!(cost.micro_usd, 2_000_000);
    assert_eq!(cost.label, smith_client::status::CostLabel::Estimated);
    assert_eq!(
        current.render_sources(&usage),
        "price unknown for earlier models · google/current $2.000"
    );
}

#[test]
fn restored_usage_without_manifests_has_no_invented_binding() {
    let mut status = smith_client::status::Status::new("current", "project");
    status.switch_model(Some("google".into()), "current");
    status.set_price(Some(price("google", "current")));
    restore_usage_with_bindings(&mut status, &[root_record(100)], None, [], |_, _| {
        panic!("missing manifests must not request a price")
    });
    let usage = status.session_usage();
    assert!(usage.cost_price(status.price()).is_none());
    assert_eq!(usage.bindings[0].model, "earlier models");
    assert!(usage.bindings[0].price.is_none());
}

fn logged_usage() -> smith_client::usage_log::SessionUsageRecord {
    let mut observed = smith_client::status::Status::new("glm-5.3", "project");
    observed.switch_model(Some("zai".into()), "glm-5.3");
    observed.set_price(Some(price("zai", "glm-5.3")));
    observed.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 11_000));
    observed.switch_model(Some("google".into()), "gemini-3.8-flash");
    let mut reference = price("google", "gemini-3.8-flash");
    reference.table.input = Some(3_000_000);
    observed.set_price(Some(reference));
    observed.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 12_000));
    smith_client::usage_log::SessionUsageRecord::new(
        "session",
        observed.provider.clone(),
        &observed.model,
        "build",
        &observed.session_usage(),
    )
}

#[test]
fn usage_restore_reads_the_latest_matching_attributed_record_from_the_project_log() {
    use smith_client::usage_log::{append, default_path};
    use smith_runtime::session::{ProjectId, SessionPaths};
    let root = tempfile::tempdir().expect("root");
    let paths = SessionPaths::new(root.path(), &ProjectId::new("project").expect("project"));
    let mut logged = logged_usage();
    append(&default_path(paths.directory()), &logged).expect("first record");
    logged.turns += 1;
    append(&default_path(paths.directory()), &logged).expect("last record");
    let mut mismatched = logged.clone();
    mismatched.totals.insert("input".into(), 24_000);
    append(&default_path(paths.directory()), &mismatched).expect("mismatched record");
    let mut other = logged.clone();
    other.session = "other-session".into();
    append(&default_path(paths.directory()), &other).expect("other record");
    let session = agent_runtime_core::ids::SessionId::new("session");
    let records = [root_record(23_000)];
    assert_eq!(
        super::last_session_usage(&paths, &session, &records),
        Some(logged)
    );
    assert!(
        super::last_session_usage(
            &paths,
            &agent_runtime_core::ids::SessionId::new("missing"),
            &records
        )
        .is_none()
    );
}

fn usage_with_unattributed_followup() -> (
    smith_client::usage_log::SessionUsageRecord,
    smith_client::usage_log::SessionUsageRecord,
    UsageRecord,
) {
    let mut observed = smith_client::status::Status::new("glm-5.3", "project");
    for (provider, model, input, output) in [
        ("zai", "glm-5.3", 1238, 14),
        ("google", "gemini-3.8-flash", 1163, 1),
    ] {
        observed.switch_model(Some(provider.into()), model);
        observed.record_usage(
            &UsageDelta::new()
                .with(CounterKind::InputUncached, input)
                .with(CounterKind::Output, output),
        );
    }
    let attributed = smith_client::usage_log::SessionUsageRecord::new(
        "session",
        observed.provider.clone(),
        &observed.model,
        "build",
        &observed.session_usage(),
    );
    let mut unattributed = attributed.clone();
    unattributed.bindings = vec![smith_client::usage_log::BindingUsageRecord {
        provider: None,
        model: "earlier models".into(),
        totals: attributed.totals.clone(),
        price: None,
        synthetic_by_purpose: Default::default(),
        reported: true,
    }];
    let mut restored = root_record(2401);
    restored.delta = restored.delta.with(CounterKind::Output, 15);
    (attributed, unattributed, restored)
}

#[test]
fn usage_restore_skips_a_later_unattributed_record_and_restores_both_models() {
    use smith_client::usage_log::{append, default_path};
    use smith_runtime::session::{ProjectId, SessionPaths};
    let root = tempfile::tempdir().expect("root");
    let paths = SessionPaths::new(root.path(), &ProjectId::new("project").expect("project"));
    let (attributed, unattributed, restored) = usage_with_unattributed_followup();
    append(&default_path(paths.directory()), &attributed).expect("attributed record");
    append(&default_path(paths.directory()), &unattributed).expect("unattributed record");
    let records = [restored];
    let logged = super::last_session_usage(
        &paths,
        &agent_runtime_core::ids::SessionId::new("session"),
        &records,
    );
    assert_eq!(logged.as_ref(), Some(&attributed));
    let mut status = smith_client::status::Status::new("current", "project");
    restore_usage_with_bindings(
        &mut status,
        &records,
        logged.as_ref(),
        [("zai", "glm-5.3"), ("google", "gemini-3.8-flash")],
        |provider, model| Some(price(provider, model)),
    );
    let usage = status.session_usage();
    assert_eq!(usage.bindings.len(), 2);
    for (binding, (provider, model, input, output)) in usage.bindings.iter().zip([
        ("zai", "glm-5.3", 1238, 14),
        ("google", "gemini-3.8-flash", 1163, 1),
    ]) {
        assert_eq!(binding.provider.as_deref(), Some(provider));
        assert_eq!(binding.model, model);
        assert_eq!(binding.totals[&CounterKind::InputUncached], input);
        assert_eq!(binding.totals[&CounterKind::Output], output);
    }
}

#[test]
fn usage_restore_with_only_unattributed_matches_falls_back_to_manifests() {
    use smith_client::usage_log::{append, default_path};
    use smith_runtime::session::{ProjectId, SessionPaths};
    let root = tempfile::tempdir().expect("root");
    let paths = SessionPaths::new(root.path(), &ProjectId::new("project").expect("project"));
    let (_, unattributed, restored) = usage_with_unattributed_followup();
    append(&default_path(paths.directory()), &unattributed).expect("first record");
    append(&default_path(paths.directory()), &unattributed).expect("last record");
    let records = [restored];
    let logged = super::last_session_usage(
        &paths,
        &agent_runtime_core::ids::SessionId::new("session"),
        &records,
    );
    assert!(logged.is_none());
    for manifests in [
        vec![("zai", "glm-5.3")],
        vec![("zai", "glm-5.3"), ("google", "gemini-3.8-flash")],
    ] {
        let mut status = smith_client::status::Status::new("current", "project");
        restore_usage_with_bindings(
            &mut status,
            &records,
            logged.as_ref(),
            manifests.clone(),
            |provider, model| Some(price(provider, model)),
        );
        let usage = status.session_usage();
        assert_eq!(usage.bindings.len(), 1);
        let binding = &usage.bindings[0];
        if manifests.len() == 1 {
            assert_eq!(binding.provider.as_deref(), Some("zai"));
            assert_eq!(binding.model, "glm-5.3");
            assert!(binding.price.is_none());
        } else {
            assert!(binding.provider.is_none());
            assert_eq!(binding.model, "earlier models");
            assert!(binding.price.is_none());
        }
        assert_eq!(binding.totals[&CounterKind::InputUncached], 2401);
        assert_eq!(binding.totals[&CounterKind::Output], 15);
    }
}

#[test]
fn matching_usage_log_restores_both_model_prices_and_keeps_the_active_binding() {
    let logged = logged_usage();
    let mut status = smith_client::status::Status::new("current", "project");
    status.switch_model(Some("google".into()), "current");
    status.restore_turn_count(2);
    restore_usage_with_bindings(
        &mut status,
        &[root_record(23_000)],
        Some(&logged),
        [("zai", "glm-5.3"), ("google", "gemini-3.8-flash")],
        |provider, model| {
            let mut reference = price(provider, model);
            if provider == "google" {
                reference.table.input = Some(3_000_000);
            }
            Some(reference)
        },
    );
    let usage = status.session_usage();
    assert_eq!(usage.turns, 2);
    assert_eq!(usage.bindings.len(), 2);
    assert_eq!(
        usage.bindings[0].totals[&CounterKind::InputUncached],
        11_000
    );
    assert_eq!(
        usage.bindings[1].totals[&CounterKind::InputUncached],
        12_000
    );
    assert_eq!(status.model, "current");
    let retained = usage.cost_price(status.price()).expect("restored price");
    assert_eq!(
        retained.render_sources(&usage),
        "zai/glm-5.3 $0.022 · google/gemini-3.8-flash $0.036"
    );
    assert_eq!(
        smith_client::status::SessionCost::compute(&usage, retained).micro_usd,
        58_000
    );
    assert_eq!(
        smith_client::status::SessionCost::compute(&usage, retained).label,
        smith_client::status::CostLabel::Exact
    );
}

#[test]
fn harness_usage_log_restores_provider_free_and_legacy_bindings() {
    use smith_client::status::{CostLabel, SessionCost, Status};

    let model = "cli/claude-code/haiku";
    let mut observed = Status::new(model, "project");
    observed.switch_model(Some("zai".into()), model);
    observed.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1_000));
    observed.switch_model(Some("zai".into()), "glm-5.3");
    observed.set_price(Some(price("zai", "glm-5.3")));
    observed.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 2_000));
    let original = smith_client::usage_log::SessionUsageRecord::new(
        "session",
        observed.provider.clone(),
        &observed.model,
        "code",
        &observed.session_usage(),
    );
    let mut external = root_record(1_000);
    external.source = UsageSource::ExternalAgent;
    let records = [external, root_record(2_000)];
    for provider in [None, Some("zai".to_owned())] {
        let mut logged = original.clone();
        logged.bindings[0].provider = provider;
        let mut status = Status::new(model, "project");
        status.switch_model(Some("google".into()), model);
        restore_usage_with_bindings(
            &mut status,
            &records,
            Some(&logged),
            [("zai", model), ("zai", "glm-5.3")],
            |provider, model| {
                assert_eq!(model, "glm-5.3");
                Some(price(provider, model))
            },
        );
        let usage = status.session_usage();
        assert_eq!(usage.bindings.len(), 2);
        assert_eq!(usage.bindings[0].provider, None);
        assert_eq!(usage.bindings[0].model, model);
        assert!(usage.bindings[0].price.is_none());
        assert_eq!(usage.bindings[1].provider.as_deref(), Some("zai"));
        assert_eq!(status.model, model);
        assert_eq!(status.provider.as_deref(), Some("google"));
        let retained = usage.cost_price(status.price()).expect("native price");
        let cost = SessionCost::compute(&usage, retained);
        assert_eq!(cost.micro_usd, 4_000);
        assert_eq!(cost.label, CostLabel::Estimated);
        assert_eq!(
            retained.render_sources(&usage),
            "price unknown for cli/claude-code/haiku · zai/glm-5.3 $0.004"
        );
    }
}

#[test]
fn mismatched_or_v4_usage_log_falls_back_to_manifests() {
    let original = logged_usage();
    let mut mismatched = original.clone();
    mismatched.totals.insert("input".into(), 1);
    let mut legacy = serde_json::to_value(&original).expect("record");
    legacy["schema_version"] = serde_json::json!(4);
    legacy.as_object_mut().expect("object").remove("bindings");
    let legacy = serde_json::from_value(legacy).expect("v4 record");
    let mut invalid_partition = original;
    invalid_partition.bindings[0]
        .totals
        .insert("input".into(), 1);
    for logged in [mismatched, legacy, invalid_partition] {
        for manifests in [
            vec![("zai", "glm-5.3")],
            vec![("zai", "glm-5.3"), ("google", "gemini-3.8-flash")],
        ] {
            let mut status = smith_client::status::Status::new("current", "project");
            restore_usage_with_bindings(
                &mut status,
                &[root_record(23_000)],
                Some(&logged),
                manifests.clone(),
                |provider, model| Some(price(provider, model)),
            );
            let usage = status.session_usage();
            assert_eq!(usage.bindings.len(), 1);
            assert_eq!(
                usage.bindings[0].model,
                if manifests.len() == 1 {
                    "glm-5.3"
                } else {
                    "earlier models"
                }
            );
        }
    }
}

#[test]
fn child_profile_resolution_keeps_catalog_endpoint_identity_and_unknowns() {
    use smith_config::inventory::{ModelInventoryEntry, ProfileInventoryEntry, SelectionInventory};
    use smith_config::model::{AgentPosture, ProfileUse};
    let mut inventory = SelectionInventory {
        profiles: vec![ProfileInventoryEntry {
            name: "child".into(),
            provider: Some("child-alias".into()),
            model: Some("child-model".into()),
            posture: AgentPosture::Build,
            description: None,
            uses: vec![ProfileUse::Child],
            revision: "revision".into(),
            legacy: false,
            selectable: true,
            active: false,
            source: None,
        }],
        models: vec![ModelInventoryEntry {
            provider: "child-alias".into(),
            model: "child-model".into(),
            label: "child model".into(),
            context_tokens: None,
            max_input_tokens: None,
            max_output_tokens: None,
            output_budget: None,
            context_windows: Vec::new(),
            tool_call: None,
            reasoning: None,
            structured_output: None,
            catalog_provider: Some("google".into()),
            catalog_source: None,
            catalog_cost: None,
            catalog_revision: None,
            catalog_retrieved_at_ms: None,
            profiles: vec!["child".into()],
            selectable: true,
            disabled_reason: None,
            active: false,
        }],
        ..SelectionInventory::default()
    };
    let catalog: smith_config::catalog::CatalogSnapshot = serde_json::from_value(serde_json::json!({
            "schema_revision": smith_config::catalog::CATALOG_SCHEMA_REVISION,
            "source_url": "fixture", "source_digest": "fixture", "content_digest": "fixture",
            "source_revision": "revision", "retrieved_at_ms": 0,
            "providers": { "google": {
                "id": "google", "name": "Google", "models": { "child-model": {
                    "id": "child-model", "name": "child model", "tool_call": true,
                    "reasoning": false, "structured_output": false,
                    "cost": { "input": 3_000_000, "output": null, "cache_read": null, "cache_write": null }
                }}
            }}
        })).expect("catalog fixture");
    let binding =
        super::resolve_child_usage_binding("child", &inventory, &catalog).expect("binding");
    assert_eq!(binding.provider.as_deref(), Some("child-alias"));
    assert_eq!(binding.model, "child-model");
    let price = binding.price.expect("child price");
    assert_eq!(price.provider, "child-alias");
    assert_eq!(price.table.input, Some(3_000_000));
    assert!(super::resolve_child_usage_binding("missing", &inventory, &catalog).is_none());
    // A custom source can intentionally use the same catalog/provider ID as
    // a public deployment; its frozen source and rates remain authoritative.
    inventory.models[0].catalog_source = Some("endpoint:account-a".into());
    inventory.models[0].catalog_revision = Some("endpoint-A".into());
    inventory.models[0].catalog_retrieved_at_ms = Some(2);
    inventory.models[0].catalog_cost = Some(smith_config::catalog::CatalogModelCost {
        input: Some(7_000_000),
        output: None,
        cache_read: None,
        cache_write: None,
    });
    let binding =
        super::resolve_child_usage_binding("child", &inventory, &catalog).expect("custom source");
    let price = binding.price.expect("frozen endpoint price");
    assert_eq!(price.table.input, Some(7_000_000));
    assert_eq!(
        price.provenance.as_ref().unwrap().source,
        "endpoint:account-a"
    );
    inventory.models[0].catalog_cost = None;
    assert!(
        super::resolve_child_usage_binding("child", &inventory, &catalog)
            .unwrap()
            .price
            .is_none(),
        "an unpriced custom source must not borrow the public provider's rates"
    );
    inventory.models[0].catalog_provider = None;
    let binding = super::resolve_child_usage_binding("child", &inventory, &catalog)
        .expect("known custom binding");
    assert!(
        binding.price.is_none(),
        "no rates borrowed from a matching model name"
    );
    inventory.profiles[0].model = None;
    assert!(super::resolve_child_usage_binding("child", &inventory, &catalog).is_none());
}

#[test]
fn restored_synthetic_usage_stays_out_of_ordinary_turn_totals() {
    let mut status = smith_client::status::Status::new("model", "project");
    let ordinary = UsageRecord {
        source: UsageSource::ProviderAttempt,
        provenance: Provenance {
            attempt_purpose: Some(ProviderAttemptPurpose::Ordinary),
            ..Provenance::default()
        },
        delta: UsageDelta::new()
            .with(CounterKind::InputUncached, 400)
            .with(CounterKind::Output, 20),
    };
    let idle_summary = UsageRecord {
        source: UsageSource::SemanticSummary,
        provenance: Provenance {
            attempt_purpose: Some(ProviderAttemptPurpose::IdleCompaction),
            ..Provenance::default()
        },
        delta: UsageDelta::new()
            .with(CounterKind::InputCached, 900)
            .with(CounterKind::Output, 40),
    };

    restore_usage_records(&mut status, &[ordinary, idle_summary]);

    let usage = status.session_usage();
    assert_eq!(usage.turns, 0);
    assert_eq!(usage.totals[&CounterKind::InputUncached], 400);
    assert_eq!(usage.totals[&CounterKind::Output], 20);
    assert_eq!(usage.synthetic_totals[&CounterKind::InputCached], 900);
    assert_eq!(usage.synthetic_totals[&CounterKind::Output], 40);
    assert_eq!(status.context.value, 400);
    assert_eq!(
        usage.synthetic_by_purpose[&ProviderAttemptPurpose::IdleCompaction]
            [&CounterKind::InputCached],
        900
    );
}

#[test]
fn confirmed_module_switch_waits_for_work_and_confirmation_to_finish() {
    use smith_client::commands::ModuleSwitchRequest;
    use smith_tui::status::Activity;
    let mut app = smith_tui::App::new("model", "project");
    let request = ModuleSwitchRequest {
        id: "image-generation".into(),
        enabled: false,
        fingerprint: "reviewed-edit".into(),
    };
    let mut pending = Some(request.clone());
    app.status.activity = Activity::Working;
    assert!(super::module_switch_exit(&app, &mut pending).is_none());
    assert_eq!(pending.as_ref(), Some(&request));
    app.status.activity = Activity::Idle;
    app.confirm_module_switch(request.clone(), "reviewed preview".into());
    assert!(super::module_switch_exit(&app, &mut pending).is_none());
    assert_eq!(pending.as_ref(), Some(&request));
    app.overlay = None;
    assert!(matches!(super::module_switch_exit(&app, &mut pending),
        Some(super::InteractiveExit::ModuleSwitch(applied)) if applied == request));
    assert!(pending.is_none());
}

#[test]
fn confirmed_plugin_switch_waits_for_work_and_confirmation_to_finish() {
    use smith_client::commands::PluginSwitchRequest;
    use smith_tui::status::Activity;
    let mut app = smith_tui::App::new("model", "project");
    let request = PluginSwitchRequest {
        name: "deploy-tools".into(),
        enabled: false,
        fingerprint: "reviewed-edit".into(),
    };
    let mut pending = Some(request.clone());
    app.status.activity = Activity::Working;
    assert!(super::plugin_switch_exit(&app, &mut pending).is_none());
    assert_eq!(pending.as_ref(), Some(&request));
    app.status.activity = Activity::Idle;
    app.confirm_plugin_switch(request.clone(), "reviewed preview".into());
    assert!(super::plugin_switch_exit(&app, &mut pending).is_none());
    assert_eq!(pending.as_ref(), Some(&request));
    app.overlay = None;
    assert!(matches!(super::plugin_switch_exit(&app, &mut pending),
        Some(super::InteractiveExit::PluginSwitch(applied)) if applied == request));
    assert!(pending.is_none());
}

#[tokio::test]
async fn chatgpt_subscription_price_stays_unknown_despite_openai_image_metadata() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join(".smith")).unwrap();
    std::fs::write(
        project.path().join(".smith/config.toml"),
        r#"
        default_profile = "luna"
        [profiles.luna]
        provider = "chatgpt"
        model = "gpt-5.6-luna"
        [providers.chatgpt]
        kind = "fake"
        [approval]
        mode = "deny"
    "#,
    )
    .unwrap();
    let config = smith_config::resolve::resolve(
        &smith_config::resolve::ResolveRequest::new(project.path()).with_home_dir(home.path()),
    )
    .unwrap()
    .config;
    let mut request = smith_runtime::factory::RuntimeRequest::new(
        config,
        smith_runtime::factory::HostSurface::Headless,
    );
    request.workspace = Some(std::sync::Arc::new(
        smith_host::ProjectWorkspace::new(project.path()).unwrap(),
    ));
    let runtime = smith_runtime::factory::build(
        smith_runtime::harness::resolve(smith_runtime::harness::HarnessSpec::trusted(request))
            .unwrap(),
    )
    .await
    .unwrap();
    let mut policy = runtime.policy().clone();
    policy.provider_kind = smith_config::model::KIND_CHATGPT_RESPONSES.into();
    policy.endpoint = Some(smith_config::setup::CHATGPT_ENDPOINT.into());
    let catalog =
        serde_json::from_str(smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED).unwrap();
    assert!(
        super::usage::resolve_catalog_price(
            "openai",
            policy.model.as_str(),
            Some("openai"),
            &catalog
        )
        .is_some()
    );
    assert!(super::usage::resolve_price(&policy, &catalog).is_none());
}

#[test]
fn resume_preserves_same_model_price_revisions_for_every_usage_route() {
    use smith_client::status::{CostLabel, SessionCost, Status};
    let a = price("gateway", "same-model").with_provenance("endpoint:account-a", "A", 1);
    let mut b = a.clone().with_provenance("endpoint:account-a", "B", 2);
    b.table.input = Some(5_000_000);
    let mut observed = Status::new("same-model", "project");
    observed.switch_model(Some("gateway".into()), "same-model");
    let mut records = Vec::new();
    for price in [&a, &b] {
        observed.set_price(Some(price.clone()));
        observed.set_advisor_price(Some(price.clone()));
        observed.set_vision_price(Some(price.clone()));
        for purpose in [
            None,
            Some("advisor"),
            Some("vision"),
            Some("module:review/module"),
        ] {
            let mut record = root_record(1_000);
            record.provenance.purpose = purpose.map(str::to_owned);
            observed.record_usage_record(&record);
            records.push(record);
        }
        let mut synthetic = root_record(1_000);
        synthetic.provenance.attempt_purpose = Some(ProviderAttemptPurpose::CacheKeepalive);
        observed.record_usage_record(&synthetic);
        records.push(synthetic);
    }
    let usage = observed.session_usage();
    let mut child = smith_client::status::BindingUsage::new(
        Some("gateway".into()),
        "same-model",
        Some(a.clone()),
    );
    child.record(
        &UsageDelta::new().with(CounterKind::InputUncached, 1_000),
        true,
    );
    let mut usage = usage;
    usage.bindings.push(child);
    usage
        .delegated_totals
        .insert(CounterKind::InputUncached, 1_000);
    let logged = smith_client::usage_log::SessionUsageRecord::new(
        "session",
        Some("gateway".into()),
        "same-model",
        "build",
        &usage,
    );
    let encoded = serde_json::to_string(&logged).expect("serialize prices");
    let logged = serde_json::from_str(&encoded).expect("restore immutable prices");
    let mut restored = Status::new("same-model", "project");
    restored.switch_model(Some("gateway".into()), "same-model");
    restored.set_price(Some(b.clone()));
    restored.set_advisor_price(Some(b.clone()));
    restored.set_vision_price(Some(b.clone()));
    restore_usage_with_bindings(
        &mut restored,
        &records,
        Some(&logged),
        [("gateway", "same-model")],
        |_, _| panic!("historical prices must not use today's catalog"),
    );
    let actual = restored.session_usage();
    assert_eq!(actual.bindings, usage.bindings);
    assert_eq!(actual.advisor_bindings, usage.advisor_bindings);
    assert_eq!(actual.vision_bindings, usage.vision_bindings);
    assert_eq!(actual.module_usage, usage.module_usage);
    let cost = SessionCost::compute(&actual, &b);
    assert_eq!(cost.micro_usd, 37_000);
    assert_eq!(cost.label, CostLabel::Exact);
    restored.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1_000));
    assert_eq!(
        SessionCost::compute(&restored.session_usage(), &b).micro_usd,
        42_000
    );
}

#[test]
fn a_legacy_log_preserves_model_identity_without_repricing_from_the_new_catalog() {
    let mut logged = logged_usage();
    logged.schema_version = 5;
    for binding in &mut logged.bindings {
        binding.price = None;
    }
    let mut status = smith_client::status::Status::new("current", "project");
    restore_usage_with_bindings(
        &mut status,
        &[root_record(23_000)],
        Some(&logged),
        [("zai", "glm-5.3"), ("google", "gemini-3.8-flash")],
        |_, _| panic!("v5 stored no historic price revision"),
    );
    let usage = status.session_usage();
    assert_eq!(usage.bindings.len(), 2);
    assert_eq!(usage.bindings[0].provider.as_deref(), Some("zai"));
    assert!(usage.bindings.iter().all(|binding| binding.price.is_none()));
}

#[test]
fn endpoint_price_lookup_uses_only_the_exact_endpoint_snapshot() {
    use smith_config::catalog::{CatalogSnapshot, EndpointCatalog};
    let mut catalog: CatalogSnapshot = serde_json::from_value(serde_json::json!({
        "schema_revision": smith_config::catalog::CATALOG_SCHEMA_REVISION,
        "source_url": "fixture", "source_digest": "public", "content_digest": "public",
        "source_revision": "public-A", "retrieved_at_ms": 1,
        "providers": { "openai": { "id": "openai", "name": "OpenAI", "models": {
            "same-model": { "id": "same-model", "name": "Model", "tool_call": true, "reasoning": false, "structured_output": false, "cost": { "input": 1_000_000 } }
        }}}
    })).expect("public snapshot");
    let endpoint = EndpointCatalog {
        binding_digest: "fixture-binding".into(),
        partial_limits: Default::default(), unadvertised_capabilities: Default::default(),
        kind: "openai".into(), base_url: "https://gateway.example/v1".into(),
        source: "endpoint:account-a".into(), revision: "endpoint-A".into(), retrieved_at_ms: 2,
        catalog: serde_json::from_value(serde_json::json!({ "id": "xiaoge", "name": "Xiaoge", "models": {
            "same-model": { "id": "same-model", "name": "Model", "tool_call": true, "reasoning": false, "structured_output": false, "cost": { "input": 7_000_000 } }
        }})).expect("endpoint provider"),
    };
    catalog.endpoint_catalogs.insert("xiaoge".into(), endpoint);
    let binding = catalog
        .bound_provider("xiaoge", "openai", Some("https://gateway.example/v1"))
        .expect("bound endpoint");
    let price = super::usage::bound_price("xiaoge", "same-model", binding).expect("endpoint rates");
    assert_eq!(price.table.input, Some(7_000_000));
    assert_eq!(
        price.provenance.as_ref().expect("provenance").revision,
        "endpoint-A"
    );
    assert!(
        catalog
            .bound_provider(
                "another-account",
                "openai",
                Some("https://gateway.example/v1")
            )
            .is_none()
    );
    assert!(
        catalog
            .bound_provider("xiaoge", "openai", Some("https://different.example/v1"))
            .is_none()
    );
    catalog
        .endpoint_catalogs
        .get_mut("xiaoge")
        .unwrap()
        .catalog
        .models
        .get_mut("same-model")
        .unwrap()
        .cost = None;
    let binding = catalog
        .bound_provider("xiaoge", "openai", Some("https://gateway.example/v1"))
        .unwrap();
    assert!(
        super::usage::bound_price("xiaoge", "same-model", binding).is_none(),
        "missing endpoint rates never borrow public same-name prices"
    );
}

#[test]
fn side_route_only_resume_retains_its_original_price() {
    use smith_client::status::{SessionCost, Status};
    let old = price("advisor", "reviewer").with_provenance("endpoint:advisor", "A", 1);
    let mut observed = Status::new("unpriced-root", "project");
    observed.set_advisor_price(Some(old.clone()));
    let record = UsageRecord {
        provenance: Provenance {
            purpose: Some("advisor".into()),
            ..Provenance::default()
        },
        ..root_record(1_000)
    };
    observed.record_usage_record(&record);
    let logged = smith_client::usage_log::SessionUsageRecord::new(
        "session",
        None,
        "unpriced-root",
        "build",
        &observed.session_usage(),
    );
    let mut restored = Status::new("unpriced-root", "project");
    restore_usage_with_bindings(&mut restored, &[record], Some(&logged), [], |_, _| None);
    let usage = restored.session_usage();
    let retained = usage.cost_price(None).expect("side-route revision A");
    assert_eq!(retained, &old);
    assert_eq!(SessionCost::compute(&usage, retained).micro_usd, 2_000);
}
