//! Smith-owned skill source precedence and trust policy.
//!
//! Agent Runtime owns descriptor-first retrieval and lazy materialization.
//! Smith owns which sources may contribute privileged instructions. This
//! module resolves one deterministic catalog in the product order
//! `built-in < plugin < user < trusted workspace < session`, while retaining bounded
//! metadata for untrusted workspace declarations without registering their
//! bodies for activation.

mod discovery;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use smith_plugin::{PathVariables, substitute_variables};
use std::sync::Arc;

use agent_runtime::ability::activation::{Activated, ActivationError};
use agent_runtime::ability::descriptor::{AbilityDescriptor, ContextCost};
use agent_runtime::ability::{Ability, AbilityKind, Named, Skill, SkillSource};
use agent_runtime::registry::{RegistrySource, TrustClass};
use agent_runtime_core::error::RuntimeError;
use smith_config::trust::TrustStatus;

pub use discovery::{DiscoveredSkill, DiscoveredSkills, SkillProblem, discover, discover_into};

/// Exact skill declaration explicitly selected by the user before a safe rebind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedSkill {
    /// Effective catalog name.
    pub name: String,
    /// Winning source layer at selection time.
    pub layer: SmithSkillLayer,
    /// Pinned descriptor revision at selection time.
    pub revision: String,
}

/// Converts inspected plugin metadata into the ordinary digest-pinned skill.
/// The host verified the bundle and supplies the SHA-256 of these on-disk bytes.
pub fn plugin_skill(component: &smith_plugin::SkillComponent, digest: [u8; 32]) -> Skill {
    Skill::from_verified_file(
        &component.name,
        component.description.as_deref().unwrap_or(&component.name),
        &component.path,
        digest,
    )
}

/// Smith's deterministic skill source layers, lowest precedence first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SmithSkillLayer {
    /// Shipped by Smith or its built-in packages.
    BuiltIn,
    /// Namespaced content admitted by the host's plugin trust decision.
    Plugin,
    /// Installed in the user's Smith profile.
    User,
    /// Supplied by the active project and gated by exact project trust.
    Workspace,
    /// Explicitly supplied by the embedding host for this session.
    Session,
}

impl SmithSkillLayer {
    /// Stable source spelling used in diagnostics and descriptor tags.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BuiltIn => "built-in",
            Self::Plugin => "plugin",
            Self::User => "user",
            Self::Workspace => "workspace",
            Self::Session => "session",
        }
    }

    fn registry_source(self) -> RegistrySource {
        match self {
            Self::BuiltIn => RegistrySource::BuiltIn,
            // All other declarations are selected by Smith's embedding host.
            // Exact Smith-layer provenance remains on `SkillIndexEntry` and a
            // bounded descriptor tag; the neutral registry has no user/project
            // source variants.
            Self::Plugin | Self::User | Self::Workspace | Self::Session => RegistrySource::Host,
        }
    }
}

/// Bounded descriptor metadata retained by Smith's source resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillIndexEntry {
    /// Searchable descriptor; constructing it performs no skill-body I/O.
    pub descriptor: AbilityDescriptor,
    /// Smith source layer.
    pub layer: SmithSkillLayer,
    /// Whether host trust admitted the body to the runtime activation view.
    pub activatable: bool,
    /// Discovery-time content trust decision, retained without body reads.
    pub trust_status: TrustStatus,
    /// Content trust class used by diagnostics.
    pub trust: TrustClass,
}

impl SkillIndexEntry {
    /// The skill's name, as precedence and shadowing spell it.
    pub fn name(&self) -> &str {
        &self.descriptor.card().id.name
    }

    /// The routing description retrieval scores against.
    pub fn description(&self) -> &str {
        &self.descriptor.card().summary
    }
}

#[derive(Clone)]
struct SkillDeclaration {
    layer: SmithSkillLayer,
    trust: TrustStatus,
    skill: Skill,
    paths: Option<(PathBuf, PathBuf, PathBuf)>,
}

impl fmt::Debug for SkillDeclaration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SkillDeclaration")
            .field("name", &self.skill.name)
            .field("description_chars", &self.skill.description.chars().count())
            .field("layer", &self.layer)
            .field("trust", &self.trust)
            .field("file_backed", &self.skill.instructions_path().is_some())
            .finish()
    }
}

/// Host-supplied skill declarations before deterministic source resolution.
#[derive(Clone, Default)]
pub struct SmithSkillSources {
    declarations: Vec<SkillDeclaration>,
}

