//! Where a model's limits come from, and why.
//!
//! Agent Runtime refuses to plan a request without enforceable limits, and
//! Smith refuses to guess them. What a model's context window actually is has
//! to come from somewhere a user can point at, so this module composes the
//! sources Smith has into the shared [`LayeredModelCatalog`] and hands the
//! runtime a profile that knows its own provenance.
//!
//! The layers, highest precedence first:
//!
//! | Layer | What Smith puts there |
//! | --- | --- |
//! | [`CatalogSource::Explicit`] | `[models."<provider>/<model>"]`, a CLI flag, or a session override |
//! | [`CatalogSource::ProviderLocal`] | provider adapter introspection or provider-owned local metadata |
//! | [`CatalogSource::Embedded`] | known-good metadata shipped with Smith |
//! | [`CatalogSource::CachedRemote`] | a host-validated cache of remote metadata |
//!
//! Merging, the same-layer conflict rule, and the missing-limit failure belong
//! to the shared catalog rather than to this module. A second merge
//! implementation here would be a second place a wrong context window could
//! come from, which is the failure this layering exists to prevent.
//!
//! Smith adds the one thing the shared catalog cannot know. A
//! [`FieldProvenance`](agent_runtime_core::catalog::FieldProvenance) records
//! that a limit was `explicit`; only Smith can say that "explicit" was a
//! particular key in a particular file, and `smith config explain` needs both
//! halves. A [`ProfileResolution`] therefore keeps every layer that offered a
//! limit — including the ones that lost — each carrying its Smith
//! configuration source where it has one.
//!
//! Nothing *here* performs I/O. `CachedRemote` is now populated, but by
//! [`crate::model_catalog::CatalogLoader`] rather than by this module: it
//! reads a validated last-good cache under the user state root, falls back to
//! the embedded seed when that cache is absent or no longer parses, and
//! schedules a background refresh once a snapshot ages past
//! [`crate::model_catalog::DEFAULT_CATALOG_MAX_AGE_MS`]. Keeping the fetch out
//! of this module is what lets layer composition stay a pure function of
//! already-resolved inputs.

use std::collections::BTreeMap;
use std::sync::Arc;

use agent_runtime_core::catalog::{
    CatalogSource, LayeredModelCatalog, ModelCatalog, ModelCatalogSource, ModelProfileError,
    ModelRecord, ProfileField, ResolvedModelProfile, StaticSource,
};
use agent_runtime_core::provider::ModelId;
use smith_config::resolve::{ResolvedModelLimits, Source};

/// The source name Smith's resolved configuration contributes under.
pub const CONFIGURED_SOURCE: &str = "smith-config";

/// The source name a provider adapter's own metadata contributes under.
pub const PROVIDER_LOCAL_SOURCE: &str = "smith-provider-local";

/// The source name Smith's embedded known-good metadata contributes under.
pub const EMBEDDED_SOURCE: &str = "smith-embedded";

/// The source name a host-validated cache of remote metadata contributes under.
pub const CACHED_REMOTE_SOURCE: &str = "smith-cached-remote";

/// The fields a [`LimitContribution`] is recorded for.
///
/// Only the three enforcement limits: they are what a run cannot start without,
/// and what an operator asks "where did that number come from" about.
const LIMIT_FIELDS: [ProfileField; 3] = [
    ProfileField::ContextTokens,
    ProfileField::MaxInputTokens,
    ProfileField::MaxOutputTokens,
];

/// The catalog layers one run resolves its model profile through.
///
/// Built for exactly one provider/model pair, because that is the granularity
/// Smith configuration works at: `[models."<provider>/<model>"]` limits say
/// nothing about another provider serving a same-named model.
#[derive(Debug, Clone)]
pub struct CatalogLayers {
    provider: String,
    model: ModelId,
    sources: Vec<Arc<dyn ModelCatalogSource>>,
    configured: BTreeMap<ProfileField, Source>,
}

impl CatalogLayers {
    /// Empty layers for `model` as served by `provider`.
    pub fn new(provider: impl Into<String>, model: ModelId) -> Self {
        Self {
            provider: provider.into(),
            model,
            sources: Vec::new(),
            configured: BTreeMap::new(),
        }
    }

