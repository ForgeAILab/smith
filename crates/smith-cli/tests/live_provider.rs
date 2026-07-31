//! Opt-in black-box proof against a real OpenAI-compatible provider.
//!
//! This test is ignored by default because it performs network I/O and spends
//! provider quota. See the Smith README for the required environment.

use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;

const API_KEY_ENV: &str = "SMITH_LIVE_API_KEY";
const CHILD_API_KEY_ENV: &str = "AGENT_RUNTIME_LIVE_API_KEY";
const MARKER: &str = "smith-live-provider-proof-4c2f8a";
const MAX_OUTPUT_TOKENS_PER_REQUEST: u32 = 2_048;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(150);

struct LiveConfig {
    base_url: String,
    model: String,
    api_key: String,
    context_tokens: u32,
    max_input_tokens: u32,
    max_output_tokens: u32,
}

impl LiveConfig {
    fn from_env() -> Self {
        Self {
            base_url: required("SMITH_LIVE_BASE_URL"),
            model: required("SMITH_LIVE_MODEL"),
            api_key: required(API_KEY_ENV),
            context_tokens: positive_u32("SMITH_LIVE_CONTEXT_TOKENS"),
            max_input_tokens: positive_u32("SMITH_LIVE_MAX_INPUT_TOKENS"),
            max_output_tokens: positive_u32("SMITH_LIVE_MAX_OUTPUT_TOKENS"),
        }
    }
}

fn required(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| panic!("set `{name}` before running the ignored live-provider test"))
}

fn positive_u32(name: &str) -> u32 {
    let raw = required(name);
    raw.parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or_else(|| panic!("`{name}` must be a positive 32-bit integer"))
}

fn toml_string(value: &str) -> String {
    // JSON string escaping is a subset of TOML basic-string escaping for the
    // endpoint/model values accepted here.
    serde_json::to_string(value).expect("a TOML-compatible quoted string")
}

#[tokio::test]
#[ignore = "requires explicit live-provider environment and spends provider quota"]
async fn configured_live_provider_completes_a_streaming_tool_turn() {
    let live = LiveConfig::from_env();
    let project = tempfile::tempdir().expect("a temporary live-test project");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a project config directory");
    std::fs::write(project.path().join("live-proof.txt"), format!("{MARKER}\n"))
        .expect("a live tool fixture");

    let config = format!(
        r#"
default_profile = "live"

[profiles.live]
provider = "live"
model = {model}
max_output_tokens = {request_output}

[providers.live]
kind = "openai-compatible"
base_url = {base_url}
credential = "env:{api_key_env}"

[models.{model_table_key}]
context_tokens = {context_tokens}
max_input_tokens = {max_input_tokens}
max_output_tokens = {max_output_tokens}

[context]
output_reserve = {request_output}
reasoning_reserve = 0
capability_budget = 12000

[limits]
max_retries = 0
max_tool_steps = 2
turn_time_limit_ms = 120000
tool_output_limit_bytes = 16384
"#,
        model = toml_string(&live.model),
        request_output = MAX_OUTPUT_TOKENS_PER_REQUEST,
        base_url = toml_string(&live.base_url),
        api_key_env = CHILD_API_KEY_ENV,
        model_table_key = toml_string(&format!("live/{}", live.model)),
        context_tokens = live.context_tokens,
        max_input_tokens = live.max_input_tokens,
        max_output_tokens = live.max_output_tokens,
    );
    assert!(
        !config.contains(&live.api_key),
        "the generated config contained credential material"
    );
    std::fs::write(config_dir.join("config.toml"), config).expect("a live provider config");

    let prompt = "Use the read tool to inspect live-proof.txt, then tell me in one concise sentence \
         what value the file contains.";
    let mut command = Command::new(env!("CARGO_BIN_EXE_smith"));
    command
        .current_dir(project.path())
        .args([
            "-p",
            prompt,
            "--output-format",
            "stream-json",
            "--approval",
            "allow-all",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (name, _) in
        std::env::vars_os().filter(|(name, _)| name.to_string_lossy().starts_with("SMITH_"))
    {
        command.env_remove(name);
    }
    command
        .env(CHILD_API_KEY_ENV, &live.api_key)
        .env("SMITH_PERSISTENCE_ENABLED", "false");

    let child = command.spawn().expect("the Smith live process started");
    let output = tokio::time::timeout(PROCESS_TIMEOUT, child.wait_with_output())
        .await
        .expect("the Smith live process exceeded its 150-second outer deadline")
        .expect("the Smith live process completed");

    let stdout = String::from_utf8(output.stdout).expect("live stdout was UTF-8 JSON Lines");
    let stderr = String::from_utf8(output.stderr).expect("live stderr was UTF-8");
    assert!(
        !stdout.contains(&live.api_key) && !stderr.contains(&live.api_key),
        "the live process exposed credential material"
    );
    assert!(
        output.status.success(),
        "live Smith exited {:?}: {stderr}",
        output.status.code()
    );
    assert!(stderr.is_empty(), "live Smith wrote diagnostics: {stderr}");

    let lines: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON value per output line"))
        .collect();
    let (result, events) = lines.split_last().expect("a terminal result");
    assert_eq!(result["type"], "result");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["provider"], "live");
    assert_eq!(result["model"], live.model.as_str());
    assert_eq!(result["error"], Value::Null);
    assert!(
        result["output"]
            .as_str()
            .is_some_and(|text| text.contains(MARKER)),
        "the final answer did not report the tool-only marker: {:?}",
        result["output"]
    );
    assert_eq!(
        result["usage"]["current_turn_provenance"],
        "provider_reported"
    );
    assert!(
        result["usage"]["current_turn"]["output"]
            .as_u64()
            .is_some_and(|tokens| tokens > 0),
        "the provider reported no output usage"
    );

    let event_named = |name: &str| {
        events
            .iter()
            .filter(|line| line["event"]["payload"]["event"] == name)
            .count()
    };
    assert!(
        event_named("provider_attempt_started") >= 2,
        "a tool-assisted turn must make a continuation request"
    );
    assert!(
        events.iter().any(|line| {
            line["event"]["payload"]["event"] == "tool_call_requested"
                && line["event"]["payload"]["name"] == "read"
        }),
        "the live model never requested Smith's read tool"
    );
    assert!(
        events.iter().any(|line| {
            line["event"]["payload"]["event"] == "tool_call_completed"
                && line["event"]["payload"]["name"] == "read"
                && line["event"]["payload"]["is_error"] == false
        }),
        "Smith's live read tool did not complete successfully"
    );
    assert_eq!(event_named("turn_completed"), 1);
    assert_eq!(event_named("session_shutdown"), 1);
}