impl fmt::Debug for SmithSkillSources {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut counts = BTreeMap::new();
        for declaration in &self.declarations {
            *counts.entry(declaration.layer.as_str()).or_insert(0usize) += 1;
        }
        formatter
            .debug_struct("SmithSkillSources")
            .field("counts", &counts)
            .finish()
    }
}

impl SmithSkillSources {
    /// Creates an empty source set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a built-in skill.
    #[must_use]
    pub fn with_built_in(mut self, skill: Skill) -> Self {
        self.push(SmithSkillLayer::BuiltIn, TrustStatus::Trusted, skill);
        self
    }

    /// Adds a digest-pinned plugin skill and its activation-time path variables.
    #[must_use]
    pub fn with_plugin(mut self, skill: Skill, paths: PathVariables<'_>) -> Self {
        self.declarations.push(SkillDeclaration {
            layer: SmithSkillLayer::Plugin,
            trust: TrustStatus::Trusted,
            skill,
            paths: Some((
                paths.plugin_root.into(),
                paths.plugin_data.into(),
                paths.project_dir.into(),
            )),
        });
        self
    }

    /// Adds a user-profile skill.
    #[must_use]
    pub fn with_user(mut self, skill: Skill) -> Self {
        self.push(SmithSkillLayer::User, TrustStatus::Trusted, skill);
        self
    }

    /// Adds workspace metadata plus the host's exact trust result.
    ///
    /// Only [`TrustStatus::Trusted`] permits activation. Untrusted, changed,
    /// and denied declarations remain bounded index metadata and cannot shadow
    /// an admitted lower-precedence source.
    #[must_use]
    pub fn with_workspace(mut self, skill: Skill, trust: TrustStatus) -> Self {
        self.push(SmithSkillLayer::Workspace, trust, skill);
        self
    }

    /// Adds an explicit per-session override.
    #[must_use]
    pub fn with_session(mut self, skill: Skill) -> Self {
        self.push(SmithSkillLayer::Session, TrustStatus::Trusted, skill);
        self
    }

    fn push(&mut self, layer: SmithSkillLayer, trust: TrustStatus, skill: Skill) {
        self.declarations.push(SkillDeclaration {
            layer,
            trust,
            skill,
            paths: None,
        });
    }

    /// Resolves deterministic precedence without opening a skill body.
    pub fn resolve(&self) -> Result<ResolvedSmithSkills, RuntimeError> {
        let mut seen_by_layer = BTreeSet::new();
        let mut winners = BTreeMap::<String, SmithSkillAbility>::new();
        let mut index = Vec::with_capacity(self.declarations.len());

        for layer in [
            SmithSkillLayer::BuiltIn,
            SmithSkillLayer::Plugin,
            SmithSkillLayer::User,
            SmithSkillLayer::Workspace,
            SmithSkillLayer::Session,
        ] {
            let mut declarations = self
                .declarations
                .iter()
                .filter(|declaration| declaration.layer == layer)
                .collect::<Vec<_>>();
            declarations.sort_by(|left, right| left.skill.name.cmp(&right.skill.name));
            for declaration in declarations {
                if layer == SmithSkillLayer::Plugin {
                    validate_plugin_name(&declaration.skill.name)?;
                } else {
                    validate_name(&declaration.skill.name)?;
                }
                if !seen_by_layer.insert((layer, declaration.skill.name.clone())) {
                    return Err(RuntimeError::conflict(format!(
                        "skill `{}` is declared more than once in the {} source",
                        declaration.skill.name,
                        layer.as_str()
                    )));
                }
                let admitted =
                    layer != SmithSkillLayer::Workspace || declaration.trust.allows_execution();
                let mut ability =
                    SmithSkillAbility::new(declaration.skill.clone(), layer, declaration.trust);
                ability.paths = declaration.paths.clone();
                index.push(SkillIndexEntry {
                    descriptor: ability.descriptor(),
                    layer,
                    activatable: admitted,
                    trust_status: declaration.trust,
                    trust: ability.trust_class(),
                });
                if admitted {
                    winners.insert(declaration.skill.name.clone(), ability);
                }
            }
        }

        Ok(ResolvedSmithSkills {
            abilities: winners
                .into_values()
                .map(|ability| Arc::new(ability) as Arc<dyn Ability>)
                .collect(),
            index,
        })
    }
}

/// One resolved Smith skill catalog.
#[derive(Clone)]
pub struct ResolvedSmithSkills {
    abilities: Vec<Arc<dyn Ability>>,
    index: Vec<SkillIndexEntry>,
}

impl fmt::Debug for ResolvedSmithSkills {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedSmithSkills")
            .field("activatable", &self.abilities.len())
            .field("indexed", &self.index.len())
            .finish()
    }
}

