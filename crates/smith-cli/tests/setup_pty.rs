//! Process-level setup contracts under a real pseudo-terminal.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

const FAKE_CONFIG: &str = r#"
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
"#;

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

    fn configure_fake(&self) {
        let directory = self.project.path().join(".smith");
        std::fs::create_dir_all(&directory).expect("project config directory");
        std::fs::write(directory.join("config.toml"), FAKE_CONFIG).expect("fake config");
    }

    fn configure_user(&self, config: &str) {
        let directory = self.home.path().join(".smith");
        std::fs::create_dir_all(&directory).expect("user config directory");
        std::fs::write(directory.join("config.toml"), config).expect("user config");
    }

    fn configure_command_bridge(&self) -> PathBuf {
        let executable = self.project.path().join("command-bridge.sh");
        let process_log = self.project.path().join("command-processes.log");
        let script = r#"#!/bin/sh
log=$1
shift
printf '%s\n' "$1" >> "$log"
if [ "$1" = "--smith-provider-probe" ]; then
  printf '%s\n' "{\"protocol\":\"smith-command-provider\",\"schema_version\":1,\"model\":\"$2\",\"implementation\":\"tui-fixture\",\"implementation_version\":\"1.0.0\"}"
  exit 0
fi
if [ "$1" = "--smith-provider-attempt" ]; then
  IFS= read -r request
  printf '%s\n' '{"protocol":"smith-command-provider","schema_version":1,"type":"text_delta","text":"Hello from the command bridge."}'
  printf '%s\n' '{"protocol":"smith-command-provider","schema_version":1,"type":"usage","input_tokens":12,"output_tokens":6}'
  printf '%s\n' '{"protocol":"smith-command-provider","schema_version":1,"type":"finish","reason":"stop"}'
  exit 0
fi
exit 2
"#;
        std::fs::write(&executable, script).expect("a command bridge fixture");
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
            .expect("an executable bridge fixture");

        let config = format!(
            r#"
default_profile = "local"

[profiles.local]
provider = "bridge"
model = "local-model"

[providers.bridge]
kind = "command-jsonl"

[providers.bridge.command]
executable = {executable:?}
args = [{process_log:?}]
cwd = "workspace"

[models."bridge/local-model"]
context_tokens = 32768
max_input_tokens = 28672
max_output_tokens = 4096
"#,
            executable = executable.display().to_string(),
            process_log = process_log.display().to_string(),
        );
        self.configure_user(&config);
        let config_path = self.home.path().join(".smith/config.toml");
        std::fs::set_permissions(config_path, std::fs::Permissions::from_mode(0o600))
            .expect("an owner-only user config");
        process_log
    }

    fn install_fresh_catalog_cache(&self) {
        let cache = self.home.path().join(".smith/cache");
        std::fs::create_dir_all(&cache).expect("catalog cache directory");
        let mut snapshot: serde_json::Value =
            serde_json::from_str(smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED)
                .expect("embedded catalog");
        snapshot["retrieved_at_ms"] = serde_json::json!(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("current time")
                .as_millis()
        );
        std::fs::write(
            cache.join("models-dev-v1.json"),
            serde_json::to_vec(&snapshot).expect("catalog JSON"),
        )
        .expect("catalog cache");
    }

    fn run_headless(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_smith"))
            .args(args)
            .current_dir(self.project.path())
            .env_clear()
            .env("HOME", self.home.path())
            .output()
            .expect("headless Smith process")
    }

    fn run_expect(&self, smith_args: &str, interaction: &str) -> Option<Output> {
        self.run_expect_sized(smith_args, interaction, 32, 100)
    }

    fn run_expect_sized(
        &self,
        smith_args: &str,
        interaction: &str,
        rows: u16,
        columns: u16,
    ) -> Option<Output> {
        if !std::path::Path::new("/usr/bin/expect").is_file() {
            return None;
        }
        let shell = format!(
            "stty rows {rows} cols {columns}; before=$(stty -g); \
             \"$TUI_TEST_BIN\" {smith_args}; code=$?; \
             after=$(stty -g); if [ \"$before\" = \"$after\" ]; then \
             echo TERMINAL_RESTORED; else echo TERMINAL_DAMAGED; fi; exit \"$code\""
        );
        let script = format!(
            "set timeout 10\n\
             spawn -noecho /bin/sh -c {{{shell}}}\n\
             {interaction}\n\
             expect eof\n\
             set result [wait]\n\
             exit [lindex $result 3]\n"
        );
        Some(
            Command::new("/usr/bin/expect")
                .args(["-c", &script])
                .current_dir(self.project.path())
                .env_clear()
                .env("HOME", self.home.path())
                .env("PATH", "/usr/bin:/bin")
                .env("TERM", "xterm-256color")
                .env("TUI_TEST_BIN", env!("CARGO_BIN_EXE_smith"))
                .env("ZAI_API_KEY", "test-only-no-network-key")
                .env("OPENROUTER_API_KEY", "test-only-no-network-key")
                .output()
                .expect("expect-driven pseudo-terminal process"),
        )
    }
}

