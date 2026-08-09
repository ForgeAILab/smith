//! Deterministic resolution tests for the adaptive cache policy surface.

use smith_config::model::CacheMaintenanceMode;
use smith_config::resolve::{
    ConfigError, Layer, Overrides, ResolveRequest, SettingValue, SyntheticCacheSpendAuthority,
    resolve,
};
use tempfile::TempDir;

const BASE: &str = r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "keychain:smith/acme"
"#;

struct Fixture {
    home: TempDir,
    project: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            home: tempfile::tempdir().expect("home"),
            project: tempfile::tempdir().expect("project"),
        };
        std::fs::create_dir_all(fixture.home.path().join(".smith")).expect("user config dir");
        std::fs::create_dir_all(fixture.project.path().join(".smith")).expect("project config dir");
        fixture
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path()).with_home_dir(self.home.path())
    }

    fn resolve(&self, extra: &str) -> Result<smith_config::resolve::Resolution, ConfigError> {
        std::fs::write(
            self.project.path().join(".smith/config.toml"),
            format!("{BASE}\n{extra}"),
        )
        .expect("project config");
        resolve(&self.request())
    }
}

#[test]
fn built_in_cache_and_child_wait_defaults_are_typed_and_source_explainable() {
    let fixture = Fixture::new();
    let resolution = fixture.resolve("").expect("resolved defaults");
    let cache = &resolution.config.context.cache;

    assert_eq!(cache.requested_maintenance.value, CacheMaintenanceMode::Off);
    assert_eq!(cache.effective_maintenance.value, CacheMaintenanceMode::Off);
    assert_eq!(cache.inactivity_limit_ms.value, 3_600_000);
    assert_eq!(cache.max_hold_while_child_ms.value, 3_600_000);
    assert_eq!(cache.max_maintenance_calls.value, 1);
    assert_eq!(cache.max_maintenance_input_tokens.value, 0);
    assert_eq!(cache.max_maintenance_output_tokens.value, 256);
    assert_eq!(cache.maintenance_deadline_ms.value, 30_000);
    assert_eq!(cache.keepalive_margin_ms.value, 120_000);
    assert_eq!(cache.keepalive_jitter_percent.value, 10);
    assert!(cache.handoff_checkpoint.value);
    assert!(cache.idle_compaction.value);
    assert!(cache.resume_capsule.value);
    assert_eq!(cache.inactivity_limit_ms.source.layer, Layer::BuiltIn);
    assert_eq!(
        resolution.config.context.idle_compaction_ms,
        cache.inactivity_limit_ms
    );

    assert_eq!(
        resolution.config.child_agents.wait_default_timeout_ms.value,
        5_000
    );
    assert_eq!(
        resolution.config.child_agents.wait_max_timeout_ms.value,
        30_000
    );
    assert_eq!(
        resolution
            .config
            .child_agents
            .wait_default_timeout_ms
            .source
            .layer,
        Layer::BuiltIn
    );
    assert_eq!(
        resolution.config.synthetic_cache_spend,
        SyntheticCacheSpendAuthority::Deny
    );
}

#[test]
fn adaptive_request_is_narrowed_without_host_synthetic_authority() {
    let fixture = Fixture::new();
    let resolution = fixture
        .resolve(
            r#"
[profiles.work.context.cache]
maintenance = "adaptive"
"#,
        )
        .expect("resolved policy");
    let cache = &resolution.config.context.cache;

    assert_eq!(
        cache.requested_maintenance.value,
        CacheMaintenanceMode::Adaptive
    );
    assert_eq!(
        cache.effective_maintenance.value,
        CacheMaintenanceMode::Observe
    );
    assert_eq!(cache.requested_maintenance.source.layer, Layer::Profile);
    assert_eq!(cache.effective_maintenance.source.layer, Layer::Profile);
    assert!(
        cache
            .narrowing_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("synthetic_cache_spend"))
    );
    assert_eq!(
        resolution.config.synthetic_cache_spend,
        SyntheticCacheSpendAuthority::Deny
    );
}

#[test]
fn only_the_host_can_enable_effective_adaptive_maintenance() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.project.path().join(".smith/config.toml"),
        format!("{BASE}\n[profiles.work.context.cache]\nmaintenance = \"adaptive\"\n"),
    )
    .expect("project config");
    let resolution = resolve(
        &fixture
            .request()
            .with_synthetic_cache_spend(SyntheticCacheSpendAuthority::Allow),
    )
    .expect("resolved policy");
    let cache = &resolution.config.context.cache;
    assert_eq!(
        cache.requested_maintenance.value,
        CacheMaintenanceMode::Adaptive
    );
    assert_eq!(
        cache.effective_maintenance.value,
        CacheMaintenanceMode::Adaptive
    );
    assert!(cache.narrowing_reason.is_none());
    assert_eq!(
        resolution.config.synthetic_cache_spend,
        SyntheticCacheSpendAuthority::Allow
    );
}

#[test]
fn legacy_idle_compaction_alias_uses_one_clock_and_keeps_old_source_key() {
    let fixture = Fixture::new();
    let resolution = fixture
        .resolve(
            r#"
[profiles.work.context]
idle_compaction_ms = 900000
"#,
        )
        .expect("resolved alias");
    let limit = &resolution.config.context.cache.inactivity_limit_ms;
    assert_eq!(limit.value, 900_000);
    assert_eq!(limit.source.layer, Layer::Profile);
    assert_eq!(limit.source.key, "profiles.work.context.idle_compaction_ms");
    assert_eq!(
        resolution.config.context.idle_compaction_ms.source.key,
        limit.source.key
    );
    let explanation = resolution
        .provenance
        .explain("context.cache.inactivity_limit_ms")
        .expect("canonical explanation");
    assert_eq!(
        explanation.source.key,
        "profiles.work.context.idle_compaction_ms"
    );
    assert_eq!(explanation.value, SettingValue::Integer(900_000));
}

