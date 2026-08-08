// -- `/mcp` -------------------------------------------------------------------

/// A project that declares one server, plus a user root to resolve against.
fn mcp_project(declaration: &str) -> (tempfile::TempDir, tempfile::TempDir) {
    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    std::fs::create_dir_all(home.path().join(".smith")).expect("a user `.smith`");
    std::fs::create_dir_all(project.path().join(".smith")).expect("a project `.smith`");
    std::fs::write(
        project.path().join(".smith/config.toml"),
        format!("{LOCAL_COMMAND_CONFIG}\n{declaration}"),
    )
    .expect("a project config");
    (home, project)
}

fn mcp_context(
    home: &std::path::Path,
    project: &std::path::Path,
) -> std::sync::Arc<crate::mcp::McpContext> {
    let resolution = smith_config::resolve::resolve(
        &smith_config::resolve::ResolveRequest::new(project).with_home_dir(home),
    )
    .expect("a resolved project");
    crate::mcp::McpContext::start(
        &resolution.config,
        &resolution.layout.user_dir,
        project,
        None,
    )
    .expect("a planned context")
    .expect("a declared server")
}

#[tokio::test]
async fn mcp_command_never_renders_a_credential_value() {
    const TOKEN: &str = "ghp_this_value_must_never_reach_a_surface";

    let (home, project) = mcp_project(
        "[mcp.servers.github]\ncommand = \"npx\"\nargs = [\"-y\", \"server-github\"]\n\
         env = { GITHUB_TOKEN = \"keychain:smith/github\" }\n",
    );
    // The value lives where a credential value lives: behind the reference,
    // in the store. Nothing on this path may reach it, and nothing on this
    // path may print it if it did.
    let context = mcp_context(home.path(), project.path());

    let listed = context.render_list();
    assert!(listed.contains("github"), "{listed}");
    assert!(
        listed.contains("untrusted") && listed.contains("/mcp trust github"),
        "an untrusted server says what to do about it: {listed}"
    );
    assert!(
        listed.contains("GITHUB_TOKEN") && listed.contains("keychain:smith/github"),
        "the credential is named by its reference: {listed}"
    );
    assert!(!listed.contains(TOKEN), "{listed}");

    let confirmation = context.confirmation("github").expect("a confirmation");
    assert!(
        confirmation.contains("npx") && confirmation.contains("-y server-github"),
        "the exact invocation is shown: {confirmation}"
    );
    assert!(
        confirmation.contains("identity"),
        "the content identity is shown: {confirmation}"
    );
    assert!(confirmation.contains("GITHUB_TOKEN"), "{confirmation}");
    assert!(!confirmation.contains(TOKEN), "{confirmation}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_literal_environment_value_is_withheld_from_every_mcp_surface() {
    use std::os::unix::fs::PermissionsExt;

    const LITERAL: &str = "literal-value-must-not-render";

    let (home, project) = mcp_project("");
    let user_config = home.path().join(".smith/config.toml");
    std::fs::write(
        &user_config,
        format!(
            "[mcp.servers.docs]\ncommand = \"docs-mcp\"\nenv = {{ DOCS_TOKEN = \"{LITERAL}\" }}\n"
        ),
    )
    .expect("a user config");
    std::fs::set_permissions(&user_config, std::fs::Permissions::from_mode(0o600))
        .expect("an owner-only user config");

    let context = mcp_context(home.path(), project.path());
    let listed = context.render_list();
    let confirmation = context.confirmation("docs").expect("a confirmation");
    assert!(listed.contains("DOCS_TOKEN") && listed.contains("withheld"), "{listed}");
    assert!(!listed.contains(LITERAL), "{listed}");
    assert!(!confirmation.contains(LITERAL), "{confirmation}");
}

#[tokio::test]
async fn granting_trust_records_the_decision_and_leaves_the_session_running() {
    let (home, project) = mcp_project("[mcp.servers.github]\ncommand = \"npx\"\n");
    let context = mcp_context(home.path(), project.path());
    assert!(context.render_list().contains("untrusted"));

    let notice = context.trust("github").expect("a recorded decision");
    assert!(
        notice.contains("trusted") && notice.contains("next safe boundary"),
        "{notice}"
    );
    // The decision is persisted where the next run will look for it, and the
    // server is no longer waiting on the user.
    let trust = smith_config::trust::TrustStore::open(home.path().join(".smith"))
        .expect("the persisted store");
    let resolution = smith_config::resolve::resolve(
        &smith_config::resolve::ResolveRequest::new(project.path()).with_home_dir(home.path()),
    )
    .expect("a resolved project");
    let server = &resolution.config.mcp.servers["github"];
    assert_eq!(
        trust
            .status(project.path(), &smith_config::mcp::executable(server))
            .expect("a status"),
        smith_config::trust::TrustStatus::Trusted
    );

    let unknown = context.trust("nope").expect_err("an undeclared server");
    assert!(unknown.contains("not a declared MCP server"), "{unknown}");
}

#[test]
fn the_mcp_command_parses_its_only_two_forms() {
    assert_eq!(
        smith_tui::commands::parse("/mcp"),
        Ok(CommandAction::Mcp(smith_tui::McpAction::List))
    );
    assert_eq!(
        smith_tui::commands::parse("/mcp trust github"),
        Ok(CommandAction::Mcp(smith_tui::McpAction::Trust(
            "github".to_owned()
        )))
    );
    assert!(smith_tui::commands::parse("/mcp trust").is_err());
    assert!(smith_tui::commands::parse("/mcp nonsense").is_err());
}

#[tokio::test]
async fn a_project_declaring_no_server_gets_no_supervisor_and_no_trust_file() {
    let (home, project) = mcp_project("");
    let resolution = smith_config::resolve::resolve(
        &smith_config::resolve::ResolveRequest::new(project.path()).with_home_dir(home.path()),
    )
    .expect("a resolved project");
    assert!(resolution.config.mcp.servers.is_empty());

    let context = crate::mcp::McpContext::start(
        &resolution.config,
        &resolution.layout.user_dir,
        project.path(),
        None,
    )
    .expect("planning succeeds");
    assert!(
        context.is_none(),
        "a project with no `[mcp]` table must behave exactly as it did before"
    );
    assert!(
        !home.path().join(".smith/trust.json").exists(),
        "nothing about MCP touched user state"
    );
}

#[tokio::test]
async fn a_remote_server_is_listed_by_endpoint_with_its_credentials_named() {
    let (home, project) = mcp_project(
        "[mcp.servers.internal]\nurl = \"https://mcp.example.test/v1\"\n\
         credential = \"keychain:smith/internal\"\nheaders = { X-Tenant = \"acme\" }\n",
    );
    let context = mcp_context(home.path(), project.path());

    let listed = context.render_list();
    assert!(listed.contains("http"), "the transport is named: {listed}");
    assert!(
        listed.contains("header Authorization ← keychain:smith/internal"),
        "the bearer credential is named by the header it is sent under: {listed}"
    );
    assert!(
        listed.contains("header X-Tenant ← value withheld"),
        "a literal header value is withheld even though it is not a secret: {listed}"
    );

    let confirmation = context.confirmation("internal").expect("a confirmation");
    assert!(
        confirmation.contains("endpoint https://mcp.example.test/v1"),
        "the endpoint is what a remote server's trust is about: {confirmation}"
    );
    assert!(
        confirmation.contains("sends the credentials above to that endpoint"),
        "the prompt says what trusting it actually does: {confirmation}"
    );
    assert!(confirmation.contains("identity"), "{confirmation}");
}
