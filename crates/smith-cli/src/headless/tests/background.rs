//! Background-exit policy, account projection, and host integration tests.

use super::*;

fn pooled(active: usize) -> SharedPool {
    let mut pool = smith_runtime::pool::CredentialPool::new(
        "acme",
        [
            "keychain:smith/personal".to_owned(),
            "keychain:smith/work".to_owned(),
        ],
        None,
    );
    pool.set_active(active);
    SharedPool::new(pool)
}

#[test]
fn a_single_credential_provider_projects_no_account() {
    // Nothing to disambiguate, so the field is absent rather than a row
    // naming the only credential there is.
    assert!(account_output(None, None).is_none());
}

#[test]
fn a_headless_run_projects_the_account_it_used() {
    let pool = pooled(1);
    let account = account_output(Some(&pool), None).expect("an account");

    assert_eq!(account.position, 1);
    assert_eq!(account.reference, "keychain:smith/work");
    // Never measured, so no percentage is invented.
    assert_eq!(account.used_percent, None);
    assert!(!account.exhausted);
    assert_eq!(account.resets_at_ms, None);
    // One other account existed and was deliberately not used.
    assert_eq!(account.unused_members, 1);

    let value = serde_json::to_value(&account).expect("the account serializes");
    assert_eq!(value["reference"], "keychain:smith/work");
    assert_eq!(value["exhausted"], false);
    // Absent rather than null: a consumer must not read "unmeasured" as a
    // number.
    assert!(value.get("used_percent").is_none());
}

#[tokio::test]
async fn an_exhausted_headless_run_reports_the_reset_it_stopped_on() {
    let pool = pooled(0);
    let rotation = HeadlessRotation::new();
    let request = smith_host::rotation::RotationRequest {
        provider: "acme".to_owned(),
        trigger: smith_host::rotation::RotationTrigger::Exhausted,
        outgoing: smith_host::rotation::RotationMember {
            position: 0,
            label: "keychain:smith/personal".to_owned(),
            used_percent: Some(100.0),
            cooling_until_ms: None,
        },
        eligible: vec![smith_host::rotation::RotationMember {
            position: 1,
            label: "keychain:smith/work".to_owned(),
            used_percent: None,
            cooling_until_ms: None,
        }],
        outgoing_resets_at_ms: Some(1_785_866_400_000),
    };
    // The policy declines and records, which is what a headless run does.
    {
        use smith_host::rotation::RotationPolicy;
        rotation.decide(&request).await;
    }

    let account = account_output(Some(&pool), Some(&rotation)).expect("an account");
    assert!(account.exhausted);
    assert_eq!(account.resets_at_ms, Some(1_785_866_400_000));
    // The run stayed put: the account is still the one it started on.
    assert_eq!(account.reference, "keychain:smith/personal");
}

fn sample_running_task(task_id: &str, command: &str) -> BackgroundTaskInfo {
    BackgroundTaskInfo {
        task_id: task_id.to_owned(),
        command: command.to_owned(),
        cwd: std::env::temp_dir(),
        spool_path: std::env::temp_dir().join("fixture.log"),
        status: TaskStatus::Running,
        timeout_ms: None,
    }
}

/// A fresh session ID every call so task and spool diagnostics stay
/// unambiguous even though every test owns an isolated registry.
fn unique_background_test_session(label: &str) -> SessionId {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    SessionId::new(format!(
        "headless-bg-exit-{label}-{}-{n}",
        std::process::id()
    ))
}

#[test]
fn no_running_tasks_never_needs_a_background_exit_decision() {
    for policy in [
        BackgroundExit::Error,
        BackgroundExit::Wait,
        BackgroundExit::Stop,
    ] {
        assert_eq!(
            decide_background_exit(policy, &[]),
            BackgroundExitDecision::Clear
        );
    }
}

#[test]
fn the_default_policy_is_error_and_names_the_task_by_id_and_command() {
    let running = [sample_running_task("task:7", "cargo test --workspace")];
    let decision = decide_background_exit(BackgroundExit::default(), &running);
    let BackgroundExitDecision::Error(message) = decision else {
        panic!("the default policy must fail closed instead of orphaning: {decision:?}");
    };
    assert!(message.contains("task:7"), "{message}");
    assert!(message.contains("cargo test --workspace"), "{message}");
}

