//! Selection-inventory behavior for installed coding agents and per-candidate
//! preview budgets.

use smith_config::cli_agents::{
    CLI_AGENT_CONTEXT_TOKENS, CLI_AGENT_MAX_INPUT_TOKENS, CLI_AGENT_MAX_OUTPUT_TOKENS,
};
use smith_config::inventory::{ModelLimitOrigin, local_inventory};
use smith_config::resolve::{ResolveRequest, resolve};

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().expect("a home"),
            project: tempfile::tempdir().expect("a project"),
        }
    }

    fn write_user(&self, text: &str) {
        let directory = self.home.path().join(".smith");
        std::fs::create_dir_all(&directory).expect("a user config directory");
        let path = directory.join("config.toml");
        std::fs::write(&path, text).expect("a user config");
        let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o600);
        std::fs::set_permissions(&path, permissions).expect("owner-only permissions");
    }

    fn inventory(
        &self,
        cli: smith_config::resolve::Overrides,
    ) -> smith_config::inventory::SelectionInventory {
        let request = ResolveRequest::new(self.project.path())
            .with_home_dir(self.home.path())
            .with_cli(cli);
        let resolution = resolve(&request).expect("ready config");
        local_inventory(&resolution, &["openai-compatible", "gemini-interactions"])
            .expect("local inventory")
    }
}

/// The active profile pins a request and reserve; `cc` selects an installed
/// agent with no `[models]` declaration for it.
const PROFILE_PINNED: &str = r#"
default_profile = "code"
profile_order = ["code", "cc"]

[profiles.code]
provider = "zai"
model = "glm-5.3"
max_output_tokens = 32768
posture = "build"
use = ["main", "child"]

[profiles.code.context]
output_reserve = 8192

[profiles.cc]
provider = "google"
model = "cli/claude-code/sonnet"
posture = "build"
use = ["main", "child"]

[providers.zai]
kind = "openai-compatible"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential = "env:ZAI_API_KEY"

[providers.google]
kind = "gemini-interactions"
credential = "env:GEMINI_API_KEY"

[models."zai/glm-5.3"]
context_tokens = 1000000
max_input_tokens = 1000000
max_output_tokens = 131072
"#;

#[test]
fn installed_agent_profile_enumerates_and_is_selectable_without_declaration() {
    let fixture = Fixture::new();
    fixture.write_user(PROFILE_PINNED);
    let inventory = fixture.inventory(Default::default());

    let entry = inventory
        .models
        .iter()
        .find(|entry| entry.id() == "google/cli/claude-code/sonnet")
        .expect("the installed-agent pair enumerates");
    assert!(entry.selectable, "disabled: {:?}", entry.disabled_reason);
    assert_eq!(
        entry.context_tokens.as_ref().unwrap().value,
        CLI_AGENT_CONTEXT_TOKENS
    );
    assert_eq!(
        entry.max_input_tokens.as_ref().unwrap().value,
        CLI_AGENT_MAX_INPUT_TOKENS
    );
    assert_eq!(
        entry.max_output_tokens.as_ref().unwrap().value,
        CLI_AGENT_MAX_OUTPUT_TOKENS
    );
    for limit in [
        entry.context_tokens.as_ref().unwrap(),
        entry.max_input_tokens.as_ref().unwrap(),
        entry.max_output_tokens.as_ref().unwrap(),
    ] {
        assert_eq!(limit.origin, ModelLimitOrigin::BuiltIn);
    }

    let profile = inventory
        .profiles
        .iter()
        .find(|profile| profile.name == "cc")
        .expect("the cc profile");
    assert!(
        profile.selectable,
        "a resolving profile must not be unavailable"
    );
}

#[test]
fn explicit_models_table_overrides_builtin_bookkeeping() {
    let fixture = Fixture::new();
    fixture.write_user(&format!(
        "{PROFILE_PINNED}\n[models.\"google/cli/claude-code/sonnet\"]\ncontext_tokens = 128000\nmax_input_tokens = 120000\nmax_output_tokens = 16384\n"
    ));
    let inventory = fixture.inventory(Default::default());

    let entry = inventory
        .models
        .iter()
        .find(|entry| entry.id() == "google/cli/claude-code/sonnet")
        .expect("the pair enumerates");
    assert!(entry.selectable, "disabled: {:?}", entry.disabled_reason);
    assert_eq!(entry.context_tokens.as_ref().unwrap().value, 128_000);
    assert_eq!(entry.max_input_tokens.as_ref().unwrap().value, 120_000);
    assert_eq!(entry.max_output_tokens.as_ref().unwrap().value, 16_384);
    assert!(matches!(
        entry.context_tokens.as_ref().unwrap().origin,
        ModelLimitOrigin::Configured(_)
    ));
}

#[test]
fn active_profile_request_and_reserve_do_not_disable_other_candidates() {
    let fixture = Fixture::new();
    fixture.write_user(PROFILE_PINNED);
    let inventory = fixture.inventory(Default::default());

    // The installed-agent candidate previews its own automatic budget bounded
    // by its 32,000-token bookkeeping ceiling, not the active profile's
    // pinned 32,768 request.
    let candidate = inventory
        .models
        .iter()
        .find(|entry| entry.id() == "google/cli/claude-code/sonnet")
        .expect("the candidate");
    assert!(
        candidate.selectable,
        "disabled: {:?}",
        candidate.disabled_reason
    );
    assert_eq!(candidate.output_budget.unwrap().request_tokens, 32_000);

    // The active pair keeps its own effective policy.
    let active = inventory
        .models
        .iter()
        .find(|entry| entry.id() == "zai/glm-5.3")
        .expect("the active pair");
    assert!(active.active);
    assert_eq!(active.output_budget.unwrap().request_tokens, 32_768);
}

#[test]
fn persistent_global_request_still_conflicts_honestly() {
    // A command-line request is not scoped to the active profile: it survives
    // a profile or model switch, so it remains active policy while previewing
    // another candidate and conflicts with its smaller ceiling.
    let fixture = Fixture::new();
    fixture.write_user(PROFILE_PINNED);
    let inventory = fixture.inventory(smith_config::resolve::Overrides {
        max_output_tokens: Some(40_000),
        ..Default::default()
    });

    let candidate = inventory
        .models
        .iter()
        .find(|entry| entry.id() == "google/cli/claude-code/sonnet")
        .expect("the candidate");
    assert!(!candidate.selectable);
    let reason = candidate.disabled_reason.as_deref().unwrap();
    assert!(
        reason.contains("40000") && reason.contains("32000"),
        "bounded reason names the request and the ceiling: {reason}"
    );
}