    /// Adds Smith's resolved configuration as the explicit layer.
    ///
    /// Each limit is independently optional and none is defaulted, so a
    /// configuration declaring two of the three contributes two fields and
    /// leaves the last to a lower layer. Configuration declaring none registers
    /// no source at all: an empty record would make the model *known* but
    /// unusable, replacing an "unknown model" diagnostic with a vaguer one.
    #[must_use]
    pub fn with_configured_limits(mut self, limits: &ResolvedModelLimits) -> Self {
        let mut record = ModelRecord::new();
        for (field, sourced) in [
            (ProfileField::ContextTokens, limits.context_tokens.as_ref()),
            (
                ProfileField::MaxInputTokens,
                limits.max_input_tokens.as_ref(),
            ),
            (
                ProfileField::MaxOutputTokens,
                limits.max_output_tokens.as_ref(),
            ),
        ] {
            let Some(sourced) = sourced else {
                continue;
            };
            set_limit(&mut record, field, sourced.value);
            self.configured.insert(field, sourced.source.clone());
        }

        if self.configured.is_empty() {
            return self;
        }
        self.push(CONFIGURED_SOURCE, CatalogSource::Explicit, record)
    }

    /// Adds a provider-local layer: adapter introspection, or metadata the
    /// provider itself publishes locally.
    #[must_use]
    pub fn with_provider_local(self, record: ModelRecord) -> Self {
        self.push(PROVIDER_LOCAL_SOURCE, CatalogSource::ProviderLocal, record)
    }

    /// Adds known-good metadata embedded in a Smith build.
    #[must_use]
    pub fn with_embedded(self, record: ModelRecord) -> Self {
        self.push(EMBEDDED_SOURCE, CatalogSource::Embedded, record)
    }

    /// Adds a record from a host-validated cache of remote metadata.
    ///
    /// Smith ships no fetcher for this layer. The seam exists so that adding
    /// one later means adding a caller here rather than a second precedence
    /// order somewhere else.
    #[must_use]
    pub fn with_cached_remote(self, record: ModelRecord) -> Self {
        self.push(CACHED_REMOTE_SOURCE, CatalogSource::CachedRemote, record)
    }

    /// Adds a prepared source, for a layer Smith does not build itself.
    #[must_use]
    pub fn with_source(mut self, source: Arc<dyn ModelCatalogSource>) -> Self {
        self.sources.push(source);
        self
    }

    /// Adds many prepared sources.
    #[must_use]
    pub fn with_sources(
        mut self,
        sources: impl IntoIterator<Item = Arc<dyn ModelCatalogSource>>,
    ) -> Self {
        self.sources.extend(sources);
        self
    }

    /// The provider these layers describe.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The model these layers describe.
    pub fn model(&self) -> &ModelId {
        &self.model
    }

    /// How many sources are registered.
    pub fn len(&self) -> usize {
        self.sources.len()
    }

    /// Whether no source is registered, in which case resolution fails with an
    /// unknown-model diagnostic.
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// The shared catalog these layers form.
    pub fn catalog(&self) -> LayeredModelCatalog {
        self.sources
            .iter()
            .fold(LayeredModelCatalog::new(), |catalog, source| {
                catalog.with_source(Arc::clone(source))
            })
    }

    /// Resolves the profile, or explains which limit no layer supplied.
    pub fn resolve(&self) -> Result<ProfileResolution, ModelProfileError> {
        let profile = self.catalog().resolve(&self.provider, &self.model)?;
        Ok(ProfileResolution {
            profile,
            contributions: self.contributions(),
        })
    }

    /// Registers `record` as a named source at `layer`, scoped to this provider
    /// so a record cannot answer for a provider the user switched to.
    fn push(mut self, name: &str, layer: CatalogSource, record: ModelRecord) -> Self {
        let source = StaticSource::new(name, layer)
            .for_provider(self.provider.clone())
            .with_model(self.model.as_str(), record);
        self.sources.push(Arc::new(source));
        self
    }

