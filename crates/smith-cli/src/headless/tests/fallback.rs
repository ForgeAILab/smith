use super::*;

const CONFIG: &str = r#"
default_profile = "work"
[providers.local]
kind = "fake"
[providers.backup]
kind = "fake"
[profiles.work]
provider = "local"
model = "primary"
[profiles.work.fallback]
first_response_timeout_ms = 100
models = [{ provider = "backup", model = "secondary", first_response_timeout_ms = 5000 }]
[models."local/primary"]
context_tokens = 32768
max_input_tokens = 30000
max_output_tokens = 1024
[models."backup/secondary"]
context_tokens = 16384
max_input_tokens = 15000
max_output_tokens = 512
"#;

#[tokio::test]
async fn fallback_reports_actual_model_unknown_abandoned_usage_and_typed_stream_events() {
    for format in [OutputFormat::Json, OutputFormat::StreamJson] {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".smith")).unwrap();
        std::fs::write(project.path().join(".smith/config.toml"), CONFIG).unwrap();
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .unwrap()
            .config;
        let primary = Arc::new(FakeProvider::new(
            "primary",
            Capabilities::basic_streaming(),
            vec![ScriptedStream::blocking(vec![])],
        ));
        let backup = Arc::new(FakeProvider::new(
            "secondary",
            Capabilities::basic_streaming(),
            vec![ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "fallback completed".into(),
                },
                usage_event(9, 3),
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])],
        ));
        let mut runtime = RuntimeRequest::new(config, HostSurface::Headless);
        runtime.workspace = Some(Arc::new(ProjectWorkspace::new(project.path()).unwrap()));
        runtime.approval = Some(Arc::new(HeadlessApproval::new()));
        runtime.provider = Some(primary.clone());
        runtime
            .fallback_providers
            .insert(("backup".into(), "secondary".into()), backup.clone());
        let host = smith_runtime::host::start(host_request(runtime, project.path()))
            .await
            .unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let outcome = tokio::time::timeout(
            HEADLESS_TEST_WATCHDOG,
            run_with_io(
                &host,
                UserInput::text("hello"),
                format,
                HeadlessBrokers::default(),
                BackgroundExit::Error,
                &mut stdout,
                &mut stderr,
            ),
        )
        .await
        .expect("fallback run watchdog")
        .unwrap();
        assert_eq!(outcome.exit_code, 0, "{}", String::from_utf8_lossy(&stdout));
        let lines = String::from_utf8(stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        let result = lines.last().unwrap();
        assert_eq!(result["provider"], "backup");
        assert_eq!(result["model"], "secondary");
        assert_eq!(result["output"], "fallback completed");
        let bindings = result["usage"]["current_run_bindings"].as_array().unwrap();
        let abandoned = bindings
            .iter()
            .find(|binding| binding["provider"] == "local")
            .unwrap();
        assert_eq!(abandoned["reported"], false);
        assert!(abandoned["totals"].as_object().unwrap().is_empty());
        let completed = bindings
            .iter()
            .find(|binding| binding["provider"] == "backup")
            .unwrap();
        assert_eq!(completed["reported"], true);
        assert_eq!(primary.requests().len(), 1);
        assert_eq!(backup.requests().len(), 1);
        let notice = String::from_utf8(stderr).unwrap();
        assert!(notice.contains("model fallback"), "{notice}");
        assert!(notice.contains("backup/secondary"), "{notice}");
        if format == OutputFormat::StreamJson {
            assert!(
                lines
                    .iter()
                    .any(|line| line["event"]["payload"]["event"] == "model_fallback")
            );
        }
    }
}
