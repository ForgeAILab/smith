use super::{LOCAL_COMMAND_CONFIG, TestCheckpointKeys};
use std::collections::BTreeMap;
use std::sync::Arc;

use smith_client::commands::{HostCommand, ModuleSwitchRequest, ModulesAction};
use smith_client::local_result::LocalResult;
use smith_client::modules_report::render_plain;
use smith_config::resolve::{KnownModule, Layer, Overrides, ResolveRequest, resolve};
use smith_host::ProjectWorkspace;
use smith_module::{
    CompiledModule, Module, ModuleComposition, ModuleContext, ModuleDescriptor, ModuleError,
    ModuleOrigin, Mounted,
};
use smith_runtime::factory::{HostSurface, RuntimeRequest};
use smith_runtime::host::{HostSession, HostSessionRequest};
use smith_tui::App;

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            home: tempfile::tempdir().unwrap(),
            project: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir(fixture.project.path().join(".smith")).unwrap();
        std::fs::create_dir(fixture.home.path().join(".smith")).unwrap();
        fixture.project_config(LOCAL_COMMAND_CONFIG);
        fixture
    }
    fn project_config(&self, text: &str) {
        std::fs::write(self.project.path().join(".smith/config.toml"), text).unwrap();
    }
    fn user_dir(&self) -> std::path::PathBuf {
        self.home.path().join(".smith")
    }
    fn user_config(&self, text: &str) {
        std::fs::write(self.user_dir().join("config.toml"), text).unwrap();
    }
    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path())
            .with_home_dir(self.home.path())
            .with_known_modules(crate::modules::known_modules())
    }
    fn prepared(&self) -> crate::config_command::Prepared {
        crate::config_command::Prepared {
            resolution: resolve(&self.request()).unwrap(),
            project: self.project.path().into(),
        }
    }
    async fn start(&self, modules: ModuleComposition) -> HostSession {
        let known = modules
            .compiled
            .iter()
            .map(CompiledModule::descriptor)
            .chain(modules.known.iter().cloned())
            .map(|descriptor| KnownModule {
                id: descriptor.id,
                default_enabled: descriptor.default_enabled,
                compiled_in: descriptor.compiled_in,
                legacy_enabled_key: None,
            })
            .collect();
        let mut config = resolve(&self.request().with_known_modules(known))
            .unwrap()
            .config;
        config.persistence.enabled.value = false;
        // The composition's explicit off set is the resolved fixture choice.
        for (id, choice) in &mut config.modules {
            choice.enabled.value = modules.enabled.contains(id);
        }
        let request = RuntimeRequest {
            modules,
            approval: Some(Arc::new(agent_runtime_core::approval::DenyAll)),
            workspace: Some(Arc::new(
                ProjectWorkspace::new(self.project.path()).unwrap(),
            )),
            ..RuntimeRequest::new(config, HostSurface::Terminal)
        };
        smith_runtime::host::start(
            HostSessionRequest::new(request, self.project.path())
                .checkpoint_keys(Arc::new(TestCheckpointKeys)),
        )
        .await
        .unwrap()
    }
}

