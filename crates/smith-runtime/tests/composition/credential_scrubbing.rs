use super::*;

use agent_runtime::context::ContextBudget;
use smith_module::{
    CompiledModule, Module, ModuleComposition, ModuleContext, ModuleError, ModuleOrigin, Mounted,
};
use smith_runtime::background_tasks::{BackgroundServices, TaskStatus};

#[test]
fn resolved_credential_env_vars_extracts_primary_pool_fallback_web_and_checkpoint_vars() {
    let fixture = Fixture::new_private_user(
        r#"
default_profile = "dev"

[profiles.dev]
provider = "primary"
model = "primary-model"

[profiles.dev.fallback]
models = [{ provider = "backup", model = "fallback-model" }]

[providers.primary]
kind = "openai-compatible"
base_url = "https://primary.example.test/v1"
credentials = ["env:ZAI_API_KEY", "env:ZAI_API_KEY_ALT", "keychain:smith/primary"]

[providers.backup]
kind = "openai-compatible"
base_url = "https://backup.example.test/v1"
credential = "env:FALLBACK_API_KEY"

[models."primary/primary-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[models."backup/fallback-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[tools.web_search]
mode = "backend"
backend = "brave"
api_key = "env:BRAVE_SEARCH_KEY"

[persistence]
checkpoint_key_credential = "env:SMITH_CHECKPOINT_KEY"

[approval]
mode = "allow-all"
"#,
    );

    let config = fixture.config();
    let vars = factory::resolved_credential_env_vars(&config);

    assert_eq!(
        vars.as_ref(),
        &[
            "BRAVE_SEARCH_KEY".to_owned(),
            "FALLBACK_API_KEY".to_owned(),
            "SMITH_CHECKPOINT_KEY".to_owned(),
            "ZAI_API_KEY".to_owned(),
            "ZAI_API_KEY_ALT".to_owned(),
        ]
    );
}

#[test]
fn request_credential_env_vars_collects_vision_advisor_compaction_and_child_profiles() {
    let primary = Fixture::new(
        r#"
default_profile = "dev"
[profiles.dev]
provider = "primary"
model = "m"
[providers.primary]
kind = "openai-compatible"
base_url = "https://primary.example.test/v1"
credential = "env:PRIMARY_KEY"
[models."primary/m"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#,
    );
    let extra = |env_name: &str| {
        Fixture::new(&format!(
            r#"
default_profile = "dev"
[profiles.dev]
provider = "p"
model = "m"
[providers.p]
kind = "openai-compatible"
base_url = "https://p.example.test/v1"
credential = "env:{env_name}"
[models."p/m"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#
        ))
        .config()
    };

    let mut req = RuntimeRequest::new(primary.config(), HostSurface::Headless);
    req.vision_profile = Some(Box::new(factory::VisionProfileRequest {
        config: extra("VISION_KEY"),
        catalog_sources: Vec::new(),
        provider: None,
    }));
    req.advisor_profile = Some(factory::AdvisorProfileRequest {
        config: extra("ADVISOR_KEY"),
        catalog_sources: Vec::new(),
        provider: None,
    });
    req.compaction_profile = Some(Box::new(factory::VisionProfileRequest {
        config: extra("COMPACTION_KEY"),
        catalog_sources: Vec::new(),
        provider: None,
    }));
    req.child_profiles.push(factory::ChildProfileRequest {
        vision_profile: Some(Box::new(factory::VisionProfileRequest {
            config: extra("CHILD_VISION_KEY"),
            catalog_sources: Vec::new(),
            provider: None,
        })),
        config: extra("CHILD_KEY"),
        catalog_sources: Vec::new(),
    });

    let vars = factory::request_credential_env_vars(&req);
    assert_eq!(
        vars.as_ref(),
        &[
            "ADVISOR_KEY".to_owned(),
            "CHILD_KEY".to_owned(),
            "CHILD_VISION_KEY".to_owned(),
            "COMPACTION_KEY".to_owned(),
            "PRIMARY_KEY".to_owned(),
            "VISION_KEY".to_owned(),
        ]
    );
}

#[tokio::test]
async fn built_runtime_scrubs_resolved_credential_env_vars_from_foreground_and_background_shell() {
    let secret_var = ["CARGO_PKG_NAME", "CARGO_MANIFEST_DIR", "HOME", "USER"]
        .into_iter()
        .find(|name| std::env::var(name).is_ok_and(|value| !value.is_empty()))
        .expect("test process has an inherited non-PATH environment variable");

    let config = format!(
        r#"
default_profile = "dev"

[profiles.dev]
provider = "zai"
model = "glm-5"

[providers.zai]
kind = "openai-compatible"
base_url = "https://zai.example.test/v1"
credential = "env:{secret_var}"

[models."zai/glm-5"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[approval]
mode = "allow-all"
"#
    );
    let fixture = Fixture::new(&config);
    let fg_cmd = format!(
        "printf 'fg_secret=%s\\nfg_path=%s\\n' \"${{{secret_var}:-UNSET}}\" \"${{PATH:-UNSET}}\""
    );
    let bg_cmd = format!(
        "printf 'bg_secret=%s\\nbg_path=%s\\n' \"${{{secret_var}:-UNSET}}\" \"${{PATH:-UNSET}}\""
    );

    let mut fg_events = tool_call_fragments(
        0,
        "call-fg",
        "shell",
        &serde_json::json!({"command": fg_cmd}).to_string(),
    );
    fg_events.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let mut bg_events = tool_call_fragments(
        0,
        "call-bg",
        "shell",
        &serde_json::json!({"command": bg_cmd, "run_in_background": true}).to_string(),
    );
    bg_events.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });

    let provider = Arc::new(FakeProvider::new(
        "glm-5",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(fg_events),
            ScriptedStream::new(bg_events),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "done".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));

    let bg_services = BackgroundServices::new();
    let mut req = request(&fixture, HostSurface::Headless);
    req.workspace = Some(Arc::new(
        ProjectWorkspace::new(fixture.project.path()).expect("project workspace"),
    ));
    req.provider = Some(provider.clone());
    req.background_services = Some(bg_services.clone());

    let smith = factory::build_request(req).await.expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("check env scrubbing"))
        .await
        .expect("turn runs");

    let requests = provider.requests();
    let fg_wire = serde_json::to_string(&requests[1].messages).expect("fg tool result");
    assert!(
        fg_wire.contains("fg_secret=UNSET"),
        "foreground shell must scrub `{secret_var}`: {fg_wire}"
    );
    assert!(
        !fg_wire.contains("fg_path=UNSET"),
        "foreground shell must preserve PATH: {fg_wire}"
    );

    let bg_wire = serde_json::to_string(&requests[2].messages).expect("bg tool result");
    let task_id = "task:1";
    let mut final_status = TaskStatus::Running;
    for _ in 0..200 {
        let out = bg_services
            .registry()
            .get_task_output(session.id(), task_id, 0, 65536)
            .await
            .expect("background task output");
        if out.status.is_terminal() {
            final_status = out.status;
            assert!(
                out.output.contains("bg_secret=UNSET"),
                "background shell must scrub `{secret_var}`: {} (wire: {bg_wire})",
                out.output
            );
            assert!(
                !out.output.contains("bg_path=UNSET"),
                "background shell must preserve PATH: {}",
                out.output
            );
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(final_status, TaskStatus::Exited { code: Some(0) });

    session.shutdown().await.expect("clean shutdown");
}

#[derive(Debug)]
struct BudgetCaptureModule {
    observed_budget: Arc<Mutex<Option<u32>>>,
}

impl Module for BudgetCaptureModule {
    fn id(&self) -> &str {
        "budget-capture"
    }

    fn revision(&self) -> &str {
        "budget-capture-v1"
    }

    fn description(&self) -> &str {
        "Captures the input budget handed to modules"
    }

    fn default_enabled(&self) -> bool {
        true
    }

    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError> {
        *self.observed_budget.lock().expect("budget lock") = Some(context.max_input_tokens);
        Ok(Mounted::Contributions(Vec::new()))
    }
}

#[tokio::test]
async fn modules_receive_enforceable_context_budget_input_budget_not_raw_max_input_tokens() {
    let fixture = Fixture::new(
        r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 100000
max_input_tokens = 100000
max_output_tokens = 10000

[context]
output_reserve = 10000
reasoning_reserve = 2000

[approval]
mode = "allow-all"
"#,
    );

    let observed = Arc::new(Mutex::new(None));
    let mut req = request(&fixture, HostSurface::Headless);
    req.modules = ModuleComposition::with_defaults(
        vec![CompiledModule {
            module: Arc::new(BudgetCaptureModule {
                observed_budget: Arc::clone(&observed),
            }),
            origin: ModuleOrigin::FirstParty,
        }],
        vec![],
    );

    let smith = factory::build_request(req).await.expect("a runtime");
    let expected = ContextBudget::from_limits(
        &smith.policy().model_profile.limits,
        &smith.policy().context_policy,
    )
    .input_budget;
    assert_eq!(
        expected, 88_000,
        "context_tokens(100k) - output_reserve(10k) - reasoning_reserve(2k)"
    );
    assert_eq!(
        *observed.lock().expect("budget lock"),
        Some(expected),
        "modules::prepare must receive ContextBudget::from_limits().input_budget rather than raw max_input_tokens (100_000)"
    );
}
