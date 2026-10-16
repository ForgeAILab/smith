//! Caller-authorized image ingestion and request projection.

use super::*;
use agent_runtime_core::content::ContentPart;

fn image_config(
    accepts_images: bool,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    smith_config::resolve::ResolvedConfig,
) {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join(".smith")).unwrap();
    let modalities = if accepts_images {
        "[\"text\", \"image\"]"
    } else {
        "[\"text\"]"
    };
    std::fs::write(
        project.path().join(".smith/config.toml"),
        format!(
            r#"
default_profile = "dev"
[profiles.dev]
provider = "local"
model = "example-model"
delegation = false
[providers.local]
kind = "fake"
[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
input_modalities = {modalities}
"#
        ),
    )
    .unwrap();
    let config = resolve(
        &ResolveRequest::new(project.path())
            .with_known_modules(crate::modules::known_modules())
            .with_home_dir(home.path()),
    )
    .unwrap()
    .config;
    (home, project, config)
}

#[test]
fn headless_image_errors_name_the_path_and_cli_agents_are_refused_first() {
    let (_home, project, mut config) = image_config(true);
    let missing = project.path().join("missing.png");
    let error = prepare_image_input("describe".into(), std::slice::from_ref(&missing), &config)
        .unwrap_err();
    assert!(
        error.to_string().contains(missing.to_str().unwrap()),
        "{error}"
    );
    let corrupt = project.path().join("broken.png");
    std::fs::write(&corrupt, b"\x89PNG\r\n\x1a\ninvalid").unwrap();
    let error = prepare_image_input("describe".into(), std::slice::from_ref(&corrupt), &config)
        .unwrap_err();
    assert!(
        error.to_string().contains(corrupt.to_str().unwrap()),
        "{error}"
    );
    assert!(
        error.to_string().contains("unsupported or corrupt image"),
        "{error}"
    );
    config.model.value = "cli/codex".into();
    let error = prepare_image_input("describe".into(), &[missing], &config).unwrap_err();
    assert_eq!(
        error.to_string(),
        "--image is not supported for installed coding agents yet"
    );
    assert_eq!(
        prepare_image_input("text still works".into(), &[], &config).unwrap(),
        UserInput::text("text still works")
    );
}

#[tokio::test]
async fn headless_image_prompt_reaches_the_first_request_or_text_only_placeholder() {
    for accepts_images in [true, false] {
        let (home, project, config) = image_config(accepts_images);
        // Both inputs are outside the project, exercising caller authority.
        let png = crate::submission::encode_png(2, 3, &[255; 24]).unwrap();
        let paths = [
            home.path().join("first.png"),
            home.path().join("second.dat"),
        ];
        std::fs::write(&paths[0], &png).unwrap();
        std::fs::write(
            &paths[1],
            crate::submission::encode_png(2, 3, &[0; 24]).unwrap(),
        )
        .unwrap();
        let input = prepare_image_input("compare these pictures".into(), &paths, &config).unwrap();
        let expected_urls = input
            .parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Image { url, .. } => Some(url.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(expected_urls.len(), 2);
        let provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "two pictures".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])],
        ));
        let mut runtime = RuntimeRequest::new(config, HostSurface::Headless);
        runtime.workspace = Some(Arc::new(ProjectWorkspace::new(project.path()).unwrap()));
        runtime.approval = Some(Arc::new(HeadlessApproval::new()));
        runtime.provider = Some(provider.clone());
        let host = smith_runtime::host::start(host_request(runtime, project.path()))
            .await
            .unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let outcome = tokio::time::timeout(
            HEADLESS_TEST_WATCHDOG,
            run_with_io(
                &host,
                input,
                OutputFormat::Json,
                HeadlessBrokers::default(),
                BackgroundExit::Error,
                &mut stdout,
                &mut stderr,
            ),
        )
        .await
        .expect("headless image watchdog")
        .unwrap();
        assert_eq!(outcome.exit_code, 0, "{}", String::from_utf8_lossy(&stdout));
        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        let user = requests[0]
            .messages
            .iter()
            .find(|message| message.role == Role::User)
            .unwrap();
        let images = user
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Image { url, .. } => Some(url),
                _ => None,
            })
            .collect::<Vec<_>>();
        if accepts_images {
            assert_eq!(images, expected_urls.iter().collect::<Vec<_>>());
        } else {
            assert!(images.is_empty());
            assert!(
                user.joined_text().contains("[image "),
                "{}",
                user.joined_text()
            );
        }
        assert!(user.joined_text().contains("compare these pictures"));
        assert!(user.joined_text().contains("2×3 PNG · 1 KiB"));
    }
}

#[test]
fn headless_image_relative_paths_use_process_cwd_instead_of_project() {
    let (_home, _project, config) = image_config(true);
    let cwd = std::env::current_dir().unwrap();
    let fixture = tempfile::tempdir_in(&cwd).unwrap();
    let path = fixture.path().join("picture.dat");
    let png = crate::submission::encode_png(1, 1, &[255; 4]).unwrap();
    std::fs::write(&path, png).unwrap();
    let relative = path.strip_prefix(&cwd).unwrap().to_owned();
    let input = prepare_image_input("describe".into(), &[relative], &config).unwrap();
    assert!(matches!(input.parts[1], ContentPart::Image { .. }));
}
