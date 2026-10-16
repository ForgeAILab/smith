//! Installed-CLI drift canary helpers.

use super::*;

const CANARY_TIMEOUT: Duration = Duration::from_secs(180);

fn has_help_token(help: &str, flag: &str) -> bool {
    help.split(|character: char| !(character.is_ascii_alphanumeric() || "-_".contains(character)))
        .any(|token| token == flag)
}

fn require_help(help: &str, flags: &[&str], context: &str, scope: &str) {
    for flag in flags {
        assert!(
            has_help_token(help, flag),
            "{context}: {scope} no longer advertises flag/subcommand {flag}"
        );
    }
}

fn help_contract(cli: &Installed) {
    let help = cli.help(&["--help"]);
    match cli.kind {
        Kind::Claude => require_help(
            &help,
            &[
                "-p",
                "--output-format",
                "--verbose",
                "--allowedTools",
                "--permission-mode",
                "bypassPermissions",
                "--model",
                "--resume",
                "--mcp-config",
            ],
            &cli.context,
            "--help",
        ),
        Kind::Codex => {
            require_help(
                &help,
                &["exec", "-C", "--sandbox", "-m", "-c"],
                &cli.context,
                "--help",
            );
            let exec = cli.help(&["exec", "--help"]);
            require_help(
                &exec,
                &[
                    "--json",
                    "--skip-git-repo-check",
                    "-C",
                    "--sandbox",
                    "--dangerously-bypass-approvals-and-sandbox",
                    "-m",
                    "-c",
                    "resume",
                ],
                &cli.context,
                "exec --help",
            );
            let resume = cli.help(&["exec", "resume", "--help"]);
            for flag in ["-C", "--sandbox"] {
                assert!(
                    !has_help_token(&resume, flag),
                    "{}: exec resume --help now lists {flag}; Smith's flags-before-resume ordering workaround may no longer be needed; review it",
                    cli.context
                );
            }
        }
    }
}

pub(super) fn canary(kind: Kind) {
    let cli = Installed::discover(kind);
    help_contract(&cli);
    let directory =
        tempfile::tempdir().unwrap_or_else(|error| panic!("{}: project: {error}", cli.context));
    let server = stand_in(&cli.context);
    let args = counting_args(directory.path());
    let mut command = cli.command();
    command.current_dir(directory.path());
    match kind {
        Kind::Claude => {
            // Same argv order and environment as invocation.rs, substituting
            // a counting server for the relay and an impossible model.
            command.args([
                "-p",
                "Do not call tools. This model must not exist.",
                "--output-format",
                "stream-json",
                "--verbose",
                "--allowedTools",
                "mcp__smith",
                "--mcp-config",
                &json!({"mcpServers":{"smith":{"command":server,"args":args}}}).to_string(),
                "--model",
                INVALID_MODEL,
            ]);
            command
                .env("MCP_TOOL_TIMEOUT", "604800000")
                .env("MCP_TIMEOUT", "60000");
        }
        Kind::Codex => {
            command
                .args(["exec", "--json", "--skip-git-repo-check", "-C"])
                .arg(directory.path())
                .args(["--sandbox", "read-only", "-m", INVALID_MODEL]);
            // Unknown server keys must be errors, rather than ignored TOML.
            // Keep the same six -c values (and order) as invocation.rs.
            command.arg("--strict-config");
            for value in [
                format!("mcp_servers.smith.command={}", json!(server)),
                format!("mcp_servers.smith.args={args}"),
                "mcp_servers.smith.default_tools_approval_mode=\"approve\"".into(),
                "mcp_servers.smith.tool_timeout_sec=604800".into(),
                "mcp_servers.smith.required=true".into(),
                "mcp_servers.smith.startup_timeout_sec=60".into(),
            ] {
                command.args(["-c", &value]);
            }
            command.arg("Do not call tools. This model must not exist.");
        }
    }
    let context = format!(
        "{}: bridge invocation flags and mcp_servers.smith keys",
        cli.context
    );
    let output = capture(
        &mut command,
        None,
        Instant::now() + CANARY_TIMEOUT,
        &context,
    );
    let context = format!(
        "{context}; exit={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = json_lines(&output.stdout, &context);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{context}: impossible-model exit status changed; stdout={rows:?}"
    );
    let recorded = recorded_failed(kind, &context);
    match kind {
        Kind::Claude => claude_envelope(&rows, &recorded, &context),
        Kind::Codex => codex_envelope(&rows, &recorded, &context),
    }
    assert_handshake(directory.path(), &context);
    println!("{}: drift canary passed (no model completion)", cli.context);
}

fn claude_envelope(rows: &[Value], recorded: &[Value], context: &str) {
    let init = claude_init(rows, context);
    let reference = claude_init(recorded, context);
    fields_match(init, reference, CLAUDE_INIT_FIELDS, context);
    let servers = init["mcp_servers"]
        .as_array()
        .unwrap_or_else(|| panic!("{context}: system.init.mcp_servers is not an array"));
    let smith = servers
        .iter()
        .find(|server| server["name"] == "smith")
        .unwrap_or_else(|| {
            panic!("{context}: system.init.mcp_servers[].name=smith missing: {servers:?}")
        });
    let recorded_server = &reference["mcp_servers"][0];
    fields_match(smith, recorded_server, &["/name", "/status"], context);
    assert_eq!(
        smith["status"], recorded_server["status"],
        "{context}: system.init.mcp_servers[smith].status changed"
    );
    assert!(
        init["tools"]
            .as_array()
            .is_some_and(|tools| tools.contains(&json!("mcp__smith__count"))),
        "{context}: system.init.tools lacks mcp__smith__count: {}",
        init["tools"]
    );
    let result = event(rows, "result", context);
    fields_match(
        result,
        event(recorded, "result", context),
        CLAUDE_RESULT_FIELDS,
        context,
    );
    assert_eq!(
        result["session_id"], init["session_id"],
        "{context}: result.session_id changed within turn"
    );
    assert_eq!(
        result["total_cost_usd"].as_f64(),
        Some(0.0),
        "{context}: impossible model spent quota"
    );
    assert!(
        result["result"]
            .as_str()
            .is_some_and(|text| text.contains(INVALID_MODEL)),
        "{context}: result.result no longer reports the impossible model: {result}"
    );
    let start = rows.iter().position(|row| std::ptr::eq(row, init));
    let end = rows.iter().position(|row| std::ptr::eq(row, result));
    assert!(start < end, "{context}: system.init must precede result");
}

fn codex_envelope(rows: &[Value], recorded: &[Value], context: &str) {
    for (kind, fields) in [
        ("thread.started", CODEX_THREAD_FIELDS),
        ("error", CODEX_ERROR_FIELDS),
        ("turn.failed", CODEX_FAILED_FIELDS),
    ] {
        fields_match(
            event(rows, kind, context),
            event(recorded, kind, context),
            fields,
            context,
        );
    }
    let position = |kind| {
        rows.iter()
            .position(|row| row["type"] == kind)
            .unwrap_or_else(|| panic!("{context}: missing {kind}"))
    };
    assert!(
        position("thread.started") < position("error")
            && position("error") < position("turn.failed"),
        "{context}: thread.started -> error -> turn.failed ordering changed"
    );
    assert!(
        event(rows, "turn.failed", context)["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains(INVALID_MODEL)),
        "{context}: turn.failed.error.message no longer reports the impossible model"
    );
}
