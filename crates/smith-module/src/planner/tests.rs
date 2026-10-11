use std::sync::{Arc, Mutex};

use agent_runtime_testkit::transport::ReplayTransport;

use super::*;
use crate::{
    Module, ModuleError, ModuleOrigin, ModulePosture, ModuleSettings, SettingValue, SlashCommand,
};

#[derive(Debug)]
struct Fixture {
    id: &'static str,
    requires: &'static [&'static str],
    outcome: Result<Mounted, ModuleError>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl Module for Fixture {
    fn id(&self) -> &str {
        self.id
    }
    fn revision(&self) -> &str {
        "1"
    }
    fn description(&self) -> &str {
        "fixture module"
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn requirements(&self) -> &[&str] {
        self.requires
    }
    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError> {
        self.calls.lock().unwrap().push(self.id.into());
        if self.id == "settings" {
            assert_eq!(
                context.settings,
                ModuleSettings::from([(
                    "model".into(),
                    SettingValue::String("gpt-image-2".into())
                ),])
            );
        }
        self.outcome.clone()
    }
}

fn context() -> ModuleContext {
    ModuleContext {
        settings: ModuleSettings::new(),
        user_dir: "user".into(),
        posture: ModulePosture::ReadWrite,
        transport: Arc::new(ReplayTransport::single(Vec::new())),
        image_binding: None,
        session_history: None,
        semantic_summary_enabled: false,
        max_input_tokens: 32_768,
        built_in_tools: true,
    }
}

fn fixture(
    id: &'static str,
    requires: &'static [&'static str],
    calls: &Arc<Mutex<Vec<String>>>,
) -> CompiledModule {
    CompiledModule {
        module: Arc::new(Fixture {
            id,
            requires,
            outcome: Ok(Mounted::Contributions(vec![ModuleContribution::Command(
                SlashCommand {
                    name: id.into(),
                    description: "fixture command".into(),
                },
            )])),
            calls: calls.clone(),
        }),
        origin: ModuleOrigin::FirstParty,
    }
}

fn state<'a>(plan: &'a MountPlan, id: &str) -> &'a ModuleState {
    &plan
        .report
        .iter()
        .find(|report| report.descriptor.id == id)
        .unwrap()
        .state
}

#[test]
fn mounts_in_requirement_order_independent_of_list_order() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let a = fixture("a", &["z"], &calls);
    let z = fixture("z", &[], &calls);
    let forward = mount_modules(
        &ModuleComposition::with_defaults(vec![a.clone(), z.clone()], vec![]),
        &context(),
    );
    let reverse = mount_modules(
        &ModuleComposition::with_defaults(vec![z, a], vec![]),
        &context(),
    );
    assert_eq!(*calls.lock().unwrap(), ["z", "a", "z", "a"]);
    assert_eq!(forward.report, reverse.report);
    assert_eq!(
        forward
            .mounted
            .iter()
            .map(|module| module.descriptor.id.as_str())
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
}

#[test]
fn absent_off_and_not_built_requirements_block_mounting() {
    for unavailable in ["absent", "off", "not-built"] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut composition =
            ModuleComposition::with_defaults(vec![fixture("b", &["a"], &calls)], vec![]);
        if unavailable == "off" {
            composition.compiled.push(fixture("a", &[], &calls));
        } else if unavailable == "not-built" {
            composition.known.push(ModuleDescriptor {
                id: "a".into(),
                description: "omitted module".into(),
                default_enabled: true,
                origin: ModuleOrigin::FirstParty,
                compiled_in: false,
            });
            composition.enabled.insert("a".into());
        }
        let plan = mount_modules(&composition, &context());
        assert!(plan.mounted.is_empty());
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(
            state(&plan, "b"),
            &ModuleState::Blocked {
                reason: ModuleBlockReason::Requirement("a".into())
            }
        );
        if unavailable == "off" {
            assert_eq!(state(&plan, "a"), &ModuleState::Off);
        } else if unavailable == "not-built" {
            assert_eq!(state(&plan, "a"), &ModuleState::NotBuilt);
        }
    }
}

