use super::*;

/// Builds one immutable cached-remote source scoped to a configured provider.
pub fn runtime_catalog_source(
    snapshot: &CatalogSnapshot,
    local_provider: &str,
    kind: &str,
    base_url: Option<&str>,
) -> Option<Arc<dyn ModelCatalogSource>> {
    let catalog_provider = catalog_provider_for(kind, base_url)?;
    let provider = snapshot.provider(catalog_provider)?;
    let mut source =
        StaticSource::new("models.dev", CatalogSource::CachedRemote).for_provider(local_provider);
    for model in provider.models.values().filter(|model| {
        model.disabled_reason.is_none()
            && model.tool_call
            && model.has_text_output()
            && model.limits.is_some()
    }) {
        let limits = model.limits.expect("filtered above");
        let mut record = ModelRecord::new()
            .with_limits(ModelLimits::new(
                limits.context_tokens,
                limits.max_input_tokens,
                limits.max_output_tokens,
            ))
            .with_capabilities(Capabilities {
                streaming: true,
                tools: model.tool_call,
                reasoning: if model.reasoning {
                    ReasoningSupport::Fixed
                } else {
                    ReasoningSupport::Unsupported
                },
                structured_output: model.structured_output,
                usage: true,
                cache: false,
                // The catalog describes the model; the serving adapter
                // declares how it drives a prompt cache.
                prompt_cache: PromptCacheControl::None,
                // Cache behavior is intentionally left to the resolved
                // provider/model adapter.  A public model catalog may say
                // nothing about endpoint-specific retention, evidence, or
                // synthetic conformance and must not grant those capabilities
                // by accident.
                cache_contract: None,
                auth: AuthKind::ApiKey,
                continuation: false,
                max_output_tokens: Some(limits.max_output_tokens),
            })
            .with_revision(snapshot.source_revision.clone());
        record.retrieved = Some(Timestamp(snapshot.retrieved_at_ms));
        record.input_modalities = Some(
            model
                .input_modalities
                .iter()
                .copied()
                .map(runtime_modality)
                .collect(),
        );
        record.output_modalities = Some(
            model
                .output_modalities
                .iter()
                .copied()
                .map(runtime_modality)
                .collect(),
        );
        source = source.with_model(&model.id, record);
    }
    Some(Arc::new(source))
}

fn runtime_modality(modality: CatalogModality) -> Modality {
    match modality {
        CatalogModality::Text => Modality::Text,
        CatalogModality::Image => Modality::Image,
        CatalogModality::Audio => Modality::Audio,
        CatalogModality::Video => Modality::Video,
        CatalogModality::Document => Modality::Document,
    }
}