    /// Every layer's offer for each limit field, highest precedence first.
    fn contributions(&self) -> BTreeMap<ProfileField, Vec<LimitContribution>> {
        let mut out: BTreeMap<ProfileField, Vec<LimitContribution>> = BTreeMap::new();
        for source in &self.sources {
            let Some(record) = source.lookup(&self.provider, &self.model) else {
                continue;
            };
            for field in LIMIT_FIELDS {
                let Some(value) = limit(&record, field) else {
                    continue;
                };
                let configured = match source.name() {
                    CONFIGURED_SOURCE => self.configured.get(&field).cloned(),
                    _ => None,
                };
                out.entry(field).or_default().push(LimitContribution {
                    layer: source.source(),
                    name: source.name().to_owned(),
                    value,
                    configured,
                });
            }
        }

        for offers in out.values_mut() {
            offers.sort_by(|a, b| {
                b.layer
                    .precedence()
                    .cmp(&a.layer.precedence())
                    .then_with(|| a.name.cmp(&b.name))
            });
        }
        out
    }
}

/// One layer's offered value for one limit field.
///
/// Losing layers are kept as well as the winner: "the cached catalog said 128k
/// and your project config said 32k" is the answer a configuration diagnostic
/// needs, and the winning value alone cannot give it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitContribution {
    /// The catalog layer that offered it.
    pub layer: CatalogSource,
    /// The offering source's name.
    pub name: String,
    /// The value it offered.
    pub value: u32,
    /// The Smith configuration source behind it, for the configured layer.
    pub configured: Option<Source>,
}

/// A resolved profile together with the layers that produced its limits.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileResolution {
    /// The frozen profile the run plans every request against.
    pub profile: ResolvedModelProfile,
    /// Every layer's offer per limit field, highest precedence first.
    pub contributions: BTreeMap<ProfileField, Vec<LimitContribution>>,
}

impl ProfileResolution {
    /// Every layer that offered a value for `field`, highest precedence first.
    pub fn contributions_for(&self, field: ProfileField) -> &[LimitContribution] {
        self.contributions
            .get(&field)
            .map_or(&[], |offers| offers.as_slice())
    }

    /// The Smith configuration source behind `field`, when Smith configuration
    /// is what supplied it.
    pub fn configured_source(&self, field: ProfileField) -> Option<&Source> {
        self.contributions_for(field)
            .iter()
            .find_map(|offer| offer.configured.as_ref())
    }
}

/// Reads one limit field out of a record.
fn limit(record: &ModelRecord, field: ProfileField) -> Option<u32> {
    match field {
        ProfileField::ContextTokens => record.context_tokens,
        ProfileField::MaxInputTokens => record.max_input_tokens,
        ProfileField::MaxOutputTokens => record.max_output_tokens,
        _ => None,
    }
}

