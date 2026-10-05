use super::*;

/// A project whose config is the base plus one `[mcp.servers.*]` block.
fn resolve_mcp_project(text: &str) -> Result<Resolution, ConfigError> {
    resolve_project(&format!("{BASE_PROJECT_CONFIG}\n{text}"))
}

#[test]
fn a_declared_server_resolves_with_its_transport_and_defaults() {
    let resolution = resolve_mcp_project(
        r#"
[mcp.servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "keychain:smith/github" }
"#,
    )
    .expect("a declared server");

    let server = &resolution.config.mcp.servers["github"];
    assert_eq!(server.name, "github");
    assert_eq!(server.transport.as_str(), "stdio");
    assert_eq!(
        server.transport.args(),
        ["-y", "@modelcontextprotocol/server-github"]
    );
    // Omitting `enabled` leaves the server on, and says so from the built-in
    // layer rather than from nowhere.
    assert!(server.enabled.value);
    assert_eq!(server.enabled.source.layer, Layer::BuiltIn);
    assert_eq!(
        server.env["GITHUB_TOKEN"].value.credential(),
        Some("keychain:smith/github"),
        "a reference stays a reference rather than being read as a literal"
    );
    assert_eq!(resolution.config.mcp.enabled().count(), 1);
}

#[test]
fn server_declaration_reports_its_winning_layer() {
    let fixture = Fixture::new();
    fixture.write_user(
        r#"
[mcp.servers.github]
command = "npx"
args = ["-y", "server-github"]
"#,
    );
    fixture.write_project(&format!(
        "{BASE_PROJECT_CONFIG}\n[mcp.servers.github]\ncommand = \"./scripts/github-mcp\"\n"
    ));

    let resolution = resolve(&fixture.request()).expect("a layered declaration");
    let server = &resolution.config.mcp.servers["github"];
    let smith_config::resolve::ResolvedMcpTransport::Stdio { command, args } = &server.transport
    else {
        panic!("a stdio server");
    };

    assert_eq!(command.value, "./scripts/github-mcp");
    assert_eq!(command.source.layer, Layer::ProjectFile);
    // The layer that lost still supplies what the winner did not say.
    assert_eq!(
        args.as_ref().expect("inherited arguments").source.layer,
        Layer::UserFile
    );

    let explanation = resolution
        .provenance
        .explain("mcp.servers.github.command")
        .expect("an explained declaration");
    assert_eq!(explanation.source.layer, Layer::ProjectFile);
    assert_eq!(
        explanation
            .overridden
            .iter()
            .map(|entry| entry.source.layer)
            .collect::<Vec<_>>(),
        vec![Layer::UserFile],
        "the overridden layer stays visible as the explanation"
    );
}

#[test]
fn literal_secret_in_project_config_is_rejected() {
    const TOKEN: &str = "ghp_this_value_must_never_be_reproduced";

    let error = resolve_mcp_project(&format!(
        "[mcp.servers.github]\ncommand = \"npx\"\nenv = {{ GITHUB_TOKEN = \"{TOKEN}\" }}\n"
    ))
    .expect_err("a token written into a repository file");

    let ConfigError::PlaintextSecret { source, message } = &error else {
        panic!("expected a plaintext-secret diagnostic, got {error}");
    };
    assert_eq!(source.key, "mcp.servers.github.env.GITHUB_TOKEN");
    assert!(
        !format!("{error}").contains(TOKEN),
        "the diagnostic must name the key without reproducing the value: {message}"
    );
}

#[cfg(unix)]
#[test]
fn a_literal_in_owner_only_user_config_is_accepted_and_kept_redaction_safe() {
    const TOKEN: &str = "ghp_owner_only_value";

    let fixture = Fixture::new();
    fixture.write_private_user(&format!(
        "[mcp.servers.github]\ncommand = \"npx\"\nenv = {{ GITHUB_TOKEN = \"{TOKEN}\" }}\n"
    ));
    fixture.write_project(BASE_PROJECT_CONFIG);

    let resolution = resolve(&fixture.request()).expect("an owner-only literal");
    let value = &resolution.config.mcp.servers["github"].env["GITHUB_TOKEN"];
    assert!(
        value.value.credential().is_none(),
        "a literal is not a reference"
    );
    // It reaches the child process, and nothing else: neither the explanation
    // ledger nor a debug rendering may reproduce it.
    let explained = resolution
        .provenance
        .explain("mcp.servers.github.env.GITHUB_TOKEN")
        .expect("an explained variable");
    assert!(!format!("{}", explained.value).contains(TOKEN));
    assert!(!format!("{value:?}").contains(TOKEN));
}

#[test]
fn an_ordinary_project_environment_value_is_not_mistaken_for_a_secret() {
    let resolution = resolve_mcp_project(
        "[mcp.servers.docs]\ncommand = \"docs-mcp\"\nenv = { DOCS_ROOT = \"./docs\" }\n",
    )
    .expect("a non-credential variable");
    assert!(
        resolution.config.mcp.servers["docs"]
            .env
            .contains_key("DOCS_ROOT"),
        "a variable whose name claims no credential stays usable in a project file"
    );
}