impl ResolvedSmithSkills {
    /// Resolves the effective admitted declaration without loading its body.
    pub(crate) fn selection(&self, name: &str) -> Result<RequestedSkill, RuntimeError> {
        let entry = self
            .index
            .iter()
            .rev()
            .find(|entry| entry.name() == name && entry.activatable)
            .ok_or_else(|| {
                let reason = self
                    .index
                    .iter()
                    .rev()
                    .find(|entry| entry.name() == name)
                    .map_or("unknown", |entry| match entry.trust_status {
                        TrustStatus::Changed => "content changed",
                        TrustStatus::Denied => "trust denied",
                        _ => "awaiting trust",
                    });
                RuntimeError::config(format!(
                    "skill `{name}` is {reason}; inspect `/skills` or use `/skills trust {name}`"
                ))
            })?;
        Ok(RequestedSkill {
            name: name.to_owned(),
            layer: entry.layer,
            revision: entry.descriptor.content_revision().as_str().to_owned(),
        })
    }

    /// Bounded source index, including untrusted workspace metadata.
    pub fn index(&self) -> &[SkillIndexEntry] {
        &self.index
    }

    /// Uses the same descriptor-first selection for the tool-free reviewer.
    pub(crate) fn instructions_for(
        &self,
        query: &str,
        budget: u32,
    ) -> Result<Vec<String>, RuntimeError> {
        use agent_runtime::capability::{ActivationBudget, CapabilityResolver, RoutingQuery};
        use agent_runtime::registry::{RegistryBuilder, RegistryEntry, ViewFilter};
        let mut builder = RegistryBuilder::new();
        for ability in &self.abilities {
            let descriptor = ability.descriptor();
            builder.declare(RegistryEntry::new(descriptor.card().clone(), descriptor));
        }
        let registry = builder
            .seal()
            .map_err(|error| RuntimeError::conflict(error.to_string()))?;
        let selected = CapabilityResolver::new().pre_activate(
            &registry.view(&ViewFilter::new()),
            &RoutingQuery::derive(query, Vec::<String>::new()),
            ActivationBudget::new(0, 8).with_instruction_budget(budget),
        );
        let ids = selected.result().activated_ids();
        self.abilities
            .iter()
            .filter(|ability| {
                ids.iter()
                    .any(|(id, _)| id == &ability.descriptor().card().id)
            })
            .map(|ability| match ability.materialize() {
                Ok(Activated::SkillInstructions(body)) => Ok(body),
                Ok(_) => Err(RuntimeError::config("skill did not supply instructions")),
                Err(error) => Err(RuntimeError::config(error.to_string())),
            })
            .collect()
    }

    pub(crate) fn abilities(&self) -> &[Arc<dyn Ability>] {
        &self.abilities
    }
}

#[derive(Clone)]
struct SmithSkillAbility {
    skill: Skill,
    layer: SmithSkillLayer,
    trust: TrustStatus,
    paths: Option<(PathBuf, PathBuf, PathBuf)>,
}

impl SmithSkillAbility {
    fn new(skill: Skill, layer: SmithSkillLayer, trust: TrustStatus) -> Self {
        Self {
            skill: skill.with_registry_source(layer.registry_source()),
            layer,
            trust,
            paths: None,
        }
    }

    fn trust_class(&self) -> TrustClass {
        match self.layer {
            SmithSkillLayer::BuiltIn => TrustClass::HostPolicy,
            SmithSkillLayer::Plugin | SmithSkillLayer::User | SmithSkillLayer::Session => {
                TrustClass::UserContent
            }
            SmithSkillLayer::Workspace if self.trust.allows_execution() => TrustClass::UserContent,
            SmithSkillLayer::Workspace => TrustClass::ExternalContent,
        }
    }
}

impl fmt::Debug for SmithSkillAbility {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SmithSkillAbility")
            .field("name", &self.skill.name)
            .field("layer", &self.layer)
            .field("trust", &self.trust)
            .field("file_backed", &self.skill.instructions_path().is_some())
            .finish()
    }
}

impl Named for SmithSkillAbility {
    fn name(&self) -> &str {
        &self.skill.name
    }
}

impl SmithSkillAbility {
    /// Retrieval keywords: an authored `smith.keywords` list when the skill
    /// carries one, otherwise terms derived from its name and description.
    ///
    /// Pre-activation binds any skill whose keywords meet a prompt term, so a
    /// shipped how-to whose description says "file" or "turn" would load on
    /// ordinary coding prompts. An authored list keeps those to distinctive
    /// terms while the description stays readable for `registry.search`.
    fn keywords(&self) -> Vec<String> {
        match self.skill.metadata.get(AUTHORED_KEYWORDS) {
            Some(list) => list.split_whitespace().map(str::to_lowercase).collect(),
            None => skill_keywords(&self.skill.name, &self.skill.description),
        }
    }
}

