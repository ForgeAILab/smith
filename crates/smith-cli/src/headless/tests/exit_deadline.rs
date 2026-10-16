//! Deadline failures still produce the headless terminal result.

use super::*;

#[tokio::test]
async fn headless_result_reports_exit_deadline_for_a_hung_shutdown() {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    let directory = project.path().join(".smith");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("config.toml"),
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
[approval]
mode = "deny"
"#,
    )
    .unwrap();
    let config = resolve(
        &ResolveRequest::new(project.path())
            .with_known_modules(crate::modules::known_modules())
            .with_home_dir(home.path()),
    )
    .unwrap()
    .config;
    let mut runtime = RuntimeRequest::new(config, HostSurface::Headless);
    runtime.workspace = Some(Arc::new(ProjectWorkspace::new(project.path()).unwrap()));
    runtime.provider = Some(Arc::new(FakeProvider::text_reply("done")));
    let host = smith_runtime::host::start(host_request(runtime, project.path()))
        .await
        .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let outcome = crate::exit_deadline::with_hung_shutdown(
        Duration::from_millis(30),
        Box::pin(run_with_io(
            &host,
            UserInput::text("hello"),
            OutputFormat::Json,
            HeadlessBrokers::default(),
            BackgroundExit::Error,
            &mut stdout,
            &mut stderr,
        )),
    )
    .await
    .unwrap();
    assert_eq!(outcome.exit_code, 1);
    assert!(stderr.is_empty());
    let result: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(result["type"], "result");
    assert_eq!(result["status"], "failed");
    assert_eq!(result["error"], "exit deadline reached");
    assert_eq!(result["output"], "done");
    Box::pin(host.shutdown()).await.expect("fixture cleanup");
}
