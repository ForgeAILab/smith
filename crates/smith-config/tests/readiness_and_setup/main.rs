//! First-run readiness and setup-data contracts.

use std::collections::BTreeMap;
use std::fs;

use smith_config::catalog::{
    CATALOG_SCHEMA_REVISION, CatalogLimits, CatalogModality, CatalogModel, CatalogProvider,
    CatalogSnapshot, MODELS_DEV_SOURCE_URL,
};
use smith_config::inventory::{
    ModelLimitOrigin, ModelSelectionError, local_inventory, local_inventory_with_catalog,
};
use smith_config::model::{
    ConfigFile, ConfigSecret, ContextSection, ModelSection, ProfileSection, ProviderSection,
    ReasoningOnlyBehavior,
};
use smith_config::output_budget::OutputBudgetOrigin;
use smith_config::resolve::{
    ConfigError, ConfigReadiness, Layer, Overrides, ResolveRequest, inspect, resolve,
};
use smith_config::user_config::{UserConfigEditError, prepare_user_config_edit};

const READY: &str = r#"
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

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().expect("a home"),
            project: tempfile::tempdir().expect("a project"),
        }
    }

    fn request(&self) -> ResolveRequest {
        ResolveRequest::new(self.project.path()).with_home_dir(self.home.path())
    }

    fn write_user(&self, text: &str) {
        let directory = self.home.path().join(".smith");
        fs::create_dir_all(&directory).expect("a user config directory");
        fs::write(directory.join("config.toml"), text).expect("a user config");
    }

    fn write_project(&self, text: &str) {
        let directory = self.project.path().join(".smith");
        fs::create_dir_all(&directory).expect("a project config directory");
        fs::write(directory.join("config.toml"), text).expect("a project config");
    }
}

fn glm_patch(credential: &str) -> ConfigFile {
    ConfigFile {
        default_profile: Some("glm".into()),
        profiles: BTreeMap::from([(
            "glm".into(),
            ProfileSection {
                provider: Some("zai".into()),
                model: Some("glm-4.7".into()),
                max_output_tokens: Some(8_192),
                ..ProfileSection::default()
            },
        )]),
        providers: BTreeMap::from([(
            "zai".into(),
            ProviderSection {
                kind: Some("openai-compatible".into()),
                base_url: Some("https://api.z.ai/api/coding/paas/v4".into()),
                credential: Some(credential.into()),
                ..ProviderSection::default()
            },
        )]),
        models: BTreeMap::from([(
            "zai/glm-4.7".into(),
            ModelSection {
                context_tokens: Some(200_000),
                max_input_tokens: Some(196_000),
                max_output_tokens: Some(131_072),
                ..ModelSection::default()
            },
        )]),
        context: Some(ContextSection {
            output_reserve: Some(8_192),
            ..ContextSection::default()
        }),
        ..ConfigFile::default()
    }
}

fn catalog_snapshot() -> CatalogSnapshot {
    let valid = |id: &str, name: &str, context: u32, input: u32, output: u32| CatalogModel {
        id: id.to_owned(),
        name: name.to_owned(),
        limits: Some(CatalogLimits {
            context_tokens: context,
            max_input_tokens: input,
            max_output_tokens: output,
        }),
        input_modalities: vec![CatalogModality::Text],
        output_modalities: vec![CatalogModality::Text],
        tool_call: true,
        reasoning: true,
        reasoning_controls: None,
        structured_output: true,
        cost: None,
        disabled_reason: None,
    };
    let mut openrouter_models = BTreeMap::from([
        (
            "vendor/model".to_owned(),
            valid("vendor/model", "Vendor Model", 128_000, 100_000, 16_000),
        ),
        (
            "no-tools".to_owned(),
            CatalogModel {
                tool_call: false,
                ..valid("no-tools", "No Tools", 32_000, 32_000, 4_000)
            },
        ),
    ]);
    openrouter_models.insert(
        "invalid-limits".to_owned(),
        CatalogModel {
            id: "invalid-limits".to_owned(),
            name: "Invalid Limits".to_owned(),
            limits: None,
            input_modalities: vec![CatalogModality::Text],
            output_modalities: vec![CatalogModality::Text],
            tool_call: true,
            reasoning: false,
            reasoning_controls: None,
            structured_output: false,
            cost: None,
            disabled_reason: Some("catalog output limit exceeds its context window".to_owned()),
        },
    );
    CatalogSnapshot {
        schema_revision: CATALOG_SCHEMA_REVISION,
        source_url: MODELS_DEV_SOURCE_URL.to_owned(),
        source_digest: format!("sha256:{}", "1".repeat(64)),
        content_digest: format!("sha256:{}", "2".repeat(64)),
        source_revision: "fixture-r1".to_owned(),
        retrieved_at_ms: 1_000,
        providers: BTreeMap::from([
            (
                "openrouter".to_owned(),
                CatalogProvider {
                    id: "openrouter".to_owned(),
                    name: "OpenRouter".to_owned(),
                    models: openrouter_models,
                },
            ),
            (
                "zai-coding-plan".to_owned(),
                CatalogProvider {
                    id: "zai-coding-plan".to_owned(),
                    name: "Z.AI Coding Plan".to_owned(),
                    models: BTreeMap::from([(
                        "glm-next".to_owned(),
                        valid("glm-next", "GLM Next", 200_000, 180_000, 64_000),
                    )]),
                },
            ),
        ]),
    }
}

mod inventory;
mod readiness;
mod response_policy;
mod trusted_providers;
mod user_edits;
mod windows;