#[test]
fn wait_and_stop_policies_defer_to_the_async_poll_instead_of_deciding_synchronously() {
    let running = [sample_running_task("task:8", "sleep 30")];
    assert_eq!(
        decide_background_exit(BackgroundExit::Wait, &running),
        BackgroundExitDecision::Wait
    );
    assert_eq!(
        decide_background_exit(BackgroundExit::Stop, &running),
        BackgroundExitDecision::Stop
    );
}

#[test]
fn multiple_running_tasks_are_all_named_in_the_error_policy_message() {
    let running = [
        sample_running_task("task:1", "make build"),
        sample_running_task("task:2", "npm test"),
    ];
    let decision = decide_background_exit(BackgroundExit::Error, &running);
    let BackgroundExitDecision::Error(message) = decision else {
        panic!("expected an error decision: {decision:?}");
    };
    assert!(
        message.starts_with("2 background shell task(s)"),
        "{message}"
    );
    assert!(
        message.contains("task:1") && message.contains("make build"),
        "{message}"
    );
    assert!(
        message.contains("task:2") && message.contains("npm test"),
        "{message}"
    );
}

#[tokio::test]
async fn no_running_tasks_leaves_no_error_and_no_report_under_every_policy() {
    let registry = BackgroundTaskRegistry::new();
    let session_id = unique_background_test_session("clear");
    for policy in [
        BackgroundExit::Error,
        BackgroundExit::Wait,
        BackgroundExit::Stop,
    ] {
        let (error, output) = apply_background_exit_policy(&registry, &session_id, policy).await;
        assert!(error.is_none());
        assert!(output.is_none());
    }
}

#[tokio::test]
async fn error_policy_reports_a_running_task_without_waiting_for_it() {
    let session_id = unique_background_test_session("error");
    let registry = BackgroundTaskRegistry::new();
    let (task_id, _spool) = registry
        .spawn_background_task(&session_id, "sleep 2".into(), std::env::temp_dir(), None)
        .await
        .expect("a spawned background task");

    let (error, output) =
        apply_background_exit_policy(&registry, &session_id, BackgroundExit::Error).await;

    let error = error.expect("the default policy fails closed");
    assert!(error.contains(&task_id), "{error}");
    assert!(error.contains("sleep 2"), "{error}");
    let output = output.expect("a background-exit report");
    assert_eq!(output.policy, "error");
    assert_eq!(output.tasks.len(), 1);
    assert_eq!(output.tasks[0].task_id, task_id);
    assert_eq!(output.tasks[0].status, "running");
    assert_eq!(output.tasks[0].exit_code, None);

    // The policy only reports; it never waits or stops on its own.
    assert_eq!(registry.running_tasks(&session_id).len(), 1);
    let _ = registry.stop_task(&session_id, &task_id).await;
}

#[tokio::test]
async fn wait_policy_blocks_until_the_task_exits_and_reports_its_exit_code() {
    let session_id = unique_background_test_session("wait");
    let registry = BackgroundTaskRegistry::new();
    let (task_id, _spool) = registry
        .spawn_background_task(&session_id, "sleep 0.2".into(), std::env::temp_dir(), None)
        .await
        .expect("a spawned background task");

    let (error, output) =
        apply_background_exit_policy(&registry, &session_id, BackgroundExit::Wait).await;

    assert!(error.is_none());
    let output = output.expect("a background-exit report");
    assert_eq!(output.policy, "wait");
    assert_eq!(output.tasks.len(), 1);
    assert_eq!(output.tasks[0].task_id, task_id);
    assert_eq!(output.tasks[0].status, "exited");
    assert_eq!(output.tasks[0].exit_code, Some(0));
    assert!(registry.running_tasks(&session_id).is_empty());
}

#[tokio::test]
async fn stop_policy_ends_a_long_running_task_well_before_its_own_deadline() {
    let session_id = unique_background_test_session("stop");
    let registry = BackgroundTaskRegistry::new();
    let (task_id, _spool) = registry
        .spawn_background_task(&session_id, "sleep 30".into(), std::env::temp_dir(), None)
        .await
        .expect("a spawned background task");

    let (error, output) = tokio::time::timeout(
        HEADLESS_TEST_WATCHDOG,
        apply_background_exit_policy(&registry, &session_id, BackgroundExit::Stop),
    )
    .await
    .expect("stop should not wait for the 30s command to finish on its own");

    assert!(error.is_none());
    let output = output.expect("a background-exit report");
    assert_eq!(output.policy, "stop");
    assert_eq!(output.tasks.len(), 1);
    assert_eq!(output.tasks[0].task_id, task_id);
    assert_eq!(output.tasks[0].status, "stopped");
    assert_eq!(output.tasks[0].exit_code, None);
    assert!(registry.running_tasks(&session_id).is_empty());
}

