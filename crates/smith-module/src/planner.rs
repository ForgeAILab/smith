use std::collections::{BTreeMap, BTreeSet};

use crate::{
    CompiledModule, ModuleComposition, ModuleContext, ModuleContribution, ModuleDescriptor, Mounted,
};

/// Why a selected module cannot satisfy the mount graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleBlockReason {
    /// Named requirement is absent, off, inactive, blocked, or failed.
    Requirement(String),
    /// Exact strongly connected requirement group, in id order.
    Cycle(Vec<String>),
}

/// Plain session state for future configuration and client listings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleState {
    /// The complete returned contribution set entered composition.
    Mounted,
    /// The explicit enabled set excludes this compiled module.
    Off,
    /// The binary contains no implementation for this known id.
    NotBuilt,
    /// Requirements prevent mounting.
    Blocked {
        /// Requirement or cycle responsible.
        reason: ModuleBlockReason,
    },
    /// Mount returned an error and contributed nothing.
    Failed {
        /// Safe user-facing mount error.
        reason: String,
    },
    /// Selected but inapplicable under the current host facts.
    Inactive {
        /// Safe explanation provided by the module.
        reason: String,
    },
}

/// One plain listing row, with no executable handles or configuration dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleReport {
    /// Known id, build status, default, description, and provenance.
    pub descriptor: ModuleDescriptor,
    /// Outcome of this composition's selection and mount.
    pub state: ModuleState,
}

/// Complete values returned by one successfully mounted module.
#[derive(Debug, Clone)]
pub struct MountedModule {
    /// Plain build metadata.
    pub descriptor: ModuleDescriptor,
    /// Exact implementation revision.
    pub revision: String,
    /// Values consumed atomically by the factory.
    pub contributions: Vec<ModuleContribution>,
}

/// Mount values in requirement order and plain reports in stable id order.
#[derive(Debug, Clone, Default)]
pub struct MountPlan {
    /// Only successful mounts appear here.
    pub mounted: Vec<MountedModule>,
    /// Every known module, including omitted first-party implementations.
    pub report: Vec<ModuleReport>,
}

/// Mounts an explicit enabled set, isolating errors and rejecting cycle members.
///
/// Requirements depend on successful mounting, not merely selection. Cycles
/// are identified before calling any mount so downstream modules name their
/// unmet requirement rather than being mislabeled as cycle members.
pub fn mount_modules(composition: &ModuleComposition, context: &ModuleContext) -> MountPlan {
    let mut catalog = composition
        .known
        .iter()
        .cloned()
        .map(|mut descriptor| {
            descriptor.compiled_in = false;
            (descriptor.id.clone(), descriptor)
        })
        .collect::<BTreeMap<_, _>>();
    let mut implementations = BTreeMap::new();
    let mut states = BTreeMap::new();
    for compiled in &composition.compiled {
        let descriptor = compiled.descriptor();
        let id = descriptor.id.clone();
        catalog.insert(id.clone(), descriptor);
        if implementations.insert(id.clone(), compiled).is_some() {
            states.insert(
                id,
                ModuleState::Failed {
                    reason: "duplicate compiled module id".into(),
                },
            );
        }
    }
    for (id, descriptor) in &catalog {
        if !descriptor.compiled_in {
            states.insert(id.clone(), ModuleState::NotBuilt);
        } else if !composition.enabled.contains(id) {
            states.entry(id.clone()).or_insert(ModuleState::Off);
        }
    }
    let graph = implementations
        .iter()
        .filter(|(id, _)| !states.contains_key(*id))
        .map(|(id, compiled)| {
            let requirements = compiled
                .module
                .requirements()
                .iter()
                .map(|id| (*id).to_owned())
                .collect();
            (id.clone(), requirements)
        })
        .collect::<BTreeMap<String, Vec<String>>>();
    // Small native catalogs need no graph dependency. Mutual reachability
    // identifies exact cycle membership, including overlapping cycles.
    let reachability = graph
        .keys()
        .map(|id| (id.clone(), reachable(id, &graph)))
        .collect::<BTreeMap<_, _>>();
    for id in graph.keys() {
        if reachability[id].contains(id) {
            let members = graph
                .keys()
                .filter(|other| {
                    reachability[id].contains(*other) && reachability[*other].contains(id)
                })
                .cloned()
                .collect();
            states.insert(
                id.clone(),
                ModuleState::Blocked {
                    reason: ModuleBlockReason::Cycle(members),
                },
            );
        }
    }
    let mut mounted = Vec::new();
    for id in graph.keys() {
        mount_one(
            id,
            &implementations,
            &mut states,
            &mut mounted,
            composition,
            context,
        );
    }
    MountPlan {
        mounted,
        report: catalog
            .into_iter()
            .map(|(id, descriptor)| ModuleReport {
                descriptor,
                state: states
                    .remove(&id)
                    .expect("every catalog entry has a mount outcome"),
            })
            .collect(),
    }
}

fn reachable(id: &str, graph: &BTreeMap<String, Vec<String>>) -> BTreeSet<String> {
    let mut reached = BTreeSet::new();
    let mut pending = graph.get(id).cloned().unwrap_or_default();
    while let Some(next) = pending.pop() {
        if reached.insert(next.clone())
            && let Some(requirements) = graph.get(&next)
        {
            pending.extend(requirements.iter().cloned());
        }
    }
    reached
}

fn mount_one(
    id: &str,
    implementations: &BTreeMap<String, &CompiledModule>,
    states: &mut BTreeMap<String, ModuleState>,
    mounted: &mut Vec<MountedModule>,
    composition: &ModuleComposition,
    context: &ModuleContext,
) {
    if states.contains_key(id) {
        return;
    }
    let Some(compiled) = implementations.get(id) else {
        return;
    };
    let mut requirements = compiled.module.requirements().to_vec();
    requirements.sort_unstable();
    for requirement in requirements {
        mount_one(
            requirement,
            implementations,
            states,
            mounted,
            composition,
            context,
        );
        if states.get(requirement) != Some(&ModuleState::Mounted) {
            states.insert(
                id.into(),
                ModuleState::Blocked {
                    reason: ModuleBlockReason::Requirement(requirement.into()),
                },
            );
            return;
        }
    }
    let mut own_context = context.clone();
    own_context.settings = composition.settings.get(id).cloned().unwrap_or_default();
    let state = match compiled.module.mount(&own_context) {
        Ok(Mounted::Contributions(contributions)) => {
            mounted.push(MountedModule {
                descriptor: compiled.descriptor(),
                revision: compiled.module.revision().into(),
                contributions,
            });
            ModuleState::Mounted
        }
        Ok(Mounted::Inactive { reason }) => ModuleState::Inactive { reason },
        Err(error) => ModuleState::Failed {
            reason: error.to_string(),
        },
    };
    states.insert(id.into(), state);
}

#[cfg(test)]
mod tests;
