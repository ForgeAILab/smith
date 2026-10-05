use super::{
    AVAILABLE_ADAPTER_KINDS, BTreeMap, Context, KIND_ANTHROPIC_MESSAGES, ProviderSetupDescriptor,
    ProviderSetupFlow, QuickKeySetup, ResourceEntry, Result, SETUP_ENVIRONMENT_VARIABLE_ERROR,
    SETUP_PROVIDER_NAME_HELP, SelectionInventory, SetupContext, SetupEntry, SetupFlow,
    SetupKeyReview, SetupMode, SetupModelLimits, SetupPrompts, SetupProviderKind, SetupQuickKey,
    SetupQuickStart, compact_tokens, plural, provider_descriptors, setup_endpoint_help,
};

pub(super) fn catalog_model_entries(
    context: &SetupContext,
    catalog_provider: &str,
    label: &str,
) -> Result<(Vec<ResourceEntry>, BTreeMap<String, SetupModelLimits>)> {
    let provider = context
        .catalog
        .provider(catalog_provider)
        .with_context(|| format!("the Smith model catalog has no {label} descriptor"))?;
    let mut limits_by_model = BTreeMap::new();
    let mut entries = Vec::new();
    for model in provider.models.values().filter(|model| {
        model.disabled_reason.is_none()
            && model.tool_call
            && model.has_text_output()
            && model.limits.is_some()
    }) {
        let limits = model.limits.expect("the model was filtered for limits");
        limits_by_model.insert(
            model.id.clone(),
            SetupModelLimits {
                context_tokens: limits.context_tokens,
                max_input_tokens: limits.max_input_tokens,
                max_output_tokens: limits.max_output_tokens,
            },
        );
        entries.push(ResourceEntry::new(
            model.id.clone(),
            model.name.clone(),
            format!(
                "{} · {} context · {} input · {} output{}",
                model.id,
                compact_tokens(u64::from(limits.context_tokens)),
                compact_tokens(u64::from(limits.max_input_tokens)),
                compact_tokens(u64::from(limits.max_output_tokens)),
                if model.reasoning { " · reasoning" } else { "" }
            ),
        ));
    }
    if entries.is_empty() {
        anyhow::bail!("the {label} model catalog has no tool-capable text model with limits");
    }
    Ok((entries, limits_by_model))
}

pub(crate) fn setup_prompts() -> SetupPrompts {
    SetupPrompts {
        provider_name_help: SETUP_PROVIDER_NAME_HELP.to_owned(),
        endpoint_help: setup_endpoint_help(),
        environment_variable_error: SETUP_ENVIRONMENT_VARIABLE_ERROR.to_owned(),
    }
}

pub(super) fn provider_action_entries() -> Vec<SetupEntry> {
    provider_descriptors(AVAILABLE_ADAPTER_KINDS)
        .into_iter()
        .map(|descriptor| SetupEntry {
            id: descriptor.setup_id.to_owned(),
            label: descriptor.label.to_owned(),
            detail: descriptor.description.to_owned(),
            flow: provider_setup_flow(descriptor, false),
        })
        .collect()
}

pub(crate) fn setup_action_entries(mode: &SetupMode) -> Vec<SetupEntry> {
    let mut entries = provider_action_entries();
    if matches!(mode, SetupMode::Menu) {
        entries.push(SetupEntry {
            id: "add-model".into(),
            label: "Add model".into(),
            detail: "attach explicit limits to an existing provider".into(),
            flow: SetupFlow::AddModel,
        });
        entries.push(SetupEntry {
            id: "change-default".into(),
            label: "Change default".into(),
            detail: "choose a configured provider/model pair".into(),
            flow: SetupFlow::ChangeDefault,
        });
    }
    entries
}

