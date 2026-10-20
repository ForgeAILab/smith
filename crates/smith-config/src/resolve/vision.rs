//! Image-input metadata before runtime defaults, and early vision validation.

use crate::catalog::{
    CatalogBinding, CatalogModality, CatalogModel, CatalogSnapshot, modality_catalog_provider_for,
};
use crate::model::KIND_COMMAND_JSONL;

use super::load::join_key;
use super::provenance::*;
use super::provider::{list, text};
use super::types::*;

/// Determines image-input support without filling in missing metadata.
///
/// An explicit field replaces the catalog field, including an empty override.
/// An empty catalog field means metadata is unknown. Command providers always
/// require text, even when an override or catalog advertises images.
pub fn image_input(
    explicit: Option<&[CatalogModality]>,
    catalog: Option<&CatalogModel>,
    provider_kind: &str,
) -> ImageInput {
    if provider_kind == KIND_COMMAND_JSONL {
        return ImageInput::TextOnly;
    }
    let modalities = explicit.or_else(|| {
        catalog
            .filter(|model| !model.input_modalities.is_empty())
            .map(|model| model.input_modalities.as_slice())
    });
    match modalities {
        Some(modalities) if modalities.contains(&CatalogModality::Image) => ImageInput::Accepts,
        Some(_) => ImageInput::TextOnly,
        None => ImageInput::Unknown,
    }
}

pub(super) struct ModelInput {
    pub(super) image: Sourced<ImageInput>,
    pub(super) modalities: Option<Sourced<Vec<CatalogModality>>>,
    catalog: Option<Sourced<Vec<CatalogModality>>>,
}

