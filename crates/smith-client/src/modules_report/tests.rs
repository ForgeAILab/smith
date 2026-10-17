use std::collections::BTreeMap;

use smith_config::resolve::{Layer, ResolvedModule, Source, Sourced};
use smith_module::{ModuleDescriptor, ModuleReport};

use super::{ModuleBlockReason, ModuleOrigin, ModuleState, module_report, render_plain};

#[test]
fn non_fatal_watcher_diagnostics_remain_visible_on_a_mounted_module() {
    let health = std::sync::Arc::new(smith_module::ModuleHealth::default());
    health.contribution_diagnostic(
        "watch",
        "watcher refused: session_notify capability was not granted",
    );
    let reports = [ModuleReport {
        health: Some(health),
        descriptor: ModuleDescriptor {
            id: "plugin/module".into(),
            description: "Fixture".into(),
            default_enabled: true,
            compiled_in: true,
            origin: ModuleOrigin::Sandboxed {
                plugin: "plugin".into(),
                requested_capabilities: [smith_module::ModuleCapability::SessionNotify]
                    .into_iter()
                    .collect(),
                granted_capabilities: Default::default(),
                capability_details: vec![],
                withheld_capabilities: vec!["notify the live session and start turns".into()],
            },
        },
        state: ModuleState::Mounted,
    }];
    let report = module_report(&reports, &BTreeMap::new());
    assert_eq!(report.rows[0].state, ModuleState::Mounted);
    let text = render_plain(&report);
    assert!(text.contains("watcher refused: session_notify capability was not granted"));
    assert!(text.contains("withheld by the user"));
}

#[test]
fn every_mount_outcome_keeps_the_deciding_key_layer_and_origin() {
    let cases = [
        (ModuleState::Mounted, "mounted"),
        (ModuleState::Off, "off"),
        (ModuleState::NotBuilt, "not built"),
        (
            ModuleState::Blocked {
                reason: ModuleBlockReason::Requirement("dependency".into()),
            },
            "blocked · requires dependency",
        ),
        (
            ModuleState::Blocked {
                reason: ModuleBlockReason::Cycle(vec!["a".into(), "b".into()]),
            },
            "blocked · requirement cycle: a, b",
        ),
        (
            ModuleState::Failed {
                reason: "mount refused".into(),
            },
            "failed · mount refused",
        ),
        (
            ModuleState::Inactive {
                reason: "no image binding".into(),
            },
            "inactive · no image binding",
        ),
    ];
    for (state, label) in cases {
        let switches = BTreeMap::from([(
            "image-generation".into(),
            ResolvedModule {
                enabled: Sourced {
                    value: true,
                    source: Source::file(
                        Layer::UserFile,
                        "/home/.smith/config.toml",
                        "tools.image_generation.enabled",
                    ),
                },
                compiled_in: true,
            },
        )]);
        let reports = [ModuleReport {
            health: None,
            descriptor: ModuleDescriptor {
                id: "image-generation".into(),
                description: "Make images".into(),
                default_enabled: true,
                compiled_in: true,
                origin: ModuleOrigin::ThirdParty {
                    crate_name: "external-images".into(),
                },
            },
            state: state.clone(),
        }];
        let report = module_report(&reports, &switches);
        assert_eq!(report.rows[0].state, state);
        let text = render_plain(&report);
        assert!(text.contains(label), "{text}");
        assert!(text.contains("Make images"), "{text}");
        assert!(
            text.contains("tools.image_generation.enabled ← user config"),
            "{text}"
        );
        assert!(text.contains("third-party (external-images)"), "{text}");
    }
}
