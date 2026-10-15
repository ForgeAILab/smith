use super::*;

#[tokio::test]
async fn untrusted_mcp_server_reports_interactive_trust_in_every_output_format() {
    for format in [
        OutputFormat::Text,
        OutputFormat::Json,
        OutputFormat::StreamJson,
    ] {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("config directory");
        let marker = project.path().join("mcp-started");
        let args = serde_json::json!(["-c", "touch \"$1\"", "mcp-test", marker]);
        std::fs::write(
            config_dir.join("config.toml"),
            format!(
                r#"
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
[mcp.servers.docs]
command = "/bin/sh"
args = {args}
"#,
            ),
        )
        .expect("config");
        let resolution = resolve(
            &ResolveRequest::new(project.path())
                .with_known_modules(crate::modules::known_modules())
                .with_home_dir(home.path()),
        )
        .expect("resolution");
        let context = crate::mcp::McpContext::start(
            &resolution.config,
            &resolution.layout.user_dir,
            project.path(),
            None,
        )
        .expect("MCP context")
        .expect("declared server");
        let notices = context.trust_notices();
        let approval = Arc::new(HeadlessApproval::new());
        let request = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("workspace"),
            )),
            provider: Some(Arc::new(FakeProvider::text_reply("done"))),
            approval: Some(approval.clone()),
            mcp: Some(context.supervisor()),
            ..RuntimeRequest::new(resolution.config, HostSurface::Headless)
        };
        let host = Box::pin(smith_runtime::host::start(host_request(
            request,
            project.path(),
        )))
        .await
        .expect("host");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let outcome = tokio::time::timeout(
            HEADLESS_TEST_WATCHDOG,
            Box::pin(run_with_io(
                &host,
                "Continue without the untrusted server.".into(),
                format,
                HeadlessBrokers {
                    approval: Some(&approval),
                    mcp_trust_notices: &notices,
                    ..HeadlessBrokers::default()
                },
                BackgroundExit::Error,
                &mut stdout,
                &mut stderr,
            )),
        )
        .await
        .expect("headless must not wait for trust")
        .expect("headless output");
        assert_eq!(outcome.exit_code, 0);
        const NOTICE: &str = "`docs` requires interactive trust — run `/mcp trust docs`";
        match format {
            OutputFormat::Text => {
                assert_eq!(String::from_utf8(stdout).expect("text"), "done\n");
                let stderr = String::from_utf8(stderr).expect("diagnostics");
                assert_eq!(stderr.matches(&format!("smith: {NOTICE}\n")).count(), 1);
            }
            OutputFormat::Json | OutputFormat::StreamJson => {
                assert!(stderr.is_empty(), "JSON diagnostics stay in the result");
                let stdout = String::from_utf8(stdout).expect("JSON output");
                let result: serde_json::Value =
                    serde_json::from_str(stdout.lines().last().expect("terminal result"))
                        .expect("result");
                assert_eq!(result["status"], "ok");
                assert_eq!(result["diagnostics"], serde_json::json!([NOTICE]));
            }
        }
        assert!(!marker.exists(), "untrusted server was spawned");
        assert!(context.supervisor().tools().is_empty());
        assert!(
            approval.required().is_none(),
            "no trust or execution prompt"
        );
        assert!(!home.path().join(".smith/trust.json").exists());
        context.supervisor().shutdown().await;
    }
}
