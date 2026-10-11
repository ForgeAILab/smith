//! Withheld installed namespaces use the ordinary file-command refusal path.

use std::sync::Arc;
use std::time::Duration;

use agent_runtime::provider::fake::FakeProvider;
use agent_runtime_core::content::{Role, UserInput};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use smith_config::inventory::local_inventory_with_catalog;
use smith_config::resolve::{ResolveRequest, resolve};
use smith_runtime::factory::{AVAILABLE_ADAPTER_KINDS, HostSurface};
use smith_runtime::host::HostSessionRequest;
use smith_tui::{App, Block};

use super::{Fixture, write};
use crate::local_command::file_commands::CommandContext;
use crate::tui_driver::{InteractiveExit, InteractiveResources, run_scripted_tui};

const INVOCATION: &str = "/deploy-tools:status api";
const CHANGED: &str = "`/deploy-tools:status` cannot run: plugin `deploy-tools` content changed; run `smith plugin update deploy-tools` to review and confirm it again";
const OFF: &str = "`/deploy-tools:status` cannot run: plugin `deploy-tools` is off; run `/plugins deploy-tools on` or set `plugins.deploy-tools.enabled = true`";
const UNTRUSTED: &str = "`/deploy-tools:status` cannot run: plugin `deploy-tools` bundle trust is Untrusted; run `smith plugin update deploy-tools` to review and confirm it";

fn withhold(fixture: &Fixture, state: &str) {
    match state {
        "changed" => write(
            &fixture.installed.install_directory,
            "server.js",
            "local edits",
        ),
        "off" => write(
            &fixture.user,
            "config.toml",
            "[persistence]\nenabled = false\n[plugins.deploy-tools]\nenabled = false\n",
        ),
        "failed" => write(
            &fixture.installed.install_directory,
            ".claude-plugin/plugin.json",
            "broken JSON",
        ),
        "untrusted" => {}
        _ => panic!("unknown fixture state"),
    }
}

// Mirrors main's ordering: preflight must succeed before starting a host or turn.
async fn headless_run(
    fixture: &Fixture,
    prompt: &str,
    provider: Arc<FakeProvider>,
) -> anyhow::Result<()> {
    let config = fixture.config(None);
    let prompt = crate::headless::prepare_prompt_with_config(
        prompt,
        &fixture.user,
        &fixture.project,
        &config,
    )?;
    let mut request = fixture.request(config);
    request.provider = Some(provider);
    let host = Box::pin(smith_runtime::host::start(HostSessionRequest::new(
        request,
        &fixture.project,
    )))
    .await?;
    host.session().run(UserInput::text(prompt)).await?;
    Box::pin(host.shutdown()).await?;
    Ok(())
}

async fn assert_headless_refusal(state: &str, expected: Option<&str>) {
    let fixture = Fixture::new(state != "untrusted");
    withhold(&fixture, state);
    let provider = Arc::new(FakeProvider::text_reply("must not run"));
    let error = headless_run(&fixture, INVOCATION, provider.clone())
        .await
        .expect_err("installed withheld plugin must fail closed");
    let mut stderr = Vec::new();
    assert_eq!(
        crate::report_failure(&error, &mut stderr),
        std::process::ExitCode::FAILURE
    );
    assert!(provider.requests().is_empty());
    let diagnostic = String::from_utf8(stderr).unwrap();
    if let Some(expected) = expected {
        assert_eq!(diagnostic, format!("smith: {expected}\n"));
    } else {
        assert!(diagnostic.starts_with(
            "smith: `/deploy-tools:status` cannot run: plugin `deploy-tools` failed: "
        ));
        assert!(diagnostic.contains("run `smith plugin update deploy-tools`"));
    }
}

#[tokio::test]
async fn headless_changed_plugin_command_fails_closed_without_provider_request() {
    assert_headless_refusal("changed", Some(CHANGED)).await;
}

#[tokio::test]
async fn headless_off_plugin_command_fails_closed_without_provider_request() {
    assert_headless_refusal("off", Some(OFF)).await;
}

#[tokio::test]
async fn headless_untrusted_and_failed_plugin_commands_fail_closed() {
    assert_headless_refusal("untrusted", Some(UNTRUSTED)).await;
    assert_headless_refusal("failed", None).await;
}

#[tokio::test]
async fn headless_unknown_plugin_prefix_and_literal_escape_keep_provider_text() {
    for (prompt, expected) in [
        ("/unknown:status api\nnext  ", "/unknown:status api\nnext  "),
        ("//deploy-tools:status api", "/deploy-tools:status api"),
    ] {
        let fixture = Fixture::new(true);
        withhold(&fixture, "changed");
        let provider = Arc::new(FakeProvider::text_reply("done"));
        headless_run(&fixture, prompt, provider.clone())
            .await
            .unwrap();
        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        let user = requests[0]
            .messages
            .iter()
            .filter(|message| message.role == Role::User)
            .map(|message| message.joined_text())
            .collect::<Vec<_>>();
        assert_eq!(user, vec![expected]);
    }
}

