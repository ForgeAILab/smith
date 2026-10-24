use super::*;

#[test]
fn delegated_wait_parks_and_automatically_wakes_in_a_real_terminal() {
    let fixture = Fixture::new();
    let release = fixture.project.path().join("release-child");
    let (script, record) = support::script_files(
        fixture.project.path(),
        &serde_json::json!({
            "initial":[
                {"call":{"tool":"agent","arguments":{"action":"spawn","profile":"worker","task":"held terminal child"}}},
                {"call":{"tool":"agent","arguments":{"action":"wait","child_id":{"$last":"/spawned"},"timeout_ms":2000}}},
                {"say":"PARENT_PARKED"}
            ],
        "resumed":[{"list_tools":true},{"say":"AUTOMATIC_WAKE"}],
        "by_prompt":{"held terminal child":[{"list_tools":true},{"wait_file":release},{"say":"CHILD_FINISHED"}]}
        }),
    );
    let wrapper = fixture.project.path().join("harness-fixture.sh");
    std::fs::write(&wrapper, format!(
        "#!/bin/sh\nexport CLI_STAND_IN_SCRIPT='{}'\nexport CLI_STAND_IN_RECORD='{}'\nexport CLI_STAND_IN_WAIT_TIMEOUT_MS=10000\nexec '{}' \"$@\"\n",
        script.display(), record.display(), support::stand_in_path().display(),
    )).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let _cleanup = support::BridgeCleanup::from_record(record.clone());
    fixture.configure_user(&format!(
        r#"
default_profile = "dev"
[profiles.dev]
provider = "local"
model = "cli/claude-code/sonnet"
posture = "build"
delegation = true
[profiles.worker]
provider = "local"
model = "cli/claude-code/sonnet"
posture = "review"
use = ["child"]
[providers.local]
kind = "fake"
[harness.claude-code]
executable = {}
[persistence]
checkpoint_key = "5151515151515151515151515151515151515151515151515151515151515151"
[approval]
mode = "allow-all"
"#,
        serde_json::json!(wrapper.to_str().unwrap())
    ));
    std::fs::set_permissions(
        fixture.home.path().join(".smith/config.toml"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let interaction = format!(
        r#"
expect {{
    -exact "Ask Smith to do anything" {{}}
    timeout {{ exit 124 }}
    eof {{ exit 125 }}
}}
send -- "wait check\r"
expect {{
    -re {{nitor(\e\[[0-9;]*H| )[0-9]}} {{}}
    timeout {{ exit 126 }}
    eof {{ exit 125 }}
}}
expect {{
    -exact "PARENT_PARKED" {{}}
    timeout {{ exit 127 }}
    eof {{ exit 125 }}
}}
expect {{
    -exact "delegation.wait-timeout" {{}}
    timeout {{ exit 128 }}
    eof {{ exit 125 }}
}}
expect {{
    -exact "AUTOMATIC_WAKE" {{}}
    timeout {{ exit 129 }}
    eof {{ exit 125 }}
}}
set gate [open {{{}}} w]
puts $gate "finish"
close $gate
expect {{
    -exact "CHILD_FINISHED" {{}}
    timeout {{ exit 130 }}
    eof {{ exit 125 }}
}}
send -- "/quit\r"
expect {{
    -exact "TERMINAL_RESTORED" {{}}
    timeout {{ exit 131 }}
    eof {{ exit 125 }}
}}
"#,
        release.display()
    );
    let Some(output) = fixture.run_expect("--no-color --no-motion", &interaction) else {
        return;
    };
    assert!(
        output.status.success(),
        "status: {}\nscreen: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        (screen.contains("nitor ") || screen.contains("nitor\u{1b}["))
            && screen.contains("delegation.wait-timeout")
            && screen.contains("AUTOMATIC_WAKE")
    );
}