impl Ability for SmithSkillAbility {
    fn description(&self) -> &str {
        &self.skill.description
    }

    fn kind(&self) -> AbilityKind {
        AbilityKind::Skill
    }

    fn descriptor(&self) -> AbilityDescriptor {
        let instruction_tokens = match &self.skill.source {
            SkillSource::Inline(body) => ContextCost::estimate("", body).instruction_tokens,
            SkillSource::File(_) => self
                .skill
                .metadata
                .get("smith.instruction_tokens")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(1_024),
        };
        self.skill
            .descriptor()
            .with_tags([
                "skill".to_owned(),
                "smith".to_owned(),
                format!("smith-source-{}", self.layer.as_str()),
            ])
            .with_keywords(self.keywords())
            .with_affordances(["skill-guidance"])
            .with_context_cost(ContextCost::new(0, instruction_tokens))
            .with_input_modalities(["text"])
            .with_output_modalities(["instructions"])
    }

    fn materialize(&self) -> Result<Activated, ActivationError> {
        let activated = self.skill.materialize()?;
        match (activated, &self.paths) {
            (Activated::SkillInstructions(body), Some((root, data, project))) => {
                Ok(Activated::SkillInstructions(substitute_variables(
                    &body,
                    PathVariables {
                        plugin_root: root,
                        plugin_data: data,
                        project_dir: project,
                    },
                )))
            }
            (activated, _) => Ok(activated),
        }
    }
}

fn validate_plugin_name(name: &str) -> Result<(), RuntimeError> {
    let (plugin, component) = name
        .split_once(':')
        .ok_or_else(|| RuntimeError::config("plugin skill needs a namespace"))?;
    for part in [plugin, component] {
        smith_plugin::validate_name(part)
            .map_err(|error| RuntimeError::config(error.to_string()))?;
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), RuntimeError> {
    let valid = !name.trim().is_empty()
        && name.chars().count() <= 96
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        });
    if valid {
        Ok(())
    } else {
        Err(RuntimeError::config(
            "skill name must contain 1..=96 ASCII letters, digits, '.', '_', or '-'",
        ))
    }
}

/// Connective words a description cannot help using. They must not become
/// retrieval keywords: activation admits any candidate with a nonzero score,
/// so one stray "with" in an unrelated prompt would otherwise pull a
/// multi-thousand-token reference body into every context.
/// Metadata key naming a skill's whitespace-separated retrieval keywords.
pub(crate) const AUTHORED_KEYWORDS: &str = "smith.keywords";

const KEYWORD_STOPWORDS: &[&str] = &[
    "activate", "and", "any", "are", "before", "each", "for", "from", "has", "how", "into", "its",
    "not", "one", "only", "over", "that", "the", "them", "then", "this", "use", "using", "was",
    "what", "when", "where", "whether", "which", "with", "you", "your",
];

