use super::*;

/// A binding with an exact dialect and a real effort ladder, so a selected
/// effort is something the resolver can accept rather than refuse.
fn reasoning_config() -> String {
    format!(
        r#"{CONFIG}
[models."local/example-model".reasoning]
toggle = true
efforts = ["low", "medium", "high"]
dialect = "openai-effort"
"#
    )
}

/// The effort saved with the session, or `None` when nothing is saved.
async fn saved_effort(
    paths: &smith_runtime::session::SessionPaths,
    id: &SessionId,
) -> Option<String> {
    let snapshot = FileSessionStore::new(paths.clone())
        .load(id)
        .await
        .expect("a readable snapshot")?;
    let state = snapshot
        .extension_state
        .get("smith.reasoning.override")?
        .value
        .clone();
    state
        .get("effort")
        .and_then(|effort| effort.as_str())
        .map(str::to_owned)
}

#[tokio::test]
async fn an_invocation_effort_shadows_a_saved_one_for_the_run_without_replacing_it() {
    let fixture = Fixture::with_config(&reasoning_config());
    let config_with = |cli: Overrides, session: Overrides| {
        resolve(
            &ResolveRequest::new(fixture.project.path())
                .with_home_dir(fixture.home.path())
                .with_cli(cli)
                .with_session(session),
        )
        .expect("resolved configuration")
        .config
    };

    // A session that chose `low` for itself, the way `/effort low` does.
    let chosen = start(fixture.request_with_config(
        config_with(
            Overrides::default(),
            Overrides {
                reasoning_effort: Some("low".to_owned()),
                ..Overrides::default()
            },
        ),
        HostSurface::Headless,
    ))
    .await
    .expect("a session with an in-session effort");
    let session_id = chosen.session().id().clone();
    let paths = chosen.paths().expect("persistent paths").clone();
    assert_eq!(
        chosen.runtime().policy().reasoning.effective_effort(),
        "low"
    );
    chosen
        .session()
        .run(UserInput::text("remember this effort"))
        .await
        .expect("the turn runs");
    chosen.shutdown().await.expect("a clean shutdown");
    assert_eq!(
        saved_effort(&paths, &session_id).await.as_deref(),
        Some("low")
    );

    // Resumed with `--effort high`: the flag answers for this run, and the
    // run's own save does not rewrite what the session chose.
    let flagged = start(
        fixture
            .request_with_config(
                config_with(
                    Overrides {
                        reasoning_effort: Some("high".to_owned()),
                        ..Overrides::default()
                    },
                    Overrides::default(),
                ),
                HostSurface::Headless,
            )
            .reasoning_effort_shadowed(true)
            .resume(session_id.clone()),
    )
    .await
    .expect("a resumed session that takes the flag");
    let policy = &flagged.runtime().policy().reasoning;
    assert_eq!(policy.effective_effort(), "high");
    assert_eq!(policy.selection_source, "command-line flag `--effort`");
    flagged
        .session()
        .run(UserInput::text("run under the flag"))
        .await
        .expect("the resumed turn runs");
    flagged.shutdown().await.expect("a clean resumed shutdown");
    assert_eq!(
        saved_effort(&paths, &session_id).await.as_deref(),
        Some("low"),
        "an invocation flag must not overwrite the session's own choice"
    );

    // Resumed without the flag: the session's own effort is back, unchanged.
    let restored = start(
        fixture
            .request_with_config(
                config_with(Overrides::default(), Overrides::default()),
                HostSurface::Headless,
            )
            .resume(session_id.clone()),
    )
    .await
    .expect("a resumed session with no flag");
    assert_eq!(
        restored.runtime().policy().reasoning.effective_effort(),
        "low"
    );
    assert_eq!(
        restored.runtime().policy().reasoning.selection_source,
        "session override"
    );
    restored.shutdown().await.expect("a clean third shutdown");
}

#[tokio::test]
async fn an_unadvertised_invocation_effort_fails_before_the_session_exists() {
    let fixture = Fixture::with_config(&reasoning_config());
    let config = resolve(
        &ResolveRequest::new(fixture.project.path())
            .with_home_dir(fixture.home.path())
            .with_cli(Overrides {
                reasoning_effort: Some("ludicrous".to_owned()),
                ..Overrides::default()
            }),
    )
    .expect("resolution itself does not judge the ladder")
    .config;
    let sessions_dir = config.persistence.sessions_dir.value.clone();

    let error = start(fixture.request_with_config(config, HostSurface::Headless))
        .await
        .expect_err("an unadvertised effort is refused");
    let message = error.to_string();
    assert!(message.contains("ludicrous"), "{message}");
    assert!(message.contains("low, medium, high"), "{message}");
    assert!(
        !sessions_dir.exists(),
        "a refused effort still created session state at {}",
        sessions_dir.display()
    );
}
