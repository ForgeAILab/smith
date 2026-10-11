use std::collections::BTreeMap;

use smith_config::resolve::{Layer, ResolvedModule, Source, Sourced};
use smith_module::{ModuleDescriptor, ModuleReport};

use super::{ModuleBlockReason, ModuleOrigin, ModuleState, module_report, render_plain};

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
