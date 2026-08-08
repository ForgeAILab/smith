#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use smith_runtime::checkpoint::{
        CheckpointKey, CheckpointKeyProvider, CheckpointProtectionError,
    };
    use smith_tui::{Block, LocalResultState};

    #[derive(Debug)]
    struct TestCheckpointKeys;

    impl CheckpointKeyProvider for TestCheckpointKeys {
        fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
            Ok(CheckpointKey::new([0x52; 32]))
        }
    }

    const LOCAL_COMMAND_CONFIG: &str = r#"
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

    fn git(project: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .args(arguments)
            .current_dir(project)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("git");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    include!("host_routing.rs");
    include!("local_commands.rs");
    include!("submission.rs");
    include!("resources.rs");
    include!("rendering.rs");
    include!("mcp.rs");
}
