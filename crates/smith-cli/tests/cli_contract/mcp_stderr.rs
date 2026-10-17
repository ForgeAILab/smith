//! A stdio MCP server's stderr never reaches Smith's terminal.
//!
//! Inherited, `npx` notices were painted over the TUI's composer. These run
//! the real binary, its real watchdog, and a real server process: Smith's own
//! stderr is the terminal, so it must stay exactly empty.

use std::time::Duration;

use serde_json::Value;

use super::Fixture;
use super::support::{Deadline, run_command};

/// Answers the handshake and the tool listing, chatters on stderr the way
/// `npx` does, and serves until its input closes.
const CHATTY: &str = r#"
echo 'npm notice New major version of npm available! 10.0.0 -> 11.0.0' >&2
echo 'npm notice Run `npm install -g npm@11` to update!' >&2
read line
version=$(printf '%s' "$line" | sed -n 's/.*"protocolVersion":"\([^"]*\)".*/\1/p')
id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"%s","capabilities":{"tools":{}},"serverInfo":{"name":"chatty","version":"1"}}}\n' "$id" "$version"
read _
read line
id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[]}}\n' "$id"
echo 'npm notice still chatting after connecting' >&2
while read _; do :; done
"#;

/// Explains on stderr why it cannot start, including the token it was given.
const BROKEN: &str = r#"
echo 'npm notice New major version of npm available!' >&2
echo "auth rejected for token $BROKEN_TOKEN" >&2
echo 'Error: Cannot find module /srv/broken/server.js' >&2
exit 1
"#;

/// A credential the server is given, resolved from Smith's own environment.
const SECRET: &str = "resolved-secret-token-value";
const SECRET_VARIABLE: &str = "MCP_STDERR_TEST_TOKEN";

fn declare(fixture: &Fixture, name: &str, script: &str) {
    let config = fixture.project.path().join(".smith/config.toml");
    let mut text = std::fs::read_to_string(&config).expect("project config");
    text.push_str(&format!(
        "\n[mcp.servers.{name}]\ncommand = \"/bin/sh\"\nargs = [\"-c\", {}]\n\
         env = {{ PATH = \"/usr/bin:/bin\", BROKEN_TOKEN = \"env:{SECRET_VARIABLE}\" }}\n",
        serde_json::to_string(script).expect("a TOML basic string")
    ));
    std::fs::write(&config, text).expect("project config");
    let resolved = smith_config::resolve::resolve(
        &smith_config::resolve::ResolveRequest::new(fixture.project.path())
            .with_home_dir(fixture.home.path()),
    )
    .expect("MCP configuration")
    .config;
    smith_config::trust::TrustStore::open(fixture.home.path().join(".smith"))
        .expect("trust store")
        .record(
            fixture.project.path(),
            &smith_config::mcp::executable(&resolved.mcp.servers[name]),
            smith_config::trust::TrustDecision::Allow,
        )
        .expect("trusted server");
}

fn headless(fixture: &Fixture) -> (std::process::Output, Value) {
    let output = run_command(
        fixture
            .command()
            .env("SMITH_LOG", "info")
            .env(SECRET_VARIABLE, SECRET)
            .args(["-p", "hello", "--output-format", "json"]),
        Deadline::after(Duration::from_secs(60)),
    );
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result = serde_json::from_slice(&output.stdout).expect("one JSON envelope on stdout");
    (output, result)
}

fn session_log(fixture: &Fixture, result: &Value) -> String {
    let session = result["session_id"].as_str().expect("a session id");
    std::fs::read_to_string(
        fixture
            .home
            .path()
            .join(".smith/logs")
            .join(format!("{session}.log")),
    )
    .expect("the session log")
}

#[test]
fn a_chatty_server_never_writes_to_smith_stderr_and_its_lines_reach_the_log() {
    let fixture = Fixture::new();
    declare(&fixture, "chatty", CHATTY);
    let (output, result) = headless(&fixture);

    assert!(
        output.stderr.is_empty(),
        "server stderr leaked to the terminal: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(result["status"], "ok");
    // An empty list is omitted from the envelope.
    assert!(
        result
            .get("diagnostics")
            .is_none_or(|diagnostics| diagnostics
                .as_array()
                .expect("diagnostics")
                .iter()
                .all(|line| !line.as_str().unwrap_or_default().contains("chatty"))),
        "healthy chatter is not a diagnostic: {result}"
    );
    // Captured before the session's log file existed, yet still logged.
    let log = session_log(&fixture, &result);
    assert!(
        log.contains("npm notice New major version of npm available!"),
        "{log}"
    );
}

#[test]
fn a_failing_server_reports_its_stderr_in_diagnostics_never_on_the_terminal() {
    let fixture = Fixture::new();
    declare(&fixture, "broken", BROKEN);
    let (output, result) = headless(&fixture);

    assert!(
        output.stderr.is_empty(),
        "server stderr leaked to the terminal: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostics = result["diagnostics"].as_array().expect("diagnostics");
    let failure = diagnostics
        .iter()
        .filter_map(Value::as_str)
        .find(|line| line.starts_with("`broken` failed"))
        .unwrap_or_else(|| panic!("no failure diagnostic: {diagnostics:?}"));
    assert!(
        failure.contains("Error: Cannot find module /srv/broken/server.js"),
        "{failure}"
    );
    assert!(
        failure.contains("auth rejected for token [redacted]"),
        "{failure}"
    );
    assert!(!failure.contains(SECRET), "{failure}");

    let log = session_log(&fixture, &result);
    assert!(log.contains("MCP server failed"), "{log}");
    assert!(
        log.contains("Error: Cannot find module /srv/broken/server.js"),
        "{log}"
    );
    assert!(!log.contains(SECRET), "{log}");
}
