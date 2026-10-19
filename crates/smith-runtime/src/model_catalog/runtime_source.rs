use super::*;

/// Builds one immutable cached-remote source scoped to a configured provider.
pub fn runtime_catalog_source(
    snapshot: &CatalogSnapshot,
    local_provider: &str,
    kind: &str,
    base_url: Option<&str>,
) -> Option<Arc<dyn ModelCatalogSource>> {
    let binding = snapshot.bound_provider(local_provider, kind, base_url)?;
    let provider = binding.provider;
    let mut source =
        StaticSource::new(binding.source, CatalogSource::CachedRemote).for_provider(local_provider);
    for model in provider
        .models
        .values()
        .filter(|model| model.tool_call && model.has_text_output())
    {
        let mut record = ModelRecord::new();
        if let Some(limits) = model.limits {
            record = record.with_limits(ModelLimits::new(
                limits.context_tokens,
                limits.max_input_tokens,
                limits.max_output_tokens,
            ));
        } else if let Some(limits) = binding
            .partial_limits
            .and_then(|models| models.get(&model.id))
        {
            // Partial endpoint fields can complement an explicit override;
            // preflight still rejects a profile without complete safe limits.
            record.context_tokens = limits.context_tokens;
            record.max_input_tokens = limits.max_input_tokens;
            record.max_output_tokens = limits.max_output_tokens;
        } else {
            continue;
        }
        let output_ceiling = record.max_output_tokens;
        record = record
            .with_capabilities(Capabilities {
                streaming: true,
                tools: model.tool_call,
                hosted_web_search: false,
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
                max_output_tokens: output_ceiling,
            })
            .with_revision(binding.revision.to_owned());
        record.retrieved = Some(Timestamp(binding.retrieved_at_ms));
        record.input_modalities = (!model.input_modalities.is_empty()).then(|| {
            model
                .input_modalities
                .iter()
                .copied()
                .map(runtime_modality)
                .collect()
        });
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

pub(crate) fn runtime_modality(modality: CatalogModality) -> Modality {
    match modality {
        CatalogModality::Text => Modality::Text,
        CatalogModality::Image => Modality::Image,
        CatalogModality::Audio => Modality::Audio,
        CatalogModality::Video => Modality::Video,
        CatalogModality::Document => Modality::Document,
    }
}
