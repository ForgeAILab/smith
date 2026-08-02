//! Compiled-in harness reference skills.
//!
//! At runtime the active workspace is an arbitrary user project, so Smith's
//! reference documentation is not on disk. Each skill here embeds one shipped
//! reference document at compile time, keeping the repository documentation
//! the single source of truth: the activated instructions are byte-identical
//! to the document at the revision the binary was built from. The set
//! registers in the lowest-precedence built-in layer, where user,
//! trusted-workspace, and session declarations may shadow it by name, and
//! activation contributes bounded instructions only — never a tool,
//! permission, approval, credential, or wider workspace.

use agent_runtime::ability::Skill;

use crate::skills::SmithSkillSources;

/// One compiled-in reference document and its retrieval metadata.
///
/// The description is authored for descriptor-first retrieval: it names the
/// tasks the reference answers, because the searchable keywords are derived
/// from the name and description alone, without opening the body.
struct HarnessReference {
    name: &'static str,
    description: &'static str,
    body: &'static str,
}

const HARNESS_REFERENCES: [HarnessReference; 4] = [
    HarnessReference {
        name: "smith.configuration",
        description: "Smith configuration and agent-profile reference: \
            config.toml discovery and layered precedence, profiles with \
            posture and inheritance, providers, credentials, model limits, \
            reasoning controls, policy defaults, SMITH_ environment \
            variables, and command-line flags. Activate before editing any \
            .smith/config.toml or explaining how to configure Smith.",
        body: include_str!("../../../docs/configuration.md"),
    },
    HarnessReference {
        name: "smith.headless",
        description: "Smith headless protocol reference for smith -p: input \
            modes, text, json, and stream-json output formats, event \
            framing, exit behavior, and resuming sessions non-interactively. \
            Activate before scripting or integrating smith -p.",
        body: include_str!("../../../docs/headless-protocol.md"),
    },
    HarnessReference {
        name: "smith.persistence",
        description: "Smith persistence and recovery reference: session \
            snapshots, redacted JSONL journals, protected checkpoints, \
            resume flow, and crash recovery behavior. Activate before \
            diagnosing saved sessions, journals, checkpoints, or resume \
            questions.",
        body: include_str!("../../../docs/persistence-recovery.md"),
    },
    HarnessReference {
        name: "smith.security",
        description: "Smith security threat model reference: trust \
            boundaries, project trust, approvals, credential redaction, \
            tool containment, and why text is never authority. Activate \
            before reasoning about Smith permissions, approvals, or trust \
            behavior.",
        body: include_str!("../../../docs/security.md"),
    },
];

/// The compiled-in harness reference skills, in stable declaration order.
pub fn harness_reference_skills() -> Vec<Skill> {
    HARNESS_REFERENCES
        .iter()
        .map(|reference| Skill::inline(reference.name, reference.description, reference.body))
        .collect()
}

/// Smith's default skill sources: exactly the built-in harness references.
///
/// [`crate::factory::RuntimeRequest::new`] starts from this set so the TUI
/// and `smith -p` compose one identical index. Assigning
/// [`crate::factory::RuntimeRequest::skills`] replaces the set entirely; a
/// direct embedder receives no implicit built-in entries.
pub fn built_in_sources() -> SmithSkillSources {
    harness_reference_skills()
        .into_iter()
        .fold(SmithSkillSources::new(), SmithSkillSources::with_built_in)
}

#[cfg(test)]
mod tests {
    use agent_runtime::ability::activation::Activated;
    use agent_runtime::registry::TrustClass;

    use super::*;
    use crate::skills::SmithSkillLayer;

    #[test]
    fn built_in_references_resolve_as_activatable_host_policy_entries() {
        let resolved = built_in_sources().resolve().expect("built-ins resolve");
        let index = resolved.index();
        assert_eq!(index.len(), HARNESS_REFERENCES.len());
        for entry in index {
            assert_eq!(entry.layer, SmithSkillLayer::BuiltIn);
            assert!(entry.activatable);
            assert_eq!(entry.trust, TrustClass::HostPolicy);
        }
    }

    #[test]
    fn activation_returns_the_exact_embedded_document() {
        let resolved = built_in_sources().resolve().expect("built-ins resolve");
        for reference in &HARNESS_REFERENCES {
            let ability = resolved
                .abilities()
                .iter()
                .find(|ability| ability.name() == reference.name)
                .expect("reference is activatable");
            assert_eq!(
                ability.materialize().expect("inline body materializes"),
                Activated::SkillInstructions(reference.body.into())
            );
        }
    }

    #[test]
    fn user_declaration_shadows_a_built_in_while_it_stays_indexed() {
        let resolved = built_in_sources()
            .with_user(Skill::inline(
                "smith.configuration",
                "User replacement",
                "user body",
            ))
            .resolve()
            .expect("shadowed catalog resolves");
        let winner = resolved
            .abilities()
            .iter()
            .find(|ability| ability.name() == "smith.configuration")
            .expect("name stays activatable");
        assert_eq!(
            winner.materialize().unwrap(),
            Activated::SkillInstructions("user body".into())
        );
        assert!(resolved.index().iter().any(|entry| {
            entry.layer == SmithSkillLayer::BuiltIn
                && entry.descriptor.id().name == "smith.configuration"
        }));
    }
}
