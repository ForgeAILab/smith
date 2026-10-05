use super::*;

#[test]
fn agent_modes_are_typed_ordered_and_child_presets_stay_read_only() {
    let fixture = Fixture::new();
    fixture.write_project(BASE_PROJECT_CONFIG);
    fixture.write_user(
        r#"
default_agent = "plan"
agent_order = ["plan", "build", "review"]

[agent_modes.audit]
posture = "review"
description = "Evidence-only audit"

[child_agents.inspect]
posture = "plan"
description = "Inspect without mutation"
"#,
    );
    let resolution = resolve(&fixture.request()).expect("typed agent modes");
    assert_eq!(resolution.config.agent.active.value, "plan");
    assert_eq!(
        resolution.config.agent.order.value,
        ["plan", "build", "review"]
    );
    assert_eq!(resolution.config.agent.active_posture(), AgentPosture::Plan);
    assert_eq!(
        resolution.config.agent.modes["audit"].posture.value,
        AgentPosture::Review
    );
    assert!(
        resolution.config.agent.child_presets["inspect"]
            .posture
            .value
            .is_read_only()
    );

    fixture.write_user(
        r#"
[child_agents.unsafe]
posture = "build"
"#,
    );
    let error = resolve(&fixture.request()).expect_err("a write-capable child preset");
    assert!(
        error.to_string().contains("must use a read-only"),
        "{error}"
    );
}

#[test]
fn agent_profiles_inherit_runtime_settings_and_work_on_main_or_child() {
    let resolution = resolve_project(
        r#"
default_profile = "plan"
profile_order = ["work", "plan"]

[profiles.work]
provider = "acme"
model = "example-model"
posture = "build"
delegation = false
use = ["main", "child"]
description = "Implementation profile"
instructions = "Implement and verify the requested change."

[profiles.plan]
extends = "work"
posture = "plan"
instructions = "Inspect and produce an implementation-ready plan."

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "keychain:smith/acme"
"#,
    )
    .expect("inherited agent profiles");

    let agent = &resolution.config.agent;
    assert_eq!(agent.profile.name, "plan");
    assert_eq!(agent.active_posture(), AgentPosture::Plan);
    assert_eq!(agent.profile.provider.as_ref().unwrap().value, "acme");
    assert_eq!(agent.profile.model.as_ref().unwrap().value, "example-model");
    assert!(agent.profile.supports(ProfileUse::Main));
    assert!(agent.profile.supports(ProfileUse::Child));
    assert!(!agent.profile.delegation.value);
    assert_eq!(agent.profile.delegation.source.layer, Layer::Profile);
    assert_eq!(
        agent.profile.delegation.source.key,
        "profiles.work.delegation"
    );
    assert_eq!(agent.profile_order.value, ["work", "plan"]);
    assert_eq!(
        agent.profile.instructions.as_ref().unwrap().value,
        "Inspect and produce an implementation-ready plan."
    );
    assert_eq!(agent.profile.revision.len(), 64);
}

#[test]
fn agent_profile_delegation_defaults_to_enabled() {
    let resolution = resolve_project(BASE_PROJECT_CONFIG).expect("a default delegation policy");

    assert!(resolution.config.agent.profile.delegation.value);
}

#[test]
fn omitted_profile_order_derives_all_real_main_profiles_and_excludes_legacy_adapters() {
    let resolution = resolve_project(
        r#"
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"
posture = "build"

[profiles.plan]
extends = "work"
posture = "plan"

[profiles.inspect]
extends = "work"
posture = "review"
use = ["child"]

[agent_modes.audit]
posture = "review"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "keychain:smith/acme"
"#,
    )
    .expect("derived profile order");

    assert_eq!(
        resolution.config.agent.profile_order.value,
        ["plan", "work"]
    );
    assert!(
        !resolution
            .config
            .agent
            .profile_order
            .value
            .iter()
            .any(|name| name == "audit" || name == "inspect")
    );
}

#[test]
fn profile_inheritance_cycles_fail_before_runtime_construction() {
    let error = resolve_project(
        r#"
default_profile = "one"

[profiles.one]
extends = "two"
provider = "acme"
model = "example-model"

[profiles.two]
extends = "one"

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "keychain:smith/acme"
"#,
    )
    .expect_err("cyclic profiles");

    assert!(error.to_string().contains("one -> two -> one"), "{error}");
}