#[test]
fn same_file_legacy_and_replacement_values_fail_as_ambiguous() {
    let fixture = Fixture::new();
    let error = fixture
        .resolve(
            r#"
[context]
idle_compaction_ms = 900000
[context.cache]
inactivity_limit_ms = 800000
"#,
        )
        .expect_err("conflicting aliases");
    match error {
        ConfigError::Ambiguous { key, sources } => {
            assert_eq!(key, "context.cache.inactivity_limit_ms");
            assert_eq!(sources.len(), 2);
            assert!(
                sources
                    .iter()
                    .any(|source| source.key == "context.idle_compaction_ms")
            );
            assert!(
                sources
                    .iter()
                    .any(|source| source.key == "context.cache.inactivity_limit_ms")
            );
        }
        other => panic!("expected an alias ambiguity, got {other:?}"),
    }
}

#[test]
fn equal_same_file_alias_values_share_the_canonical_clock() {
    let fixture = Fixture::new();
    let resolution = fixture
        .resolve(
            r#"
[context]
idle_compaction_ms = 900000
[context.cache]
inactivity_limit_ms = 900000
"#,
        )
        .expect("equal aliases");

    assert_eq!(
        resolution.config.context.cache.inactivity_limit_ms.value,
        900_000
    );
    assert_eq!(
        resolution.config.context.idle_compaction_ms,
        resolution.config.context.cache.inactivity_limit_ms
    );
}

#[test]
fn alias_and_canonical_values_follow_normal_layer_precedence() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.home.path().join(".smith/config.toml"),
        "[context]\nidle_compaction_ms = 900000\n",
    )
    .expect("user config");
    let resolution = fixture
        .resolve(
            r#"
[context.cache]
inactivity_limit_ms = 800000
"#,
        )
        .expect("layered alias");

    let limit = &resolution.config.context.cache.inactivity_limit_ms;
    assert_eq!(limit.value, 800_000);
    assert_eq!(limit.source.layer, Layer::ProjectFile);
    assert_eq!(limit.source.key, "context.cache.inactivity_limit_ms");
}

#[test]
fn same_command_layer_alias_values_fail_as_ambiguous() {
    let fixture = Fixture::new();
    std::fs::write(fixture.project.path().join(".smith/config.toml"), BASE)
        .expect("project config");
    let error = resolve(&fixture.request().with_cli(Overrides {
        idle_compaction_ms: Some(900_000),
        inactivity_limit_ms: Some(800_000),
        ..Overrides::default()
    }))
    .expect_err("conflicting aliases");
    assert!(matches!(
        error,
        ConfigError::Ambiguous { key, .. } if key == "context.cache.inactivity_limit_ms"
    ));
}

#[test]
fn cache_policy_ranges_and_child_wait_order_fail_closed() {
    let fixture = Fixture::new();
    let error = fixture
        .resolve(
            r#"
[context.cache]
keepalive_jitter_percent = 51
"#,
        )
        .expect_err("invalid jitter");
    assert!(matches!(
        error,
        ConfigError::InvalidValue { ref source, .. }
            if source.key == "context.cache.keepalive_jitter_percent"
    ));

    let fixture = Fixture::new();
    let error = fixture
        .resolve(
            r#"
[profiles.work.child_agents]
wait_default_timeout_ms = 10001
wait_max_timeout_ms = 10000
"#,
        )
        .expect_err("default above maximum");
    assert!(matches!(
        error,
        ConfigError::InvalidValue { ref source, .. }
            if source.key == "profiles.work.child_agents.wait_default_timeout_ms"
    ));
}

#[test]
fn maintenance_input_budget_is_bounded_by_a_resolved_model_limit() {
    let fixture = Fixture::new();
    let error = fixture
        .resolve(
            r#"
[models."acme/example-model"]
max_input_tokens = 1024

[context.cache]
max_maintenance_input_tokens = 1025
"#,
        )
        .expect_err("input budget above model limit");
    assert!(matches!(
        error,
        ConfigError::InvalidValue { ref source, .. }
            if source.key == "context.cache.max_maintenance_input_tokens"
    ));
}

#[test]
fn nonzero_maintenance_input_budget_requires_a_resolved_model_limit() {
    let fixture = Fixture::new();
    let error = fixture
        .resolve(
            r#"
[context.cache]
max_maintenance_input_tokens = 1
"#,
        )
        .expect_err("unknown input budget cannot be bounded");
    assert!(matches!(
        error,
        ConfigError::InvalidValue { ref source, .. }
            if source.key == "context.cache.max_maintenance_input_tokens"
    ));
}

#[test]
fn miss_notice_policy_cannot_change_cache_mechanism_or_authority() {
    let fixture = Fixture::new();
    let hidden = fixture
        .resolve(
            r#"
[cache]
miss_notices = false
"#,
        )
        .expect("hidden notices");
    let visible = fixture
        .resolve(
            r#"
[cache]
miss_notices = true
"#,
        )
        .expect("visible notices");

    assert!(!hidden.cache_miss_notices.value);
    assert!(visible.cache_miss_notices.value);
    assert_eq!(hidden.config.context.cache, visible.config.context.cache);
    assert_eq!(
        hidden.config.synthetic_cache_spend,
        visible.config.synthetic_cache_spend
    );
    assert_eq!(hidden.config.child_agents, visible.config.child_agents);
}