fn skill_keywords(name: &str, description: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    name.split(['.', '_', '-', ':'])
        .chain(
            description
                .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-')),
        )
        .map(str::trim)
        .filter(|term| term.chars().count() >= 3)
        .map(str::to_lowercase)
        .filter(|term| !KEYWORD_STOPWORDS.contains(&term.as_str()))
        // Deduplicate before capping so a repeated word cannot crowd
        // distinctive terms out of the bounded keyword budget.
        .filter(|term| seen.insert(term.clone()))
        .take(32)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn deterministic_precedence_ignores_untrusted_workspace_shadowing() {
        let sources = SmithSkillSources::new()
            .with_built_in(Skill::inline("review", "Review code", "built-in"))
            .with_user(Skill::inline("review", "Review code", "user"))
            .with_workspace(
                Skill::inline("review", "Review code", "untrusted workspace"),
                TrustStatus::Untrusted,
            )
            .with_session(Skill::inline("other", "Other guidance", "session"));
        let resolved = sources.resolve().unwrap();
        assert_eq!(resolved.abilities.len(), 2);
        let review = resolved
            .abilities
            .iter()
            .find(|ability| ability.name() == "review")
            .unwrap();
        assert_eq!(
            review.materialize().unwrap(),
            Activated::SkillInstructions("user".into())
        );
        assert!(resolved.index.iter().any(|entry| {
            entry.layer == SmithSkillLayer::Workspace
                && !entry.activatable
                && entry.trust == TrustClass::ExternalContent
        }));
    }

    #[test]
    fn trusted_workspace_and_session_override_lower_sources() {
        let sources = SmithSkillSources::new()
            .with_built_in(Skill::inline("review", "Review code", "built-in"))
            .with_user(Skill::inline("review", "Review code", "user"))
            .with_workspace(
                Skill::inline("review", "Review code", "workspace"),
                TrustStatus::Trusted,
            )
            .with_session(Skill::inline("review", "Review code", "session"));
        let resolved = sources.resolve().unwrap();
        assert_eq!(resolved.abilities.len(), 1);
        assert_eq!(
            resolved.abilities[0].materialize().unwrap(),
            Activated::SkillInstructions("session".into())
        );
    }

    #[test]
    fn descriptor_resolution_does_no_io_and_activation_checks_the_pin() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("SKILL.md");
        let reviewed = b"reviewed skill body";
        let sources = SmithSkillSources::new().with_workspace(
            Skill::from_verified_file(
                "review",
                "Review Rust code",
                &path,
                Sha256::digest(reviewed).into(),
            ),
            TrustStatus::Trusted,
        );
        let resolved = sources.resolve().expect("metadata resolves");
        assert!(
            !path.exists(),
            "descriptor resolution read or created the body"
        );
        fs::write(&path, reviewed).unwrap();
        assert_eq!(
            resolved.abilities[0].materialize().unwrap(),
            Activated::SkillInstructions("reviewed skill body".into())
        );
        fs::write(&path, "changed").unwrap();
        assert!(resolved.abilities[0].materialize().is_err());
    }

    #[test]
    fn duplicate_names_inside_one_layer_fail_closed() {
        let error = SmithSkillSources::new()
            .with_user(Skill::inline("same", "One", "one"))
            .with_user(Skill::inline("same", "Two", "two"))
            .resolve()
            .unwrap_err();
        assert!(error.message.contains("more than once"));
    }
}

#[cfg(test)]
mod plugin_tests {
    use super::{SmithSkillLayer, SmithSkillSources};
    use agent_runtime::ability::Skill;
    use agent_runtime::ability::activation::Activated;
    use sha2::{Digest, Sha256};
    use smith_plugin::PathVariables;
    use std::fs;

    #[test]
    fn only_the_plugin_layer_accepts_a_single_namespace() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let paths = PathVariables {
            plugin_root: &root,
            plugin_data: &root,
            project_dir: &root,
        };
        for name in ["tools:deploy", "tools:deploy:extra", ":deploy", "tools:"] {
            let skill = Skill::inline(name, "Deploy", "Body");
            assert!(
                SmithSkillSources::new()
                    .with_user(skill.clone())
                    .resolve()
                    .is_err()
            );
            assert!(
                SmithSkillSources::new()
                    .with_session(skill.clone())
                    .resolve()
                    .is_err()
            );
            assert_eq!(
                SmithSkillSources::new()
                    .with_plugin(skill, paths)
                    .resolve()
                    .is_ok(),
                name == "tools:deploy"
            );
        }
    }

    #[test]
    fn plugin_skill_substitution_follows_the_raw_byte_pin() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = root.join("SKILL.md");
        let body = "Use ${CLAUDE_PLUGIN_ROOT}, ${CLAUDE_PLUGIN_DATA}, ${CLAUDE_PROJECT_DIR}";
        fs::write(&path, body).unwrap();
        let paths = PathVariables {
            plugin_root: &root,
            plugin_data: &root.join("data"),
            project_dir: &root.join("project"),
        };
        let skill = Skill::from_verified_file(
            "tools:deploy",
            "Deploy",
            &path,
            Sha256::digest(body.as_bytes()).into(),
        );
        let resolved = SmithSkillSources::new()
            .with_plugin(skill, paths)
            .with_user(Skill::inline("deploy", "Deploy", "User instructions"))
            .resolve()
            .unwrap();
        assert_eq!(
            resolved
                .index()
                .iter()
                .map(|entry| entry.layer)
                .collect::<Vec<_>>(),
            vec![SmithSkillLayer::Plugin, SmithSkillLayer::User]
        );
        let ability = resolved
            .abilities()
            .iter()
            .find(|ability| ability.name() == "tools:deploy")
            .unwrap();
        assert_eq!(
            ability.materialize().unwrap(),
            Activated::SkillInstructions(format!(
                "Use {}, {}, {}",
                root.display(),
                root.join("data").display(),
                root.join("project").display()
            ))
        );
        fs::write(&path, "Changed bytes").unwrap();
        assert!(ability.materialize().is_err());
    }
}