#[test]
fn mount_failure_contributes_nothing_and_blocks_only_dependents() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let failed = CompiledModule {
        module: Arc::new(Fixture {
            id: "a",
            requires: &[],
            outcome: Err(ModuleError("fixture failure".into())),
            calls: calls.clone(),
        }),
        origin: ModuleOrigin::ThirdParty {
            crate_name: "fixture-crate".into(),
        },
    };
    let plan = mount_modules(
        &ModuleComposition::with_defaults(
            vec![
                failed,
                fixture("b", &["a"], &calls),
                fixture("c", &["b"], &calls),
                fixture("independent", &[], &calls),
            ],
            vec![],
        ),
        &context(),
    );
    assert_eq!(*calls.lock().unwrap(), ["a", "independent"]);
    assert_eq!(plan.mounted.len(), 1);
    assert_eq!(
        state(&plan, "a"),
        &ModuleState::Failed {
            reason: "fixture failure".into()
        }
    );
    assert_eq!(
        state(&plan, "b"),
        &ModuleState::Blocked {
            reason: ModuleBlockReason::Requirement("a".into())
        }
    );
    assert_eq!(
        state(&plan, "c"),
        &ModuleState::Blocked {
            reason: ModuleBlockReason::Requirement("b".into())
        }
    );
}

#[test]
fn exact_cycle_members_are_reported_and_downstream_is_blocked() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let plan = mount_modules(
        &ModuleComposition::with_defaults(
            vec![
                fixture("a", &["b"], &calls),
                fixture("b", &["a", "c"], &calls),
                fixture("c", &["b"], &calls),
                fixture("downstream", &["a"], &calls),
                fixture("independent", &[], &calls),
            ],
            vec![],
        ),
        &context(),
    );
    for id in ["a", "b", "c"] {
        assert_eq!(
            state(&plan, id),
            &ModuleState::Blocked {
                reason: ModuleBlockReason::Cycle(vec!["a".into(), "b".into(), "c".into()])
            }
        );
    }
    assert_eq!(
        state(&plan, "downstream"),
        &ModuleState::Blocked {
            reason: ModuleBlockReason::Requirement("a".into())
        }
    );
    assert_eq!(*calls.lock().unwrap(), ["independent"]);
}

#[test]
fn self_cycle_never_mounts() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let plan = mount_modules(
        &ModuleComposition::with_defaults(vec![fixture("self", &["self"], &calls)], vec![]),
        &context(),
    );
    assert!(plan.mounted.is_empty());
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(
        state(&plan, "self"),
        &ModuleState::Blocked {
            reason: ModuleBlockReason::Cycle(vec!["self".into()])
        }
    );
}

#[test]
fn conditional_inactivity_explains_why_no_values_mounted() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let module = CompiledModule {
        module: Arc::new(Fixture {
            id: "conditional",
            requires: &[],
            calls: calls.clone(),
            outcome: Ok(Mounted::Inactive {
                reason: "semantic summary is disabled".into(),
            }),
        }),
        origin: ModuleOrigin::FirstParty,
    };
    let plan = mount_modules(
        &ModuleComposition::with_defaults(
            vec![module, fixture("dependent", &["conditional"], &calls)],
            vec![],
        ),
        &context(),
    );
    assert!(plan.mounted.is_empty());
    assert_eq!(
        state(&plan, "conditional"),
        &ModuleState::Inactive {
            reason: "semantic summary is disabled".into()
        }
    );
    assert_eq!(
        state(&plan, "dependent"),
        &ModuleState::Blocked {
            reason: ModuleBlockReason::Requirement("conditional".into())
        }
    );
}

#[test]
fn settings_are_scoped_to_the_mount_and_duplicate_ids_never_mount() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let duplicate = fixture("duplicate", &[], &calls);
    let mut composition = ModuleComposition::with_defaults(
        vec![
            fixture("settings", &[], &calls),
            duplicate.clone(),
            duplicate,
        ],
        vec![],
    );
    composition.settings.insert(
        "settings".into(),
        ModuleSettings::from([("model".into(), SettingValue::String("gpt-image-2".into()))]),
    );
    let plan = mount_modules(&composition, &context());
    assert_eq!(*calls.lock().unwrap(), ["settings"]);
    assert!(matches!(
        state(&plan, "duplicate"),
        ModuleState::Failed { .. }
    ));
    assert_eq!(state(&plan, "settings"), &ModuleState::Mounted);
}

#[test]
fn empty_composition_has_no_mounts_or_reports() {
    let plan = mount_modules(&ModuleComposition::default(), &context());
    assert!(plan.mounted.is_empty());
    assert!(plan.report.is_empty());
}
