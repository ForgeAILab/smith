use agent_runtime_core::provider::ProviderAttemptPurpose;
use agent_runtime_core::usage::{CounterKind, Provenance, UsageDelta, UsageRecord, UsageSource};

use super::{restore_usage_records, restore_usage_with_bindings};

fn price(provider: &str, model: &str) -> smith_client::status::PriceReference {
    smith_client::status::PriceReference {
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
fn restored_usage_uses_the_single_manifest_binding_even_after_a_switch() {
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
    let retained = usage.cost_price(status.price()).expect("restored price");
    let cost = smith_client::status::SessionCost::compute(&usage, retained);
    assert_eq!(cost.micro_usd, 2_000_000);
    assert_eq!(cost.label, smith_client::status::CostLabel::Exact);
    assert_eq!(retained.render_sources(&usage), "zai/glm-5.3");
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
    observed.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 11_000));
    observed.switch_model(Some("google".into()), "gemini-3.8-flash");
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
            assert!(binding.price.is_some());
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
