//! Feed the complete stream of a real headless harness run to the TUI reducer.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::{Value, json};
use smith_runtime::client::SmithEvent;
use smith_tui::app::App;
use smith_tui::theme::Theme;
use smith_tui::transcript::{Block, ToolStatus};

#[path = "../../smith-cli/tests/cli_contract/bridge_cleanup.rs"]
mod bridge_cleanup;

struct SmithGroup(u32);

impl Drop for SmithGroup {
    fn drop(&mut self) {
        let _ = Command::new("/bin/kill")
            .args(["-KILL", "--", &format!("-{}", self.0)])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[test]
fn live_headless_harness_stream_and_tui_have_the_same_tools_text_and_outcome() {
    // The orchestrator builds these workspace binaries before this test. The
    // explicit override also supports a separate target directory for Smith.
    let smith = std::env::var_os("CLI_SURFACE_SMITH_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .expect("test executable")
                .parent()
                .expect("deps directory")
                .parent()
                .expect("target profile")
                .join("smith")
        });
    let stand_in = smith
        .parent()
        .expect("Smith directory")
        .join("examples/cli_stand_in");
    assert!(
        smith.is_file(),
        "build smith-cli's smith binary before this test: {}",
        smith.display()
    );
    assert!(
        stand_in.is_file(),
        "build smith-cli's cli_stand_in example before this test: {}",
        stand_in.display()
    );
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let config_dir = home.path().join(".smith");
    std::fs::create_dir(&config_dir).expect("user config directory");
    let config_path = config_dir.join("config.toml");
    std::fs::write(
        &config_path,
        format!(
            r#"
default_profile = "dev"
[profiles.dev]
provider = "local"
model = "cli/claude-code/sonnet"
posture = "build"
use = ["main"]
delegation = true
[providers.local]
kind = "fake"
[harness.claude-code]
executable = {executable}
"#,
            executable = json!(stand_in)
        ),
    )
    .expect("user config");
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))
        .expect("owner-only config");
    let script = project.path().join("script.json");
    let record = project.path().join("record.json");
    // Declared before the process teardown so panic stops Smith first.
    let cleanup = bridge_cleanup::BridgeCleanup::from_record(record.clone());
    std::fs::write(
        &script,
        include_str!("fixtures/bridge-surfaces-script.json"),
    )
    .expect("shared script");
    let child = Command::new(smith)
        .args([
            "-p",
            "bridge surfaces",
            "--approval",
            "allow-all",
            "--output-format",
            "stream-json",
        ])
        .current_dir(project.path())
        .env_clear()
        .env("HOME", home.path())
        .env("CLI_STAND_IN_SCRIPT", script)
        .env("CLI_STAND_IN_RECORD", &record)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Smith process");
    let _process_group = SmithGroup(child.id());
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    let output = match receiver.recv_timeout(Duration::from_secs(30)) {
        Ok(output) => output.expect("Smith output"),
        Err(error) => panic!("Smith stand-in deadline: {error}"),
    };
    cleanup.capture();
    assert!(output.status.success(), "{output:?}");
    let lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("stream JSON"))
        .collect::<Vec<_>>();
    let results = lines
        .iter()
        .filter(|line| line["type"] == "result")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    let result = results[0];
    assert_eq!(result["status"], "ok");
    let events = lines
        .iter()
        .filter(|line| line["type"] == "runtime_event")
        .map(|line| {
            serde_json::from_value::<SmithEvent>(line["event"].clone())
                .expect("canonical runtime event")
        })
        .collect::<Vec<_>>();
    assert!(!events.is_empty());
    let mut app = App::new("cli/claude-code/sonnet", project.path().to_string_lossy());
    for event in &events {
        app.apply(event);
    }
    let tools = app
        .transcript
        .blocks()
        .iter()
        .filter_map(|block| {
            if let Block::Tool {
                call_id,
                name,
                status,
                enrichment,
                ..
            } = block
            {
                Some((
                    call_id.as_str(),
                    name.as_str(),
                    *status,
                    enrichment.as_slice(),
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(tools.len(), 2, "{:?}", app.transcript.blocks());
    assert_eq!(tools[0].1, "Bash");
    assert_eq!(tools[0].2, ToolStatus::Ok);
    assert_eq!(tools[0].3, &["agent"]);
    assert_eq!(tools[1].1, "agent");
    assert_eq!(tools[1].2, ToolStatus::Ok);
    let smith_call = events.iter().find(|event| matches!(
        &event.payload, smith_runtime::client::SmithEventKind::ToolCallRequested { name, .. } if name == "agent"
    )).expect("headless Smith tool request");
    if let smith_runtime::client::SmithEventKind::ToolCallRequested { call, .. } =
        &smith_call.payload
    {
        assert_eq!(tools[1].0, call.as_str());
    }
    let cli_call = events.iter().find(|event| matches!(
        &event.payload, smith_runtime::client::SmithEventKind::ExternalToolInvoked { name, .. } if name == "Bash"
    )).expect("headless CLI tool request");
    if let smith_runtime::client::SmithEventKind::ExternalToolInvoked { id, .. } = &cli_call.payload
    {
        assert_eq!(tools[0].0, id);
    }
    assert!(tools[1].3.is_empty());
    let texts = app
        .transcript
        .blocks()
        .iter()
        .filter_map(|block| match block {
            Block::Assistant { text, open: false } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(texts, ["before", "after"]);
    assert_eq!(result["output"], *texts.last().expect("final answer"));
    assert!(!app.is_busy());
    assert!(
        app.turn_summary
            .as_deref()
            .is_some_and(|summary| summary.starts_with("Worked"))
    );
    let blocks = app
        .transcript
        .blocks()
        .iter()
        .filter(|block| matches!(block, Block::Assistant { .. } | Block::Tool { .. }))
        .collect::<Vec<_>>();
    assert!(matches!(&blocks[0], Block::Assistant { text, .. } if text == "before"));
    assert!(matches!(&blocks[1], Block::Tool { name, .. } if name == "Bash"));
    assert!(matches!(&blocks[2], Block::Tool { name, .. } if name == "agent"));
    assert!(matches!(&blocks[3], Block::Assistant { text, .. } if text == "after"));
    let mut terminal = Terminal::new(TestBackend::new(100, 32)).expect("test terminal");
    terminal
        .draw(|frame| smith_tui::draw(frame, &app, Theme::new().without_color()))
        .expect("TUI frame");
    let buffer = terminal.backend().buffer();
    let screen = (0..32)
        .map(|y| {
            (0..100)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(screen.contains("cli/claude-code/sonnet"), "{screen}");
    assert!(!screen.contains("mcp__smith__agent"), "{screen}");
}