#[tokio::test]
async fn interactive_withheld_plugin_command_is_a_local_refusal_without_submission() {
    for state in ["changed", "off", "untrusted", "failed"] {
        let fixture = Fixture::new(state != "untrusted");
        withhold(&fixture, state);
        let config = fixture.config(None);
        let expected = crate::headless::prepare_prompt_with_config(
            INVOCATION,
            &fixture.user,
            &fixture.project,
            &config,
        )
        .unwrap_err()
        .to_string();
        let commands = Arc::new(
            CommandContext::discover(&fixture.user, &fixture.project)
                .unwrap()
                .with_plugins(&config)
                .unwrap(),
        );
        assert!(commands.catalog().runnable().next().is_none());
        assert!(
            smith_client::commands::matches_with("/deploy-tools:", &commands.catalog()).is_empty()
        );
        let resolution = resolve(
            &ResolveRequest::new(&fixture.project)
                .with_home_dir(&fixture.home)
                .with_known_modules(crate::modules::known_modules()),
        )
        .unwrap();
        let catalog = Arc::new(
            serde_json::from_str::<smith_config::catalog::CatalogSnapshot>(
                smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED,
            )
            .unwrap(),
        );
        let inventory =
            local_inventory_with_catalog(&resolution, AVAILABLE_ADAPTER_KINDS, Some(&catalog))
                .unwrap();
        let (_, skills) = crate::skills::SkillContext::compose(
            smith_runtime::skills::SmithSkillSources::new(),
            &fixture.user,
            &fixture.project,
        )
        .unwrap();
        let provider = Arc::new(FakeProvider::text_reply("must not run"));
        let mut request = fixture.request(config.clone());
        request.surface = HostSurface::Terminal;
        request.provider = Some(provider.clone());
        let host = Box::pin(smith_runtime::host::start(HostSessionRequest::new(
            request,
            &fixture.project,
        )))
        .await
        .unwrap();
        host.set_goal_continuation_enabled(false);
        let resources = InteractiveResources {
            inventory,
            agents: config.agent,
            catalog,
            sessions: Vec::new(),
            credential_pool: None,
            mcp: None,
            skills: Arc::new(skills),
            commands: commands.clone(),
            capability_denials: Vec::new(),
        };
        let mut app = App::new("example-model", "project");
        app.set_command_catalog(commands.catalog());
        app.composer.replace(INVOCATION);
        let mut keys = Box::pin(futures_util::stream::iter([
            Ok(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))),
            Ok(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            ))),
            Ok(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            ))),
        ]));
        let (exit, app) = tokio::time::timeout(
            Duration::from_secs(5),
            Box::pin(run_scripted_tui(
                &host,
                &fixture.project,
                &resources,
                app,
                &mut keys,
                false,
            )),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(exit, InteractiveExit::Quit(..)));
        assert!(provider.requests().is_empty());
        assert!(host.session().history().is_empty());
        assert!(!app.has_pending_input());
        assert!(
            app.transcript
                .blocks()
                .iter()
                .any(|block| matches!(block, Block::Error { message } if message == &expected))
        );
        assert!(
            app.transcript
                .blocks()
                .iter()
                .all(|block| !matches!(block, Block::User { .. }))
        );
        Box::pin(host.shutdown()).await.unwrap();
    }
}

#[test]
fn interactive_plugin_supporting_file_changed_since_discovery_is_refused() {
    let fixture = Fixture::new(true);
    let config = fixture.config(None);
    let commands = CommandContext::discover(&fixture.user, &fixture.project)
        .unwrap()
        .with_plugins(&config)
        .unwrap();
    let mut app = App::new("example-model", "project");
    app.set_command_catalog(commands.catalog());
    withhold(&fixture, "changed");
    app.composer.replace(INVOCATION);
    let Some(smith_tui::app::Action::FileCommand {
        typed,
        name,
        arguments,
        ..
    }) = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("host file-command action");
    };
    assert_eq!(
        commands
            .prepare(&app, typed, &name, &arguments)
            .unwrap_err(),
        CHANGED
    );
    assert!(!app.has_pending_input());
}

#[test]
fn interactive_unknown_plugin_prefix_keeps_existing_parse_error_and_escape_route() {
    let fixture = Fixture::new(true);
    withhold(&fixture, "changed");
    let catalog = fixture.catalog(&fixture.config(None));
    let input = "/unknown:status api";
    assert_eq!(
        smith_client::commands::parse_with(input, &catalog).unwrap_err(),
        smith_client::commands::parse(input).unwrap_err()
    );
    assert!(!smith_client::commands::has_exact_name_with(
        input, &catalog
    ));
    let mut app = App::new("example-model", "project");
    app.set_command_catalog(catalog);
    app.composer.replace("//deploy-tools:status api");
    let Some(smith_tui::app::Action::Submit { submission, .. }) =
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("escaped input remains an ordinary submission");
    };
    assert_eq!(submission.committed_text(), INVOCATION);
}
