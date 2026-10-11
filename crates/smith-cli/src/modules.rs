//! The explicit build catalog and resolved module composition.

#[cfg(any(feature = "module-image-generation", feature = "module-budget-notice"))]
use std::sync::Arc;

use smith_config::resolve::{KnownModule, ResolvedConfig};
#[cfg(any(
    not(feature = "module-image-generation"),
    not(feature = "module-budget-notice")
))]
use smith_module::ModuleDescriptor;
use smith_module::{CompiledModule, ModuleComposition, ModuleOrigin, SettingValue};

/// The single compiled-in list, with one feature gate per first-party module.
fn compiled_modules() -> Vec<CompiledModule> {
    Vec::from([
        #[cfg(feature = "module-image-generation")]
        CompiledModule {
            module: Arc::new(smith_module_image_generation::ImageGenerationModule),
            origin: ModuleOrigin::FirstParty,
        },
        #[cfg(feature = "module-budget-notice")]
        CompiledModule {
            module: Arc::new(smith_module_budget_notice::BudgetNoticeModule),
            origin: ModuleOrigin::FirstParty,
        },
    ])
}

pub(super) fn composition() -> ModuleComposition {
    let known = Vec::from([
        #[cfg(not(feature = "module-image-generation"))]
        ModuleDescriptor {
            id: "image-generation".into(),
            description: "Generate and edit images through the active provider".into(),
            default_enabled: true,
            origin: ModuleOrigin::FirstParty,
            compiled_in: false,
        },
        #[cfg(not(feature = "module-budget-notice"))]
        ModuleDescriptor {
            id: "budget-notice".into(),
            description: "Warn when conversation context nears the summary boundary".into(),
            default_enabled: true,
            origin: ModuleOrigin::FirstParty,
            compiled_in: false,
        },
    ]);
    ModuleComposition::with_defaults(compiled_modules(), known)
}

/// Configuration metadata comes from the same catalog used for mounting.
pub(super) fn known_modules() -> Vec<KnownModule> {
    let modules = composition();
    modules
        .compiled
        .iter()
        .map(CompiledModule::descriptor)
        .chain(modules.known)
        .map(|descriptor| KnownModule {
            legacy_enabled_key: (descriptor.id == "image-generation")
                .then(|| "tools.image_generation.enabled".into()),
            id: descriptor.id,
            default_enabled: descriptor.default_enabled,
            compiled_in: descriptor.compiled_in,
        })
        .collect()
}

/// Project effective switches and feature settings into the mount input.
pub(super) fn composition_from_resolved_config(config: &ResolvedConfig) -> ModuleComposition {
    let mut modules = composition();
    modules.enabled = config
        .modules
        .iter()
        .filter(|(_, module)| module.enabled.value)
        .map(|(id, _)| id.clone())
        .collect();
    modules.settings.insert(
        "image-generation".into(),
        [
            (
                "model".into(),
                SettingValue::String(config.image_generation.model.value.clone()),
            ),
            (
                "quality".into(),
                SettingValue::String(config.image_generation.quality.value.clone()),
            ),
            (
                "size".into(),
                SettingValue::String(config.image_generation.size.value.clone()),
            ),
        ]
        .into(),
    );
    modules
}

mod switch;
pub(crate) use switch::{commit_switch, prepare_switch, switch_outcome};

#[cfg(test)]
pub(crate) mod tests;