#[test]
fn a_disabled_server_is_visible_but_not_used() {
    let resolution =
        resolve_mcp_project("[mcp.servers.github]\ncommand = \"npx\"\nenabled = false\n")
            .expect("a disabled server");

    let server = &resolution.config.mcp.servers["github"];
    assert!(!server.enabled.value);
    assert_eq!(server.enabled.source.layer, Layer::ProjectFile);
    assert_eq!(resolution.config.mcp.enabled().count(), 0);
}

#[test]
fn a_declaration_must_name_exactly_one_transport() {
    let both = resolve_mcp_project(
        "[mcp.servers.github]\ncommand = \"npx\"\nurl = \"https://mcp.example.test/v1\"\n",
    )
    .expect_err("two transports");
    assert!(matches!(both, ConfigError::InvalidValue { .. }));
    assert!(format!("{both}").contains("two transports"));

    let neither =
        resolve_mcp_project("[mcp.servers.github]\nargs = [\"-y\"]\n").expect_err("no transport");
    let ConfigError::MissingSetting { key, .. } = &neither else {
        panic!("expected a missing-setting diagnostic, got {neither}");
    };
    assert_eq!(key, "mcp.servers.github.command");
}

#[test]
fn a_url_naming_no_supported_transport_is_rejected() {
    let error = resolve_mcp_project("[mcp.servers.remote]\nurl = \"ftp://mcp.example.test/v1\"\n")
        .expect_err("an unknown transport");
    assert!(matches!(error, ConfigError::InvalidValue { .. }));
    assert!(format!("{error}").contains("http"));
}

#[test]
fn a_server_name_no_provider_would_accept_is_rejected() {
    for name in ["git.hub", "a__b", ""] {
        let error = resolve_mcp_project(&format!("[mcp.servers.\"{name}\"]\ncommand = \"npx\"\n"))
            .unwrap_err();
        assert!(
            matches!(error, ConfigError::InvalidValue { .. }),
            "`{name}` should be refused, got {error}"
        );
    }

    let long = "s".repeat(49);
    let error = resolve_mcp_project(&format!("[mcp.servers.{long}]\ncommand = \"npx\"\n"))
        .expect_err("an over-long name");
    assert!(format!("{error}").contains("48 characters"));
}

#[test]
fn a_remote_server_resolves_its_endpoint_credential_and_headers() {
    let resolution = resolve_mcp_project(
        "[mcp.servers.remote]\nurl = \"https://mcp.example.test/v1\"\n\
         credential = \"keychain:smith/remote\"\nheaders = { X-Tenant = \"acme\" }\n",
    )
    .expect("a remote server");

    let server = &resolution.config.mcp.servers["remote"];
    assert_eq!(server.transport.as_str(), "http");
    let smith_config::resolve::ResolvedMcpTransport::StreamableHttp {
        url,
        credential,
        headers,
    } = &server.transport
    else {
        panic!("a remote server");
    };
    assert_eq!(url.value, "https://mcp.example.test/v1");
    assert_eq!(
        credential.as_ref().expect("a bearer credential").value,
        "keychain:smith/remote"
    );
    assert_eq!(headers["X-Tenant"].value.credential(), None);
    // The credential is sent as a header, so it is part of what the user has
    // to approve — by name, never by value.
    assert_eq!(
        server.transport.header_names(),
        vec!["Authorization".to_owned(), "X-Tenant".to_owned()]
    );
}

#[test]
fn an_authorization_header_written_in_plain_text_is_refused() {
    const TOKEN: &str = "Bearer sk-this-must-never-be-reproduced";

    let error = resolve_mcp_project(&format!(
        "[mcp.servers.remote]\nurl = \"https://mcp.example.test/v1\"\n\
         headers = {{ Authorization = \"{TOKEN}\" }}\n"
    ))
    .expect_err("a token written into a repository file");

    let ConfigError::PlaintextSecret { source, .. } = &error else {
        panic!("expected a plaintext-secret diagnostic, got {error}");
    };
    assert_eq!(source.key, "mcp.servers.remote.headers.Authorization");
    assert!(!format!("{error}").contains("sk-this-must"), "{error}");
}

#[test]
fn an_option_the_chosen_transport_cannot_use_is_refused() {
    // Silently ignoring a declared credential is how a user comes to believe a
    // local server is authenticated when nothing was ever sent.
    let local = resolve_mcp_project(
        "[mcp.servers.docs]\ncommand = \"docs-mcp\"\ncredential = \"keychain:smith/docs\"\n",
    )
    .expect_err("a credential a local server cannot send");
    assert!(
        format!("{local}").contains("no use for `credential`"),
        "{local}"
    );

    let remote = resolve_mcp_project(
        "[mcp.servers.remote]\nurl = \"https://mcp.example.test/v1\"\nargs = [\"-y\"]\n",
    )
    .expect_err("arguments a remote server cannot take");
    assert!(
        format!("{remote}").contains("no use for `args`"),
        "{remote}"
    );

    let remote_env = resolve_mcp_project(
        "[mcp.servers.remote]\nurl = \"https://mcp.example.test/v1\"\nenv = { A = \"b\" }\n",
    )
    .expect_err("an environment a remote server cannot receive");
    assert!(
        format!("{remote_env}").contains("no use for `env`"),
        "{remote_env}"
    );
}