#[test]
fn new_profiles_and_user_legacy_declarations_cannot_claim_the_same_name() {
    let fixture = Fixture::new();
    fixture.write_project(BASE_PROJECT_CONFIG);
    fixture.write_user(
        r#"
[profiles.audit]
extends = "work"
posture = "review"

[agent_modes.audit]
posture = "review"
"#,
    );

    let error = resolve(&fixture.request()).expect_err("ambiguous profile name");
    assert!(matches!(error, ConfigError::Ambiguous { .. }));
    assert!(error.to_string().contains("profiles.audit"), "{error}");
}

#[test]
fn profile_placement_and_cycle_order_fail_closed() {
    let main_only = Fixture::new();
    main_only.write_project(BASE_PROJECT_CONFIG);
    let error = resolve(&main_only.request().with_profile_use(ProfileUse::Child))
        .expect_err("main-only profile selected for a child");
    assert!(
        error
            .to_string()
            .contains("not enabled for child-agent use"),
        "{error}"
    );

    let error = resolve_project(
        r#"
default_profile = "work"
profile_order = ["work", "inspect"]

[profiles.work]
provider = "acme"
model = "example-model"
use = ["main"]

[profiles.inspect]
extends = "work"
posture = "review"
use = ["child"]

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "keychain:smith/acme"
"#,
    )
    .expect_err("child-only profile in the main cycle");
    assert!(
        error
            .to_string()
            .contains("profile_order` names child-only profile `inspect`"),
        "{error}"
    );
}

#[test]
fn legacy_modes_and_child_presets_are_visible_as_deprecated_profile_placements() {
    let fixture = Fixture::new();
    fixture.write_project(&format!(
        "{BASE_PROJECT_CONFIG}\n\
         [agent_modes.audit]\nposture = \"review\"\ndescription = \"legacy audit\"\n\
         [child_agents.inspect]\nposture = \"plan\"\ndescription = \"legacy inspection\"\n"
    ));
    let resolution = resolve(&fixture.request()).expect("legacy compatibility adapters");

    let audit = resolution
        .config
        .agent
        .profiles
        .get("audit")
        .expect("legacy main profile");
    assert!(audit.legacy);
    assert_eq!(audit.uses.value, [ProfileUse::Main]);
    let inspect = resolution
        .config
        .agent
        .profiles
        .get("inspect")
        .expect("legacy child profile");
    assert!(inspect.legacy);
    assert_eq!(inspect.uses.value, [ProfileUse::Child]);

    let inventory =
        local_inventory(&resolution, &["openai-compatible"]).expect("legacy inventory entries");
    assert!(inventory.profiles.iter().any(|profile| {
        profile.name == "audit" && profile.legacy && profile.uses == [ProfileUse::Main]
    }));
    assert!(inventory.profiles.iter().any(|profile| {
        profile.name == "inspect" && profile.legacy && profile.uses == [ProfileUse::Child]
    }));
}

#[test]
fn glm_5_2_gets_a_source_explainable_32768_request_budget() {
    let resolution = resolve_project(
        r#"
default_profile = "glm"

[profiles.glm]
provider = "zai"
model = "glm-5.2"

[providers.zai]
kind = "openai-compatible"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential = "env:ZAI_API_KEY"
"#,
    )
    .expect("a cataloged GLM-5.2 selection");
    assert_eq!(
        resolution
            .config
            .max_output_tokens
            .as_ref()
            .expect("product request budget")
            .value,
        32_768
    );
    let explanation = resolution
        .provenance
        .explain("max_output_tokens")
        .expect("request-budget provenance");
    assert_eq!(explanation.source.layer, Layer::BuiltIn);
    assert!(explanation.source.key.contains("glm-5.2"));
    assert!(explanation.source.key.contains("request_output_tokens"));
}

