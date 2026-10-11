use super::{
    CompiledModule, ModuleOrigin, SettingValue, composition, composition_from_resolved_config,
    known_modules,
};

#[test]
fn catalog_contains_each_port_even_when_its_feature_is_omitted() {
    let composition = composition();
    let descriptors = composition
        .compiled
        .iter()
        .map(CompiledModule::descriptor)
        .chain(composition.known.iter().cloned())
        .collect::<Vec<_>>();
    assert_eq!(descriptors.len(), 2);
    for (id, compiled_in) in [
        (
            "image-generation",
            cfg!(feature = "module-image-generation"),
        ),
        ("budget-notice", cfg!(feature = "module-budget-notice")),
    ] {
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.id == id)
            .unwrap();
        assert_eq!(descriptor.compiled_in, compiled_in);
        assert_eq!(descriptor.origin, ModuleOrigin::FirstParty);
    }
}

/// Compares the actual returned pipeline values using Runtime's own sealer.
#[cfg(feature = "module-budget-notice")]
pub(crate) fn pipeline_fingerprint(
    runtime: &smith_runtime::factory::SmithRuntime,
) -> agent_runtime::registry::Fingerprint {
    let mut pipeline = agent_runtime::harness::HarnessPipelineBuilder::new();
    for contribution in runtime
        .mounted_modules()
        .iter()
        .flat_map(|module| &module.contributions)
    {
        if let smith_module::ModuleContribution::Pipeline(component) = contribution {
            match component {
                smith_module::PipelineComponent::HistoryProjector(component) => {
                    pipeline.history_projector(component.clone());
                }
                smith_module::PipelineComponent::ContextContributor(component) => {
                    pipeline.context_contributor(component.clone());
                }
                smith_module::PipelineComponent::ToolViewResolver(component) => {
                    pipeline.tool_view_resolver(component.clone());
                }
                smith_module::PipelineComponent::ModelInterceptor(component) => {
                    pipeline.model_interceptor(component.clone());
                }
                smith_module::PipelineComponent::ToolOutputProcessor(component) => {
                    pipeline.tool_output_processor(component.clone());
                }
                smith_module::PipelineComponent::TurnCommitHook(component) => {
                    pipeline.turn_commit_hook(component.clone());
                }
            }
        }
    }
    pipeline
        .seal()
        .expect("module pipeline")
        .fingerprint()
        .clone()
}

#[test]
fn resolved_switches_and_image_settings_determine_the_composition() {
    let project = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join(".smith")).unwrap();
    std::fs::write(
        project.path().join(".smith/config.toml"),
        r#"
default_profile = "work"
[profiles.work]
provider = "local"
model = "example-model"
[providers.local]
kind = "fake"
[models."local/example-model"]
context_tokens = 32768
max_input_tokens = 24000
max_output_tokens = 4096
[modules.image-generation]
enabled = true
[modules.budget-notice]
enabled = false
[tools.image_generation]
model = "custom-image"
quality = "high"
size = "1024x1024"
"#,
    )
    .unwrap();
    let resolution = smith_config::resolve::resolve(
        &smith_config::resolve::ResolveRequest::new(project.path())
            .with_home_dir(home.path())
            .with_known_modules(known_modules()),
    )
    .unwrap();
    let composition = composition_from_resolved_config(&resolution.config);
    assert_eq!(composition.enabled, ["image-generation".into()].into());
    assert_eq!(
        composition.settings["image-generation"]["model"],
        SettingValue::String("custom-image".into())
    );
    assert_eq!(
        composition.settings["image-generation"]["quality"],
        SettingValue::String("high".into())
    );
    assert_eq!(
        composition.settings["image-generation"]["size"],
        SettingValue::String("1024x1024".into())
    );
    for module in known_modules() {
        assert_eq!(
            module.legacy_enabled_key.is_some(),
            module.id == "image-generation"
        );
        assert_eq!(
            module.compiled_in,
            composition
                .compiled
                .iter()
                .any(|entry| entry.module.id() == module.id)
        );
    }
}