pub(crate) fn provider_setup_flow(
    descriptor: ProviderSetupDescriptor,
    catalog_models: bool,
) -> SetupFlow {
    match descriptor.flow {
        ProviderSetupFlow::QuickKey(kind) => {
            let provider = descriptor
                .provider
                .expect("a quick key plan has a fixed provider");
            let endpoint = descriptor
                .endpoint
                .expect("a quick key plan has a fixed endpoint");
            SetupFlow::QuickKey {
                kind: match kind {
                    QuickKeySetup::Glm => SetupQuickKey::Glm,
                    QuickKeySetup::Xai => SetupQuickKey::Xai,
                    QuickKeySetup::Google => SetupQuickKey::Google,
                },
                provider: provider.to_owned(),
                endpoint: endpoint.to_owned(),
                review: SetupKeyReview {
                    action: format!("action: {}", descriptor.review.action),
                    provider: format!("provider: {provider} ({})", descriptor.review.adapter),
                    endpoint: format!(
                        "endpoint: {}",
                        descriptor.review.endpoint.unwrap_or(endpoint)
                    ),
                    profile: format!(
                        "default profile: {}",
                        descriptor.profile.expect("a quick key plan has a profile")
                    ),
                    reasoning: descriptor
                        .review
                        .reasoning
                        .map(|reasoning| format!("reasoning: {reasoning}")),
                },
                catalog_models,
            }
        }
        ProviderSetupFlow::CustomEndpoint => SetupFlow::CustomEndpoint {
            kind: if descriptor.adapter == KIND_ANTHROPIC_MESSAGES {
                SetupProviderKind::AnthropicMessages
            } else {
                SetupProviderKind::OpenAiCompatible
            },
            provider: descriptor.provider.map(str::to_owned),
            endpoint: descriptor.endpoint.map(str::to_owned),
            review_action: format!("action: {}", descriptor.review.action),
            adapter: descriptor.adapter.to_owned(),
            catalog_models,
        },
        ProviderSetupFlow::OAuth { busy_note } => SetupFlow::OAuth {
            busy_note: busy_note.to_owned(),
        },
    }
}

pub(crate) fn glm_quick_start() -> SetupQuickStart {
    let descriptor = provider_descriptors(AVAILABLE_ADAPTER_KINDS)
        .into_iter()
        .find(|descriptor| descriptor.flow == ProviderSetupFlow::QuickKey(QuickKeySetup::Glm))
        .expect("this build supplies the trusted quick-start descriptor");
    let model = descriptor
        .models
        .first()
        .expect("the quick start has a trusted model");
    SetupQuickStart {
        provider: descriptor
            .provider
            .expect("the quick start has a provider")
            .into(),
        endpoint: descriptor
            .endpoint
            .expect("the quick start has an endpoint")
            .into(),
        model: model.model.into(),
        model_label: model.label.into(),
        limits: SetupModelLimits {
            context_tokens: model.context_tokens,
            max_input_tokens: model.max_input_tokens,
            max_output_tokens: model.max_output_tokens,
        },
        request_output_tokens: model.request_output_tokens,
        output_reserve: model.output_reserve,
        profile: descriptor
            .profile
            .expect("the quick start has a profile")
            .into(),
        catalog_revision: model.revision,
    }
}

pub(super) fn provider_entries(inventory: &SelectionInventory) -> Vec<ResourceEntry> {
    inventory
        .providers
        .iter()
        .map(|provider| {
            let entry = ResourceEntry::new(
                provider.name.clone(),
                provider.name.clone(),
                format!(
                    "{} · {}",
                    provider.kind.as_deref().unwrap_or("unknown adapter"),
                    plural(provider.model_count, "model", "models")
                ),
            )
            .active(provider.active);
            if provider.adapter_available {
                entry
            } else {
                entry.disabled("provider adapter or declaration is unavailable")
            }
        })
        .collect()
}

pub(super) fn model_entries(inventory: &SelectionInventory) -> Vec<ResourceEntry> {
    inventory
        .models
        .iter()
        .map(|model| {
            let profiles = if model.profiles.is_empty() {
                "no profile".to_owned()
            } else {
                format!("profiles: {}", model.profiles.join(", "))
            };
            ResourceEntry::new(model.id(), model.id(), profiles).active(model.active)
        })
        .collect()
}