const BACKGROUND_EXIT_CONFIG: &str = r#"
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
"#;

#[tokio::test]
async fn default_error_policy_fails_a_headless_run_with_a_running_background_task() {
    let home = tempfile::tempdir().expect("a home");
    let project = tempfile::tempdir().expect("a project");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a config directory");
    std::fs::write(config_dir.join("config.toml"), BACKGROUND_EXIT_CONFIG).expect("a config");
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved config")
        .config;
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        approval: Some(Arc::new(HeadlessApproval::new())),
        provider: Some(Arc::new(FakeProvider::text_reply("the answer")) as Arc<dyn Provider>),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = smith_runtime::host::start(host_request(runtime, project.path()))
        .await
        .expect("a host");
    let registry = host.background_tasks().clone();
    let (task_id, _spool) = registry
        .spawn_background_task(
            host.session().id(),
            // Keep the task alive well beyond a contended headless turn.
            // A one-second sleep made this assertion depend on CI timing:
            // the correct error policy sees no running work after it exits.
            "sleep 30".into(),
            std::env::temp_dir(),
            None,
        )
        .await
        .expect("a spawned background task");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let outcome = run_with_io(
        &host,
        "hello".into(),
        OutputFormat::Json,
        HeadlessBrokers::default(),
        BackgroundExit::Error,
        &mut stdout,
        &mut stderr,
    )
    .await
    .expect("a structured result");

    assert_eq!(outcome.exit_code, 1);
    let result: serde_json::Value = serde_json::from_slice(&stdout).expect("a result envelope");
    assert_eq!(result["status"], "failed");
    assert_eq!(result["background_exit"]["policy"], "error");
    assert_eq!(result["background_exit"]["tasks"][0]["task_id"], task_id);
    assert_eq!(result["background_exit"]["tasks"][0]["status"], "running");
    assert!(
        result["error"]
            .as_str()
            .is_some_and(|error| error.contains(&task_id)),
        "{result:#}"
    );

    let _ = registry.stop_task(host.session().id(), &task_id).await;
}

#[tokio::test]
async fn wait_policy_lets_a_headless_run_finish_after_its_background_task_exits() {
    let home = tempfile::tempdir().expect("a home");
    let project = tempfile::tempdir().expect("a project");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a config directory");
    std::fs::write(config_dir.join("config.toml"), BACKGROUND_EXIT_CONFIG).expect("a config");
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved config")
        .config;
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        approval: Some(Arc::new(HeadlessApproval::new())),
        provider: Some(Arc::new(FakeProvider::text_reply("the answer")) as Arc<dyn Provider>),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = smith_runtime::host::start(host_request(runtime, project.path()))
        .await
        .expect("a host");
    let registry = host.background_tasks().clone();
    let (task_id, _spool) = registry
        .spawn_background_task(
            host.session().id(),
            // Long enough to still be running when the policy check
            // happens (after host startup and the turn itself), short
            // enough that `wait` polling it to completion stays fast.
            "sleep 3".into(),
            std::env::temp_dir(),
            None,
        )
        .await
        .expect("a spawned background task");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let outcome = run_with_io(
        &host,
        "hello".into(),
        OutputFormat::Json,
        HeadlessBrokers::default(),
        BackgroundExit::Wait,
        &mut stdout,
        &mut stderr,
    )
    .await
    .expect("a structured result");

    assert_eq!(outcome.exit_code, 0);
    let result: serde_json::Value = serde_json::from_slice(&stdout).expect("a result envelope");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["background_exit"]["policy"], "wait");
    assert_eq!(result["background_exit"]["tasks"][0]["task_id"], task_id);
    assert_eq!(result["background_exit"]["tasks"][0]["status"], "exited");
    assert_eq!(result["background_exit"]["tasks"][0]["exit_code"], 0);
}