/// Writes one limit field into a record.
fn set_limit(record: &mut ModelRecord, field: ProfileField, value: u32) {
    match field {
        ProfileField::ContextTokens => record.context_tokens = Some(value),
        ProfileField::MaxInputTokens => record.max_input_tokens = Some(value),
        ProfileField::MaxOutputTokens => record.max_output_tokens = Some(value),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use agent_runtime_core::catalog::{ModelLimits, ModelProfileErrorKind};
    use smith_config::resolve::{Layer, Sourced};

    fn configured(value: u32, key: &str) -> Sourced<u32> {
        Sourced::new(
            value,
            Source::file(Layer::ProjectFile, "/repo/.smith/config.toml", key),
        )
    }

    fn layers() -> CatalogLayers {
        CatalogLayers::new("acme", ModelId::new("example-model"))
    }

    #[test]
    fn a_model_no_layer_declares_fails_instead_of_defaulting_a_window() {
        let err = layers().resolve().expect_err("an unknown model");
        assert_eq!(err.kind, ModelProfileErrorKind::UnknownModel);
    }

    #[test]
    fn configuration_that_declares_no_limit_registers_no_layer() {
        let layers = layers().with_configured_limits(&ResolvedModelLimits::default());
        assert!(layers.is_empty());
    }

    #[test]
    fn a_partly_configured_model_still_fails_on_the_limit_nobody_declared() {
        let limits = ResolvedModelLimits {
            context_tokens: Some(configured(
                128_000,
                "models.\"acme/example-model\".context_tokens",
            )),
            ..ResolvedModelLimits::default()
        };

        let err = layers()
            .with_configured_limits(&limits)
            .resolve()
            .expect_err("a missing limit");
        assert_eq!(err.kind, ModelProfileErrorKind::MissingLimits);
        assert_eq!(err.field, Some(ProfileField::MaxInputTokens));
    }

    #[test]
    fn configured_limits_beat_catalog_metadata_and_both_layers_survive() {
        let limits = ResolvedModelLimits {
            max_output_tokens: Some(configured(
                2_048,
                "models.\"acme/example-model\".max_output_tokens",
            )),
            ..ResolvedModelLimits::default()
        };
        let resolution = layers()
            .with_embedded(
                ModelRecord::new().with_limits(ModelLimits::new(128_000, 124_000, 8_192)),
            )
            .with_configured_limits(&limits)
            .resolve()
            .expect("a profile");

        // The explicit value wins, and the layer it came from is on the record.
        assert_eq!(resolution.profile.limits.max_output_tokens, 2_048);
        assert_eq!(
            resolution
                .profile
                .provenance_of(ProfileField::MaxOutputTokens)
                .expect("provenance")
                .source,
            CatalogSource::Explicit
        );
        // Both offers survive, with the losing one still readable.
        let offers = resolution.contributions_for(ProfileField::MaxOutputTokens);
        assert_eq!(offers.len(), 2);
        assert_eq!(offers[0].layer, CatalogSource::Explicit);
        assert_eq!(offers[0].value, 2_048);
        assert_eq!(offers[1].layer, CatalogSource::Embedded);
        assert_eq!(offers[1].value, 8_192);
        // And Smith can still name the file and key the winner came from.
        let source = resolution
            .configured_source(ProfileField::MaxOutputTokens)
            .expect("a configuration source");
        assert_eq!(source.layer, Layer::ProjectFile);
        assert!(source.key.contains("max_output_tokens"));
        // The fields configuration said nothing about fell through.
        assert_eq!(resolution.profile.limits.context_tokens, 128_000);
        assert!(
            resolution
                .configured_source(ProfileField::ContextTokens)
                .is_none()
        );
    }

    #[test]
    fn the_precedence_order_runs_explicit_provider_embedded_then_cache() {
        let limits = ResolvedModelLimits {
            context_tokens: Some(configured(
                1_000,
                "models.\"acme/example-model\".context_tokens",
            )),
            ..ResolvedModelLimits::default()
        };
        let resolution = layers()
            .with_cached_remote(ModelRecord::new().with_limits(ModelLimits::new(4_000, 4_000, 400)))
            .with_embedded(ModelRecord::new().with_limits(ModelLimits::new(3_000, 3_000, 300)))
            .with_provider_local(
                ModelRecord::new().with_limits(ModelLimits::new(2_000, 2_000, 200)),
            )
            .with_configured_limits(&limits)
            .resolve()
            .expect("a profile");

        assert_eq!(resolution.profile.limits.context_tokens, 1_000);
        assert_eq!(resolution.profile.limits.max_input_tokens, 2_000);
        assert_eq!(resolution.profile.limits.max_output_tokens, 200);
        let ordered: Vec<CatalogSource> = resolution
            .contributions_for(ProfileField::ContextTokens)
            .iter()
            .map(|offer| offer.layer)
            .collect();
        assert_eq!(
            ordered,
            [
                CatalogSource::Explicit,
                CatalogSource::ProviderLocal,
                CatalogSource::Embedded,
                CatalogSource::CachedRemote,
            ]
        );
    }

    #[test]
    fn a_layer_answers_only_for_the_provider_it_was_built_for() {
        let record = ModelRecord::new().with_limits(ModelLimits::new(1_000, 900, 100));
        assert!(layers().with_embedded(record.clone()).resolve().is_ok());

        // The same record, registered for `acme`, must stay silent once the
        // session switches to another provider.
        let scoped = StaticSource::new(EMBEDDED_SOURCE, CatalogSource::Embedded)
            .for_provider("acme")
            .with_model("example-model", record);
        let other = CatalogLayers::new("other", ModelId::new("example-model"))
            .with_source(Arc::new(scoped));
        assert_eq!(
            other.resolve().expect_err("no metadata").kind,
            ModelProfileErrorKind::UnknownModel
        );
    }
}