#[derive(Debug)]
struct StateModule(&'static str);
impl Module for StateModule {
    fn id(&self) -> &str {
        self.0
    }
    fn revision(&self) -> &str {
        "fixture-v1"
    }
    fn description(&self) -> &str {
        "Module state fixture"
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn requirements(&self) -> &[&str] {
        match self.0 {
            "blocked" => &["off"],
            "cycle-a" => &["cycle-b"],
            "cycle-b" => &["cycle-a"],
            _ => &[],
        }
    }
    fn mount(&self, _: &ModuleContext) -> Result<Mounted, ModuleError> {
        match self.0 {
            "failed" => Err(ModuleError("fixture mount error".into())),
            "inactive" => Ok(Mounted::Inactive {
                reason: "fixture host condition".into(),
            }),
            _ => Ok(Mounted::Contributions(vec![])),
        }
    }
}

async fn list(host: &HostSession, fixture: &Fixture, action: ModulesAction) -> App {
    let mut app = App::new("example-model", "project");
    let (_, skills) = crate::skills::SkillContext::compose(
        smith_runtime::skills::SmithSkillSources::new(),
        &fixture.user_dir(),
        fixture.project.path(),
    )
    .unwrap();
    let commands = crate::local_command::file_commands::CommandContext::discover(
        &fixture.user_dir(),
        fixture.project.path(),
    )
    .unwrap();
    crate::local_command::handle_local_command(
        &mut app,
        host,
        fixture.project.path(),
        None,
        &skills,
        &commands,
        HostCommand::Modules(action),
    )
    .await;
    app
}

#[tokio::test]
async fn module_command_lists_every_state_and_an_independent_third_party_crate() {
    let fixture = Fixture::new();
    let mut compiled = [
        "mounted", "off", "blocked", "cycle-a", "cycle-b", "failed", "inactive",
    ]
    .into_iter()
    .map(|id| CompiledModule {
        module: Arc::new(StateModule(id)),
        origin: ModuleOrigin::FirstParty,
    })
    .collect::<Vec<_>>();
    compiled.push(CompiledModule {
        module: Arc::new(smith_test_module::ThirdPartyModule),
        origin: ModuleOrigin::ThirdParty {
            crate_name: "smith-test-module".into(),
        },
    });
    let mut composition = ModuleComposition::with_defaults(
        compiled,
        vec![ModuleDescriptor {
            id: "not-built".into(),
            description: "Omitted fixture".into(),
            default_enabled: true,
            origin: ModuleOrigin::FirstParty,
            compiled_in: false,
        }],
    );
    composition.enabled.remove("off");
    let host = fixture.start(composition).await;
    let app = list(&host, &fixture, ModulesAction::List).await;
    let report = app
        .transcript
        .blocks()
        .iter()
        .find_map(|block| match block {
            smith_tui::Block::Local(LocalResult::Modules(report)) => Some(report),
            _ => None,
        })
        .expect("typed local report");
    let text = render_plain(report);
    for expected in [
        "mounted · mounted",
        "off · off",
        "not-built · not built",
        "blocked · blocked · requires off",
        "cycle-a · blocked · requirement cycle: cycle-a, cycle-b",
        "failed · failed · fixture mount error",
        "inactive · inactive · fixture host condition",
        "third-party (smith-test-module)",
        "built-in default",
    ] {
        assert!(text.contains(expected), "{expected}: {text}");
    }
    let evidence = host
        .runtime()
        .harness_modules()
        .iter()
        .find(|module| {
            module.provenance
                == smith_runtime::harness::ModuleProvenance::CompiledThirdParty(
                    "smith-test-module".into(),
                )
        })
        .expect("independent crate recorded in evidence");
    assert_eq!(
        evidence.trust,
        smith_runtime::harness::ModuleTrust::TrustedNative
    );
    assert!(
        evidence
            .contributions
            .iter()
            .any(|contribution| matches!(contribution,
        smith_runtime::harness::Contribution::Command { name } if name == "native-fixture"))
    );
    assert!(
        host.snapshot().history.is_empty(),
        "listing must stay local"
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn headless_listing_matches_runtime_module_outcomes_without_a_session() {
    let fixture = Fixture::new();
    let prepared = fixture.prepared();
    assert_eq!(
        prepared.resolution.config.approval.mode.value,
        smith_config::model::ApprovalMode::Ask,
    );
    let report = crate::config_command::modules_report(&prepared)
        .await
        .unwrap();
    assert_eq!(report.rows.len(), 2);
    for row in &report.rows {
        let state = if !prepared.resolution.config.modules[&row.id].compiled_in {
            smith_module::ModuleState::NotBuilt
        } else if !prepared.resolution.config.modules[&row.id].enabled.value {
            smith_module::ModuleState::Off
        } else {
            smith_module::ModuleState::Inactive {
                reason: if row.id == "image-generation" {
                    "the active provider has no image binding".into()
                } else {
                    "semantic summaries are disabled".into()
                },
            }
        };
        assert_eq!(row.state, state);
        assert_eq!(
            row.source,
            prepared.resolution.config.modules[&row.id].enabled.source
        );
    }
    assert!(
        std::fs::read_dir(fixture.user_dir())
            .unwrap()
            .next()
            .is_none(),
        "headless listing must not create session state"
    );
    let mut request = crate::runtime_host::preflight_request(
        &prepared.resolution,
        fixture.project.path(),
        HostSurface::Terminal,
        None,
    )
    .unwrap();
    request.config.persistence.enabled.value = false;
    request.approval = Some(Arc::new(agent_runtime_core::approval::DenyAll));
    let host = smith_runtime::host::start(HostSessionRequest::new(request, fixture.project.path()))
        .await
        .unwrap();
    assert_eq!(
        report,
        smith_client::modules_report::module_report(
            host.runtime().module_report(),
            &host.runtime().policy().modules,
        ),
    );
    host.shutdown().await.unwrap();
}

fn edit_request(fingerprint: &str, enabled: bool) -> ModuleSwitchRequest {
    ModuleSwitchRequest {
        id: "image-generation".into(),
        enabled,
        fingerprint: fingerprint.into(),
    }
}

#[tokio::test]
async fn user_module_edits_preserve_legacy_spelling_and_rollback_exact_bytes() {
    for enabled in [false, true] {
        for original in [
            "# keep me\n".to_owned(),
            format!(
                "# keep me\n[tools.image_generation]\nenabled = {}\n",
                !enabled
            ),
            format!(
                "tools.image_generation.enabled = {}\nmodules.image-generation.enabled = {}\n",
                !enabled, !enabled
            ),
        ] {
            let fixture = Fixture::new();
            fixture.user_config(&original);
            let reports = [smith_module::ModuleReport {
                descriptor: ModuleDescriptor {
                    id: "image-generation".into(),
                    description: "Image module".into(),
                    default_enabled: true,
                    compiled_in: true,
                    origin: ModuleOrigin::FirstParty,
                },
                state: smith_module::ModuleState::Mounted,
            }];
            let prepared = crate::modules::prepare_switch(
                &fixture.user_dir(),
                &reports,
                "image-generation",
                enabled,
            )
            .unwrap();
            let legacy = original.contains("tools.image_generation");
            assert!(prepared.edit.preview().contains(if legacy {
                "tools.image_generation.enabled"
            } else {
                "modules.image-generation.enabled"
            }));
            let committed = crate::modules::commit_switch(
                &fixture.user_dir(),
                &reports,
                &edit_request(&prepared.fingerprint, enabled),
            )
            .unwrap();
            let resolved = resolve(&fixture.request()).unwrap();
            assert_eq!(
                resolved.config.modules["image-generation"].enabled.value,
                enabled
            );
            assert_eq!(
                resolved.config.modules["image-generation"]
                    .enabled
                    .source
                    .layer,
                Layer::UserFile
            );
            let stored = std::fs::read_to_string(fixture.user_dir().join("config.toml")).unwrap();
            if legacy && !original.contains("modules.") {
                assert!(!stored.contains("modules"));
            }
            committed.rollback().unwrap();
            assert_eq!(
                std::fs::read_to_string(fixture.user_dir().join("config.toml")).unwrap(),
                original
            );
        }
    }
}

#[tokio::test]
async fn module_switch_refusals_write_nothing_and_stale_previews_are_rejected() {
    let fixture = Fixture::new();
    let reports = [smith_module::ModuleReport {
        descriptor: ModuleDescriptor {
            id: "image-generation".into(),
            description: "fixture".into(),
            default_enabled: true,
            compiled_in: false,
            origin: ModuleOrigin::FirstParty,
        },
        state: smith_module::ModuleState::NotBuilt,
    }];
    let unknown = crate::modules::prepare_switch(&fixture.user_dir(), &reports, "nope", true)
        .err()
        .unwrap()
        .to_string();
    assert!(unknown.contains("unknown module `nope`"));
    assert!(unknown.contains("known ids: image-generation"));
    let missing =
        crate::modules::prepare_switch(&fixture.user_dir(), &reports, "image-generation", true)
            .err()
            .unwrap()
            .to_string();
    assert!(missing.contains("not in this build"));
    assert!(!fixture.user_dir().join("config.toml").exists());
    let prepared =
        crate::modules::prepare_switch(&fixture.user_dir(), &reports, "image-generation", false)
            .unwrap();
    let committed = crate::modules::commit_switch(
        &fixture.user_dir(),
        &reports,
        &edit_request(&prepared.fingerprint, false),
    )
    .unwrap();
    committed.rollback().unwrap();
    assert!(
        !fixture.user_dir().join("config.toml").exists(),
        "rollback removes a newly-created config"
    );
    fixture.user_config("# changed after review\n");
    let error = crate::modules::commit_switch(
        &fixture.user_dir(),
        &reports,
        &edit_request(&prepared.fingerprint, false),
    )
    .unwrap_err();
    assert!(error.to_string().contains("changed since the preview"));
    assert_eq!(
        std::fs::read_to_string(fixture.user_dir().join("config.toml")).unwrap(),
        "# changed after review\n"
    );
}

#[test]
fn higher_layers_are_named_without_claiming_the_user_switch_took_effect() {
    for (layer, legacy) in [
        (Layer::ProjectFile, true),
        (Layer::Profile, false),
        (Layer::Environment, true),
        (Layer::CommandLine, false),
        (Layer::SessionOverride, false),
    ] {
        let fixture = Fixture::new();
        let key = if legacy {
            "tools.image_generation.enabled"
        } else {
            "modules.image-generation.enabled"
        };
        let mut request = fixture.request();
        match layer {
            Layer::ProjectFile => {
                fixture.project_config(&format!("{key} = true\n{LOCAL_COMMAND_CONFIG}"))
            }
            Layer::Profile => fixture.project_config(&format!(
                "{LOCAL_COMMAND_CONFIG}\n[profiles.dev.modules.image-generation]\nenabled = true\n"
            )),
            Layer::Environment => {
                request
                    .env
                    .insert(smith_config::resolve::env_name(key), "true".into());
            }
            Layer::CommandLine | Layer::SessionOverride => {
                let overrides = Overrides {
                    modules: BTreeMap::from([("image-generation".into(), true)]),
                    ..Overrides::default()
                };
                if layer == Layer::CommandLine {
                    request.cli = overrides;
                } else {
                    request.session = overrides;
                }
            }
            _ => unreachable!(),
        }
        fixture.user_config("modules.image-generation.enabled = false\n");
        let resolved = resolve(&request).unwrap();
        let choice = &resolved.config.modules["image-generation"];
        let text = crate::modules::switch_outcome("image-generation", false, choice);
        assert!(text.contains(layer.label()), "{text}");
        assert!(text.contains(&choice.enabled.source.key), "{text}");
        assert!(text.contains("overrides it"), "{text}");
        assert!(!text.contains("applied at the safe boundary"));
    }
}

#[cfg(not(any(feature = "module-image-generation", feature = "module-budget-notice")))]
#[tokio::test]
async fn minimal_build_starts_a_session_with_core_tools_and_no_optional_modules() {
    let fixture = Fixture::new();
    let host = fixture.start(crate::modules::composition()).await;
    assert!(host.runtime().mounted_modules().is_empty());
    assert!(
        host.runtime()
            .module_report()
            .iter()
            .all(|row| row.state == smith_module::ModuleState::NotBuilt)
    );
    for tool in ["read", "search", "shell"] {
        assert!(
            host.runtime()
                .policy()
                .tools
                .iter()
                .any(|name| name == tool)
        );
    }
    let turn = host
        .session()
        .send(agent_runtime_core::content::UserInput::text("hello"))
        .unwrap();
    turn.completed().await;
    assert!(
        host.snapshot()
            .history
            .iter()
            .any(|message| message.role == agent_runtime_core::content::Role::Assistant)
    );
    host.shutdown().await.unwrap();
}

#[test]
fn config_modules_parses_selection_flags_and_help_without_starting_a_session() {
    use std::ffi::OsString;
    let parsed = crate::cli::parse(
        ["config", "modules", "--project", ".", "--profile", "dev"].map(OsString::from),
    )
    .unwrap();
    assert!(
        matches!(parsed, crate::cli::Command::ConfigModules { selection } if selection.profile.as_deref() == Some("dev") && selection.project == Some(std::path::PathBuf::from(".")))
    );
    assert_eq!(
        crate::cli::parse(["config", "modules", "--help"].map(OsString::from)).unwrap(),
        crate::cli::Command::Help
    );
    assert!(crate::cli::HELP.contains("smith config modules [SELECTION OPTIONS]"));
}

#[tokio::test]
async fn module_command_previews_before_writing_and_refuses_unknown_ids() {
    let fixture = Fixture::new();
    let composition = ModuleComposition::with_defaults(
        vec![CompiledModule {
            module: Arc::new(StateModule("mounted")),
            origin: ModuleOrigin::FirstParty,
        }],
        vec![ModuleDescriptor {
            id: "not-built".into(),
            description: "Omitted".into(),
            default_enabled: true,
            compiled_in: false,
            origin: ModuleOrigin::FirstParty,
        }],
    );
    let host = fixture.start(composition).await;
    let mut app = list(
        &host,
        &fixture,
        ModulesAction::Switch {
            id: "mounted".into(),
            enabled: false,
        },
    )
    .await;
    assert!(matches!(
        app.overlay,
        Some(smith_tui::app::Overlay::Confirm(_))
    ));
    assert!(!fixture.user_dir().join("config.toml").exists());
    assert_eq!(
        app.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE
        )),
        None
    );
    assert!(!fixture.user_dir().join("config.toml").exists());
    for (id, expected) in [
        ("nope", "known ids: mounted, not-built"),
        ("not-built", "not in this build"),
    ] {
        let app = list(
            &host,
            &fixture,
            ModulesAction::Switch {
                id: id.into(),
                enabled: true,
            },
        )
        .await;
        assert!(app.overlay.is_none());
        assert!(app.transcript.blocks().iter().any(|block| matches!(block,
            smith_tui::Block::Local(LocalResult::Message(report))
                if smith_client::message_report::render_plain(report).contains(expected))));
        assert!(!fixture.user_dir().join("config.toml").exists());
    }
    host.shutdown().await.unwrap();
}