#[test]
fn cancelling_setup_restores_the_terminal_and_writes_nothing() {
    for args in ["--no-color --no-motion", "setup --no-color --no-motion"] {
        let fixture = Fixture::new();
        let interaction = r#"
expect {
    -exact "Smith setup" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "\033"
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
        let Some(output) = fixture.run_expect(args, interaction) else {
            return;
        };
        let screen = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "screen: {screen}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(screen.contains("Smith setup"), "{screen}");
        assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");
        assert!(!screen.contains("TERMINAL_DAMAGED"), "{screen}");
        let message = "Setup cancelled · nothing was written";
        assert_eq!(screen.matches(message).count(), 1, "{screen}");
        assert!(
            screen
                .find("\x1b[?1049l")
                .expect("alternate screen restored")
                < screen.find(message).expect("cancel report"),
            "{screen}"
        );
        assert!(
            !fixture.home.path().join(".smith").exists(),
            "cancelled setup wrote user state"
        );
    }
}

#[test]
fn cancelling_after_failed_setup_check_prints_only_the_cancel_line() {
    for args in ["--no-color --no-motion", "setup --no-color --no-motion"] {
        let fixture = Fixture::new();
        let interaction = r#"
expect {
    -exact "Smith setup" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "1"
expect {
    -exact "Authentication" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "4"
expect {
    -exact "Environment variable" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "SMITH_SETUP_UNSET_API_KEY\r"
expect {
    -exact "Review" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "\r"
expect {
    -exact "resolves to nothing" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "\003"
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
        let Some(output) = fixture.run_expect(args, interaction) else {
            return;
        };
        let screen = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "screen: {screen}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!screen.contains("TERMINAL_DAMAGED"), "{screen}");
        // Keep the failed check in the alternate screen; after restoration the
        // only report is cancellation, with the terminal's cursor show removed.
        let (setup, restored) = screen.split_once("\x1b[?1049l").expect("restored terminal");
        assert!(setup.contains("resolves to nothing"), "{screen}");
        let (report, _) = restored
            .split_once("TERMINAL_RESTORED")
            .expect("terminal modes restored");
        assert_eq!(
            report.replace("\x1b[?25h", "").replace('\r', "").trim(),
            "Setup cancelled · nothing was written",
            "{screen}"
        );
        assert!(output.stderr.is_empty(), "{:?}", output.stderr);
        assert!(
            !fixture.home.path().join(".smith/config.toml").exists(),
            "cancelled setup retained a config write"
        );
    }
}

#[test]
fn fresh_glm_setup_reaches_authentication_and_can_cancel_cleanly() {
    let fixture = Fixture::new();
    let interaction = r#"
expect {
    -exact "Smith setup" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "\r"
expect {
    -exact "Authentication" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "\033\[B\033\[B\033\[B\r"
expect {
    -exact "Environment variable" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "ZAI_API_KEY"
after 150
send -- "\033"
expect {
    -exact "Authentication" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "\003"
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
    let Some(output) = fixture.run_expect("--no-color --no-motion", interaction) else {
        return;
    };
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "screen: {screen}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");
    assert!(screen.contains("Environment variable"), "{screen}");
    assert!(
        !fixture.home.path().join(".smith").exists(),
        "authentication cancellation committed user state"
    );
}

fn setup_entry_advances_and_cancels(query: &str, next_step: &str) {
    let index =
        smith_config::setup::provider_descriptors(smith_runtime::factory::AVAILABLE_ADAPTER_KINDS)
            .iter()
            .position(|entry| entry.setup_id.contains(query))
            .expect("setup action")
            + 1;
    for args in ["--no-color --no-motion", "setup --no-color --no-motion"] {
        let fixture = Fixture::new();
        let interaction = format!(
            r#"
expect {{
    -exact "Smith setup" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
send -- "{index}"
expect {{
    -exact "{next_step}" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
send -- "\033"
expect {{
    -exact "Quick start with GLM" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
send -- "\033"
expect {{
    -exact "TERMINAL_RESTORED" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
"#
        );
        let Some(output) = fixture.run_expect(args, &interaction) else {
            return;
        };
        let screen = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{query} via {args}\nscreen: {screen}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(screen.contains(next_step), "{screen}");
        assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");
        assert!(!screen.contains("TERMINAL_DAMAGED"), "{screen}");
        assert!(
            !fixture.home.path().join(".smith").exists(),
            "cancelled {query} setup wrote user state"
        );
    }
}

#[test]
fn anthropic_setup_entry_reaches_authentication_and_cancels_without_writes() {
    setup_entry_advances_and_cancels("anthropic", "Authentication");
}

#[test]
fn chatgpt_setup_entry_reaches_oauth_picker_and_cancels_without_writes() {
    // Backing out of the method picker returns to setup before constructing
    // an OAuth client; Escape on setup's first step then cancels with no writes.
    setup_entry_advances_and_cancels("chatgpt", "Browser login");
}

#[test]
fn fresh_glm_setup_commits_then_enters_the_normal_tui() {
    let fixture = Fixture::new();
    let interaction = r#"
expect {
    -exact "Smith setup" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "\r"
after 300
send -- "\033\[B\033\[B\033\[B\r"
after 300
send -- "ZAI_API_KEY\r"
after 500
send -- "\r"
after 1000
expect {
    -exact "Ask Smith to do anything" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "/quit\r"
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
    let Some(output) = fixture.run_expect("--no-color --no-motion", interaction) else {
        return;
    };
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "screen: {screen}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(screen.contains("zai/glm-5.3"), "{screen}");
    assert!(
        !screen.contains("Setup cancelled · nothing was written"),
        "{screen}"
    );
    assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");
    let config = std::fs::read_to_string(fixture.home.path().join(".smith/config.toml"))
        .expect("committed GLM config");
    assert!(
        config.contains("credential = \"env:ZAI_API_KEY\""),
        "{config}"
    );
    assert!(config.contains("reasoning_only = \"text\""), "{config}");
}

#[test]
fn fresh_glm_inline_setup_is_masked_commits_owner_only_and_starts_smith() {
    use std::os::unix::fs::PermissionsExt;

    const SECRET: &str = "sk-pty-inline-must-not-render";
    let fixture = Fixture::new();
    let interaction = format!(
        r#"
expect {{
    -exact "Smith setup" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
send -- "\r"
after 300
send -- "\033\[B\033\[B\r"
expect {{
    -exact "API key" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
send -- "{SECRET}\r"
after 400
send -- "\r"
after 1000
expect {{
    -exact "Ask Smith to do anything" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
send -- "/quit\r"
expect {{
    -exact "TERMINAL_RESTORED" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
"#
    );
    let Some(output) = fixture.run_expect("--no-color --no-motion", &interaction) else {
        return;
    };
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "screen: {screen}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        screen.contains("plaintext") && screen.contains("same-user"),
        "{screen}"
    );
    assert!(screen.contains("[redacted]"), "{screen}");
    assert!(!screen.contains(SECRET), "{screen}");
    assert!(screen.contains("zai/glm-5.3"), "{screen}");
    assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");

    let path = fixture.home.path().join(".smith/config.toml");
    let config = std::fs::read_to_string(&path).expect("committed inline GLM config");
    assert!(
        config.contains(&format!("api_key = \"{SECRET}\"")),
        "{config}"
    );
    assert!(!config.contains("credential ="), "{config}");
    assert_eq!(
        std::fs::metadata(path)
            .expect("config metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn credential_command_migrates_only_the_selected_provider_with_redacted_review() {
    const SECRET: &str = "sk-pty-migration-must-not-render";
    const CONFIG: &str = r#"# preserve this user note
default_profile = "remote"

[profiles.remote]
provider = "zai"
model = "glm-4.7"
max_output_tokens = 8192

[providers.zai]
kind = "openai-compatible"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential = "keychain:smith/zai"

[models."zai/glm-4.7"]
context_tokens = 200000
max_input_tokens = 196000
max_output_tokens = 131072
"#;
    let fixture = Fixture::new();
    fixture.configure_user(CONFIG);
    let interaction = format!(
        r#"
expect {{
    -exact "Authentication" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
send -- "\033\[B\033\[B\r"
after 250
send -- "{SECRET}\r"
after 400
send -- "\r"
after 500
send -- "\r"
expect {{
    -exact "TERMINAL_RESTORED" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
"#
    );
    let Some(output) = fixture.run_expect(
        "setup credential --provider zai --no-color --no-motion",
        &interaction,
    ) else {
        return;
    };
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "screen: {screen}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(screen.contains("api_key"), "{screen}");
    assert!(screen.contains("[redacted]"), "{screen}");
    assert!(screen.contains("plaintext"), "{screen}");
    assert!(!screen.contains(SECRET), "{screen}");
    assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");

    let config = std::fs::read_to_string(fixture.home.path().join(".smith/config.toml"))
        .expect("migrated config");
    assert!(config.contains("# preserve this user note"), "{config}");
    assert!(
        config.contains("base_url = \"https://api.z.ai/api/coding/paas/v4\""),
        "{config}"
    );
    assert!(config.contains("model = \"glm-4.7\""), "{config}");
    assert!(config.contains("context_tokens = 200000"), "{config}");
    assert!(
        config.contains(&format!("api_key = \"{SECRET}\"")),
        "{config}"
    );
    assert!(!config.contains("credential ="), "{config}");
}

#[test]
fn add_provider_commits_a_second_provider_and_model_after_collision_review() {
    let fixture = Fixture::new();
    fixture.configure_user(FAKE_CONFIG);
    let interaction = r#"
expect {
    -exact "Provider name" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "openrouter\r"
after 250
send -- "http://127.0.0.1:9/v1\r"
after 250
send -- "\033\[B\033\[B\033\[B\r"
after 250
send -- "ZAI_API_KEY\r"
after 250
send -- "openai/gpt-test\r"
after 250
send -- "128000\r"
after 250
send -- "\r"
after 250
send -- "\r"
after 500
send -- "\r"
after 750
send -- "\r"
after 1000
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
    let Some(output) = fixture.run_expect("setup add-provider --no-color --no-motion", interaction)
    else {
        return;
    };
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "screen: {screen}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");
    let config = std::fs::read_to_string(fixture.home.path().join(".smith/config.toml"))
        .expect("updated user config");
    assert!(config.contains("[providers.openrouter]"), "{config}");
    assert!(
        config.contains("[models.\"openrouter/openai/gpt-test\"]"),
        "{config}"
    );
    assert!(config.contains("[providers.local]"), "{config}");
}

#[test]
fn add_model_accepts_a_valid_second_provider_that_has_no_models_yet() {
    let fixture = Fixture::new();
    fixture.configure_user(&format!(
        "{FAKE_CONFIG}\n\
         [providers.empty]\n\
         kind = \"openai-compatible\"\n\
         base_url = \"http://127.0.0.1:9/v1\"\n\
         credential = \"env:ZAI_API_KEY\"\n"
    ));
    let interaction = r#"
expect {
    -exact "Model ID" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "first-model\r"
after 250
send -- "64000\r"
after 250
send -- "\r"
after 500
send -- "\r"
after 750
send -- "\r"
after 1000
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
    let Some(output) = fixture.run_expect(
        "setup add-model --provider empty --no-color --no-motion",
        interaction,
    ) else {
        return;
    };
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "screen: {screen}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");
    let config = std::fs::read_to_string(fixture.home.path().join(".smith/config.toml"))
        .expect("updated user config");
    assert!(config.contains("[providers.empty]"), "{config}");
    assert!(
        config.contains("[models.\"empty/first-model\"]"),
        "{config}"
    );
}

#[test]
fn deterministic_host_enters_the_normal_tui_and_restores_the_terminal() {
    let fixture = Fixture::new();
    fixture.configure_fake();
    let interaction = r#"
expect {
    -exact "Ask Smith to do anything" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "/quit\r"
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
    let Some(output) = fixture.run_expect("--no-color --no-motion", interaction) else {
        return;
    };
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "screen: {screen}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(screen.contains("local/example-model"), "{screen}");
    assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");
}

#[test]
fn command_jsonl_provider_streams_a_turn_through_the_normal_tui() {
    let fixture = Fixture::new();
    let process_log = fixture.configure_command_bridge();
    let interaction = r#"
expect {
    -exact "Ask Smith to do anything" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "hello\r"
expect {
    -exact "bridge." {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "/quit\r"
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
    let Some(output) = fixture.run_expect("--no-color --no-motion", interaction) else {
        return;
    };
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "screen: {screen}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(screen.contains("bridge/local-model"), "{screen}");
    assert!(
        screen.contains("Hello") && screen.contains("bridge."),
        "{screen}"
    );
    assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");
    assert_eq!(
        std::fs::read_to_string(process_log).expect("process log"),
        "--smith-provider-probe\n--smith-provider-attempt\n",
        "the TUI uses one preflight process and one fresh process for the visible attempt"
    );
}

#[test]
fn deterministic_host_renders_in_real_narrow_normal_and_wide_terminals() {
    for (rows, columns) in [(14, 44), (24, 74), (32, 120)] {
        let fixture = Fixture::new();
        fixture.configure_fake();
        let interaction = r#"
expect {
    -exact "Ask Smith to do anything" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "/quit\r"
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
        let Some(output) =
            fixture.run_expect_sized("--no-color --no-motion", interaction, rows, columns)
        else {
            return;
        };
        let screen = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{columns}×{rows}\nscreen: {screen}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            screen.contains("local/example-model"),
            "{columns}×{rows} omitted model identity:\n{screen}"
        );
        assert!(
            !screen.contains("terminal too small"),
            "{columns}×{rows} incorrectly refused a supported viewport:\n{screen}"
        );
        assert!(
            screen.contains("TERMINAL_RESTORED") && !screen.contains("TERMINAL_DAMAGED"),
            "{columns}×{rows} did not restore terminal state:\n{screen}"
        );
    }
}

#[test]
fn configured_zai_and_openrouter_hosts_select_catalog_only_models_offline() {
    let cases = [
        (
            r#"
default_profile = "glm"
[profiles.glm]
provider = "zai"
model = "glm-4.7"
max_output_tokens = 8192
[providers.zai]
kind = "openai-compatible"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential = "env:ZAI_API_KEY"
[context]
output_reserve = 8192
"#,
            "glm-5-turbo",
            "GLM-5-Turbo",
            "zai/glm-5-turbo",
        ),
        (
            r#"
default_profile = "router"
[profiles.router]
provider = "openrouter"
model = "openai/gpt-5.2"
max_output_tokens = 8192
[providers.openrouter]
kind = "openai-compatible"
base_url = "https://openrouter.ai/api/v1"
credential = "env:OPENROUTER_API_KEY"
[context]
output_reserve = 8192
"#,
            "claude-opus-4.6",
            "Claude Opus 4.6",
            "openrouter/anthropic/claude-opus-4.6",
        ),
    ];

    for (config, query, _label, selected) in cases {
        let fixture = Fixture::new();
        fixture.configure_user(config);
        fixture.install_fresh_catalog_cache();
        let interaction = format!(
            r#"
expect {{
    -exact "Ask Smith to do anything" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
send -- "/model\r"
expect {{
    -exact "Choose model" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
# Confirmation and following input share one write, with no rebuild wait.
send -- "{query}\rtyped right after the switch"
expect {{
    -exact "typed right after the switch" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
send -- "\025/model\r"
expect {{
    -exact "Choose model" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
# Exercise the original flake: /quit immediately follows another switch.
send -- "{query}\r/quit\r"
expect {{
    -exact "TERMINAL_RESTORED" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
"#
        );
        let Some(output) = fixture.run_expect("--no-color --no-motion", &interaction) else {
            return;
        };
        let screen = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "selection {selected}\nscreen: {screen}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(screen.contains(query), "{screen}");
        assert!(screen.contains(selected), "{screen}");
        assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");
    }
}

#[test]
fn bare_resume_uses_the_pre_host_picker_then_resumes_the_selected_session() {
    let fixture = Fixture::new();
    fixture.configure_fake();
    let seeded = fixture.run_headless(&["-p", "resume picker seed", "--output-format", "json"]);
    assert!(
        seeded.status.success(),
        "{}",
        String::from_utf8_lossy(&seeded.stderr)
    );
    let result: serde_json::Value =
        serde_json::from_slice(&seeded.stdout).expect("seed result JSON");
    let session = result["session_id"].as_str().expect("session ID");

    let interaction = r#"
expect {
    -exact "Resume session" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "\r"
expect {
    -exact "Ask Smith to do anything" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "/quit\r"
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
    let Some(output) = fixture.run_expect("--resume --no-color --no-motion", interaction) else {
        return;
    };
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "screen: {screen}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        screen.contains(&format!("resume with smith --resume {session}")),
        "{screen}"
    );
    assert!(screen.contains("resume picker seed"), "{screen}");
    assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");
}

#[test]
fn quitting_without_a_user_message_omits_the_resume_hint() {
    let fixture = Fixture::new();
    fixture.configure_fake();
    let interaction = r#"
expect {
    -exact "Ask Smith to do anything" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "/quit\r"
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
    let Some(output) = fixture.run_expect("--no-color --no-motion", interaction) else {
        return;
    };
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "screen: {screen}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(screen.contains("TERMINAL_RESTORED"), "{screen}");
    assert!(!screen.contains("resume with smith --resume"), "{screen}");

    let listed = fixture.run_headless(&["sessions", "list"]);
    assert!(listed.status.success());
    let listing = String::from_utf8_lossy(&listed.stdout);
    assert!(
        listing.is_empty(),
        "piped listing must omit empty sessions: {listing}"
    );
    let sessions = fixture.home.path().join(".smith/sessions");
    for project in std::fs::read_dir(sessions).expect("session projects") {
        let project = project.expect("project directory");
        for file in std::fs::read_dir(project.path()).expect("session files") {
            let file = file.expect("session file");
            let name = file.file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.ends_with(".snapshot.json"),
                "empty snapshot survived: {name}"
            );
            assert!(
                !name.ends_with(".session.lock"),
                "empty lease survived: {name}"
            );
            assert!(
                !name.ends_with(".shell.jsonl"),
                "empty shell sidecar survived: {name}"
            );
            assert!(
                !name.ends_with(".checkpoint.bin"),
                "empty checkpoint survived: {name}"
            );
            assert!(
                !name.ends_with(".checkpoint.lock"),
                "empty checkpoint lease survived: {name}"
            );
            assert!(
                !name.ends_with(".jsonl")
                    || name == "usage.jsonl"
                    || name.ends_with(".changes.jsonl"),
                "empty journal survived: {name}"
            );
        }
    }

    let empty_resume = r#"
expect {
    -exact "No sessions to resume in this project · esc exits" {}
    timeout { exit 124 }
    eof { exit 125 }
}
send -- "\033"
expect {
    -exact "TERMINAL_RESTORED" {}
    timeout { exit 124 }
    eof { exit 125 }
}
"#;
    let output = fixture
        .run_expect("--resume --no-color --no-motion", empty_resume)
        .expect("expect was available for the first invocation");
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{screen}");
    assert!(screen.contains("sessions"), "{screen}");
    assert!(screen.contains("resume"), "{screen}");
    assert!(screen.contains("exits"), "{screen}");
    assert!(!screen.contains("Ask Smith to do anything"), "{screen}");
    let after_cancel = fixture.run_headless(&["sessions", "list"]);
    assert!(after_cancel.status.success());
    assert_eq!(
        after_cancel.stdout, listed.stdout,
        "cancel created a session"
    );
}