#[test]
fn reasoning_defaults_and_exact_controls_are_typed_and_source_explainable() {
    let fixture = Fixture::new();
    fixture.write_project(&format!(
        r#"{BASE_PROJECT_CONFIG}

[profiles.work.reasoning]
enabled = true
effort = "high"

[models."acme/example-model".reasoning]
toggle = true
mandatory = false
efforts = ["none", "low", "high"]
default_enabled = true
default_effort = "low"
dialect = "openai-effort"
"#
    ));

    let resolution = resolve(&fixture.request().with_session(Overrides {
        reasoning_effort: Some("low".to_owned()),
        ..Overrides::default()
    }))
    .expect("a resolved reasoning policy");

    assert!(
        resolution
            .config
            .reasoning
            .enabled
            .expect("profile state")
            .value
    );
    let effort = resolution.config.reasoning.effort.expect("session effort");
    assert_eq!(effort.value, "low");
    assert_eq!(effort.source.layer, Layer::SessionOverride);
    assert_eq!(
        resolution
            .config
            .model_reasoning
            .efforts
            .expect("advertised efforts")
            .value,
        vec!["none", "low", "high"]
    );
    assert_eq!(
        resolution
            .config
            .model_reasoning
            .dialect
            .expect("exact dialect")
            .value
            .as_str(),
        "openai-effort"
    );

    let explanation = resolution
        .provenance
        .explain("reasoning.effort")
        .expect("reasoning provenance");
    assert_eq!(explanation.source.layer, Layer::SessionOverride);
    assert!(
        explanation
            .overridden
            .iter()
            .any(|entry| entry.source.layer == Layer::Profile)
    );
}

#[test]
fn an_invocation_effort_outranks_the_profile_and_the_environment() {
    let fixture = Fixture::new();
    let scenario = Scenario {
        profile: vec!["[profiles.work.reasoning]\neffort = \"low\"\n".to_owned()],
        env: BTreeMap::from([("SMITH_REASONING_EFFORT".to_owned(), "medium".to_owned())]),
        cli: Overrides {
            reasoning_effort: Some("high".to_owned()),
            ..Overrides::default()
        },
        ..Scenario::default()
    };

    let resolution = scenario.resolve(&fixture).expect("a resolved run");
    let effort = resolution
        .config
        .reasoning
        .effort
        .clone()
        .expect("a resolved effort");
    assert_eq!(effort.value, "high");
    assert_eq!(effort.source.layer, Layer::CommandLine);
    // The flag is the control's name, not the key path, so the diagnostic
    // names an option the user can actually type.
    assert_eq!(effort.source.to_string(), "command-line flag `--effort`");

    let explanation = resolution
        .provenance
        .explain("reasoning.effort")
        .expect("reasoning provenance");
    assert_eq!(explanation.source.layer, Layer::CommandLine);
    assert_eq!(
        explanation
            .overridden
            .iter()
            .map(|entry| entry.source.layer)
            .collect::<Vec<_>>(),
        vec![Layer::Environment, Layer::Profile]
    );
    // The overridden profile entry stays visible and unchanged.
    assert_eq!(
        explanation
            .overridden
            .iter()
            .find(|entry| entry.source.layer == Layer::Profile)
            .map(|entry| entry.value.clone()),
        Some(SettingValue::Text("low".to_owned()))
    );
}

#[test]
fn an_in_session_effort_still_outranks_an_invocation_effort() {
    let fixture = Fixture::new();
    let scenario = Scenario {
        cli: Overrides {
            reasoning_effort: Some("high".to_owned()),
            ..Overrides::default()
        },
        session: Overrides {
            reasoning_effort: Some("low".to_owned()),
            ..Overrides::default()
        },
        ..Scenario::default()
    };

    let resolution = scenario.resolve(&fixture).expect("a resolved run");
    let effort = resolution
        .config
        .reasoning
        .effort
        .clone()
        .expect("a resolved effort");
    assert_eq!(effort.value, "low");
    assert_eq!(effort.source.layer, Layer::SessionOverride);
}

#[test]
fn an_invocation_effort_leaves_the_rest_of_the_profile_alone() {
    let fixture = Fixture::new();
    let scenario = Scenario {
        profile: vec!["[profiles.work.reasoning]\nenabled = true\neffort = \"low\"\n".to_owned()],
        cli: Overrides {
            profile: Some("work".to_owned()),
            reasoning_effort: Some("high".to_owned()),
            ..Overrides::default()
        },
        ..Scenario::default()
    };

    let resolution = scenario.resolve(&fixture).expect("a resolved run");
    assert_eq!(resolution.config.profile.expect("a profile").value, "work");
    assert_eq!(resolution.config.provider.name.value, "acme");
    assert_eq!(resolution.config.model.value, "example-model");
    assert_eq!(resolution.config.model.source.layer, Layer::Profile);
    let enabled = resolution
        .config
        .reasoning
        .enabled
        .expect("the profile's thinking state");
    assert!(enabled.value);
    assert_eq!(enabled.source.layer, Layer::Profile);
    assert_eq!(
        resolution
            .config
            .reasoning
            .effort
            .expect("the flag's effort")
            .value,
        "high"
    );
}