pub(super) fn model_input(
    provenance: &Provenance,
    provider: &str,
    model: &str,
    catalog: Option<&CatalogSnapshot>,
) -> Result<ModelInput, ConfigError> {
    let scope = join_key(&["providers", provider]);
    let kind = text(provenance, &format!("{scope}.kind"))?;
    let kind_value = kind.as_ref().map_or("", |kind| kind.value.as_str());
    if kind_value == KIND_COMMAND_JSONL {
        let source = kind.expect("command kind is present").source;
        return Ok(ModelInput {
            image: Sourced::new(ImageInput::TextOnly, source.clone()),
            modalities: Some(Sourced::new(vec![CatalogModality::Text], source)),
            catalog: None,
        });
    }
    let key = format!(
        "{}.input_modalities",
        join_key(&["models", &format!("{provider}/{model}")])
    );
    let explicit = list(provenance, &key)?
        .map(|raw| {
            let modalities = raw
                .value
                .iter()
                .map(|value| match value.as_str() {
                    "text" => Ok(CatalogModality::Text),
                    "image" => Ok(CatalogModality::Image),
                    "audio" => Ok(CatalogModality::Audio),
                    "video" => Ok(CatalogModality::Video),
                    "document" => Ok(CatalogModality::Document),
                    _ => Err(ConfigError::InvalidValue {
                        source: raw.source.clone(),
                        message: "input modalities must be text, image, audio, video, or document"
                            .to_owned(),
                    }),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, ConfigError>(Sourced::new(modalities, raw.source))
        })
        .transpose()?;
    let base_url = text(provenance, &format!("{scope}.base_url"))?;
    let endpoint = base_url.as_ref().map(|url| url.value.as_str());
    let binding = catalog.and_then(|snapshot| {
        snapshot
            .bound_provider(provider, kind_value, endpoint)
            .or_else(|| {
                // These reviewed modality-only bindings intentionally inherit no
                // API prices, limits, or inventory (for example ChatGPT).
                let public_provider = modality_catalog_provider_for(kind_value, endpoint)?;
                Some(CatalogBinding {
                    provider: snapshot.provider(public_provider)?,
                    partial_limits: None,
                    unadvertised_capabilities: None,
                    source: "models.dev",
                    revision: snapshot.revision(),
                    retrieved_at_ms: snapshot.retrieved_at_ms,
                })
            })
    });
    let catalog_model = binding.and_then(|binding| binding.provider.models.get(model));
    let image = image_input(
        explicit.as_ref().map(|value| value.value.as_slice()),
        catalog_model,
        kind_value,
    );
    let catalog_modalities = catalog_model
        .filter(|entry| !entry.input_modalities.is_empty())
        .zip(binding)
        .map(|(entry, binding)| {
            Sourced::new(
                entry.input_modalities.clone(),
                Source::built_in(format!(
                    "catalog {}@{} source={} retrieved_at_ms={} models.\"{provider}/{model}\".input_modalities",
                    binding.provider.id, binding.revision, binding.source, binding.retrieved_at_ms,
                )),
            )
        });
    let modalities = explicit.or_else(|| catalog_modalities.clone());
    let source = modalities.as_ref().map_or_else(
        || Source::built_in(format!("{key}: no image-input metadata")),
        |value| value.source.clone(),
    );
    Ok(ModelInput {
        image: Sourced::new(image, source),
        modalities,
        catalog: catalog_modalities,
    })
}

/// Projects the active model's metadata into the explanation ledger.
pub(super) fn explain_model_input(
    provenance: &mut Provenance,
    catalog: Option<&CatalogSnapshot>,
) -> Result<(), ConfigError> {
    let (Some(provider), Some(model)) = (text(provenance, "provider")?, text(provenance, "model")?)
    else {
        return Ok(());
    };
    let input = model_input(provenance, &provider.value, &model.value, catalog)?;
    let key = format!(
        "{}.input_modalities",
        join_key(&["models", &format!("{}/{}", provider.value, model.value)])
    );
    if let Some(catalog) = input.catalog {
        provenance.prepend(Contribution {
            key: key.clone(),
            value: modalities_value(&catalog.value),
            source: catalog.source,
        });
    }
    if let Some(modalities) = input.modalities {
        let value = modalities_value(&modalities.value);
        // Catalog metadata is lower precedence than every explicit field.
        // Command providers instead override every configured modality.
        let contribution = Contribution {
            key: key.clone(),
            value,
            source: modalities.source,
        };
        if input.image.source.key.ends_with(".kind") {
            provenance.extend(vec![contribution]);
        } else if provenance.winner(&key).is_none() {
            provenance.prepend(contribution);
        }
        provenance.mirror(&key, "input_modalities");
    } else {
        provenance.extend(vec![Contribution {
            key: "input_modalities".to_owned(),
            value: SettingValue::Text("unknown (no image-input metadata)".to_owned()),
            source: input.image.source,
        }]);
    }
    Ok(())
}

fn modalities_value(modalities: &[CatalogModality]) -> SettingValue {
    SettingValue::List(
        modalities
            .iter()
            .map(|modality| {
                match modality {
                    CatalogModality::Text => "text",
                    CatalogModality::Image => "image",
                    CatalogModality::Audio => "audio",
                    CatalogModality::Video => "video",
                    CatalogModality::Document => "document",
                }
                .to_owned()
            })
            .collect(),
    )
}

/// Uses the same frozen metadata and profile bindings as vision validation.
pub(super) fn image_capable_profiles(
    agent: &ResolvedAgent,
    global: &Provenance,
    catalog: Option<&CatalogSnapshot>,
) -> Result<Vec<String>, ConfigError> {
    let mut candidates = Vec::new();
    for name in &agent.profile_order.value {
        if *name == agent.profile.name {
            continue;
        }
        let Some(profile) = agent
            .profiles
            .get(name)
            .filter(|p| p.supports(crate::model::ProfileUse::Main))
        else {
            continue;
        };
        let provider = profile.provider.clone().or(text(global, "provider")?);
        let model = profile.model.clone().or(text(global, "model")?);
        if let (Some(provider), Some(model)) = (provider, model)
            && crate::cli_agents::parse_cli_model_id(&model.value).is_none()
            && model_input(global, &provider.value, &model.value, catalog)?
                .image
                .value
                == ImageInput::Accepts
        {
            candidates.push(name.clone());
        }
    }
    Ok(candidates)
}
