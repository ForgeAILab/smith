//! Compiled-in harness reference skills.
//!
//! At runtime the active workspace is an arbitrary user project, so Smith's
//! reference documentation is not on disk. Each reference below embeds one
//! shipped document at compile time, keeping the repository documentation the
//! single source of truth: the activated instructions are byte-identical to
//! the document at the revision the binary was built from. The set registers
//! in the lowest-precedence built-in layer, where user, trusted-workspace,
//! and session declarations may shadow it by name, and activation contributes
//! bounded instructions only — never a tool, permission, approval,
//! credential, or wider workspace.
//!
//! # Why a reference registers as an outline plus sections
//!
//! A whole reference document is far too large to activate speculatively.
//! `configuration.md` alone estimates near 10k tokens, and the context
//! planner charges every activated instruction against one capability budget
//! shared with the tool schemas — so two documents bound "just in case" could
//! leave no room for the tools the task actually needs, and overflowing that
//! budget fails the turn outright.
//!
//! So each document registers as one cheap outline naming its sections, plus
//! one activatable skill per `##` section. The agent binds the outline to see
//! what a reference covers, then binds only the section that answers the
//! question in front of it. The split is by heading at load time rather than
//! by physically splitting the file, so `docs/*.md` stay whole for human
//! readers and for the links README and the documents themselves carry.

use agent_runtime::ability::Skill;

use crate::skills::SmithSkillSources;

/// One compiled-in reference document and its retrieval metadata.
///
/// The description is authored for descriptor-first retrieval: it names the
/// tasks the reference answers, because the searchable keywords are derived
/// from the name and description alone, without opening the body.
struct HarnessReference {
    name: &'static str,
    /// Names the reference in a section's own description, so a section reads
    /// as "Smith security reference" rather than repeating the full routing
    /// sentence of its parent.
    subject: &'static str,
    description: &'static str,
    body: &'static str,
}

const HARNESS_REFERENCES: [HarnessReference; 4] = [
    HarnessReference {
        name: "smith.configuration",
        subject: "configuration",
        description: "Outline of the Smith configuration and agent-profile \
            reference: config.toml discovery and layered precedence, profiles \
            with posture and inheritance, providers, credentials, model \
            limits, reasoning controls, policy defaults, SMITH_ environment \
            variables, and command-line flags. Activate before editing any \
            .smith/config.toml or explaining how to configure Smith, then \
            activate the section it names.",
        body: include_str!("../../../docs/configuration.md"),
    },
    HarnessReference {
        name: "smith.headless",
        subject: "headless protocol",
        description: "Outline of the Smith headless protocol reference for \
            smith -p: input modes, text, json, and stream-json output \
            formats, event framing, exit behavior, and resuming sessions \
            non-interactively. Activate before scripting or integrating \
            smith -p, then activate the section it names.",
        body: include_str!("../../../docs/headless-protocol.md"),
    },
    HarnessReference {
        name: "smith.persistence",
        subject: "persistence and recovery",
        description: "Outline of the Smith persistence and recovery \
            reference: session snapshots, redacted JSONL journals, protected \
            checkpoints, resume flow, and crash recovery behavior. Activate \
            before diagnosing saved sessions, journals, checkpoints, or \
            resume questions, then activate the section it names.",
        body: include_str!("../../../docs/persistence-recovery.md"),
    },
    HarnessReference {
        name: "smith.security",
        subject: "security threat model",
        description: "Outline of the Smith security threat model reference: \
            trust boundaries, project trust, approvals, credential \
            redaction, tool containment, and why text is never authority. \
            Activate before reasoning about Smith permissions, approvals, or \
            trust behavior, then activate the section it names.",
        body: include_str!("../../../docs/security.md"),
    },
];

/// Whole task guides with descriptions authored for descriptor retrieval.
struct HarnessGuide {
    name: &'static str,
    description: &'static str,
    /// Distinctive pre-activation terms; see [`crate::skills::AUTHORED_KEYWORDS`].
    keywords: &'static str,
    body: &'static str,
}

const HARNESS_GUIDES: [HarnessGuide; 9] = [
    HarnessGuide {
        name: "smith.guide.overview",
        description: "Find the right Smith task guide; discover CLI subcommands, slash commands, setup, models, profiles, sessions, extensions, automation, and troubleshooting.",
        keywords: "smith smith-guides",
        body: include_str!("../../../docs/guides/overview.md"),
    },
    HarnessGuide {
        name: "smith.guide.setup",
        description: "Set up Smith with a provider and model, connect credentials through user-run setup, configure image generation, and verify configuration and module availability.",
        keywords: "providers credentials generate_image",
        body: include_str!("../../../docs/guides/setup.md"),
    },
    HarnessGuide {
        name: "smith.guide.models-and-thinking",
        description: "Select or switch Smith models, enable or disable thinking and reasoning, choose effort and context windows, and verify model capabilities and limits.",
        keywords: "effort reasoning-effort context-window",
        body: include_str!("../../../docs/guides/models-and-thinking.md"),
    },
    HarnessGuide {
        name: "smith.guide.vision",
        description: "Enable Smith image viewing for a text-only model, select a vision profile or provider/model, interpret image placeholders, and diagnose unavailable view_image.",
        keywords: "vision view_image text-only",
        body: include_str!("../../../docs/guides/vision.md"),
    },
    HarnessGuide {
        name: "smith.guide.profiles-and-advisor",
        description: "Create and inherit Smith profiles, switch profiles and postures, configure an advisor, and control delegation and capability limits.",
        keywords: "profiles profile_order advisor posture",
        body: include_str!("../../../docs/guides/profiles-and-advisor.md"),
    },
    HarnessGuide {
        name: "smith.guide.sessions",
        description: "List, resume, or start Smith sessions; undo, redo, or revert a turn's file changes; set multi-turn goals; inspect and follow up child agents; read status and history.",
        keywords: "resume undo redo",
        body: include_str!("../../../docs/guides/sessions.md"),
    },
    HarnessGuide {
        name: "smith.guide.extending",
        description: "Extend Smith with skills, file commands, plugins, MCP servers, and modules; install, trust, list, and diagnose extensions.",
        keywords: "plugins mcp skills",
        body: include_str!("../../../docs/guides/extending.md"),
    },
    HarnessGuide {
        name: "smith.guide.headless",
        description: "Run Smith non-interactively with smith -p, pass prompts and images, choose text or JSON output, resume sessions, and integrate scripts and automation.",
        keywords: "headless stream-json",
        body: include_str!("../../../docs/guides/headless.md"),
    },
    HarnessGuide {
        name: "smith.guide.troubleshooting",
        description: "Diagnose Smith startup, configuration precedence, provider and model errors, missing tools, inactive modules, trust, approvals, and session recovery.",
        keywords: "troubleshoot troubleshooting diagnostics",
        body: include_str!("../../../docs/guides/troubleshooting.md"),
    },
];

/// The shipped task guides, activated whole in every Smith host.
pub fn harness_guide_skills() -> Vec<Skill> {
    HARNESS_GUIDES
        .iter()
        .map(|guide| {
            Skill::inline(guide.name, guide.description, guide.body)
                .with_metadata(crate::skills::AUTHORED_KEYWORDS, guide.keywords)
        })
        .collect()
}

/// The marker a reference document uses for a top-level section.
const SECTION_MARKER: &str = "## ";

/// One `##` section carved out of a reference document.
struct DocumentSection<'a> {
    /// The heading text, without the `##` marker.
    heading: &'a str,
    /// A name-safe slug derived from the heading.
    slug: String,
    /// The section's full text, heading line included.
    body: &'a str,
}

/// Splits `document` into the text before its first `##` heading and one
/// entry per section.
///
/// A line only opens a section when it starts at the beginning of a line, so
/// a `## ` inside a fenced code block still belongs to the section around it
/// unless it is itself at column zero — which no reference document does.
fn split_sections(document: &str) -> (&str, Vec<DocumentSection<'_>>) {
    let mut starts = Vec::new();
    for (offset, _) in document.match_indices(SECTION_MARKER) {
        if offset == 0 || document.as_bytes()[offset - 1] == b'\n' {
            starts.push(offset);
        }
    }
    let preamble = &document[..starts.first().copied().unwrap_or(document.len())];
    let mut sections = Vec::with_capacity(starts.len());
    for (index, start) in starts.iter().copied().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(document.len());
        let body = document[start..end].trim_end();
        let heading = body[SECTION_MARKER.len()..]
            .lines()
            .next()
            .unwrap_or_default()
            .trim();
        sections.push(DocumentSection {
            heading,
            slug: slugify(heading),
            body,
        });
    }
    (preamble.trim(), sections)
}

/// A heading rendered as a name-safe slug: ASCII alphanumerics lowercased,
/// every other run collapsed to a single `-`.
fn slugify(heading: &str) -> String {
    let mut slug = String::with_capacity(heading.len());
    for character in heading.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_owned()
}

/// The first sentence of a section's prose, for its routing description.
///
/// Retrieval scores a descriptor on its name and description alone, so this
/// carries the section's own vocabulary — the terms a model would search for
/// — without charging anything against the capability budget, which only
/// activated bodies consume.
fn first_sentence(body: &str) -> String {
    let mut in_fence = false;
    let prose = body
        .lines()
        .skip(1)
        .map(str::trim)
        .find(|line| {
            if line.starts_with("```") {
                in_fence = !in_fence;
                return false;
            }
            // A sentence, not a fragment: several sections open on a config
            // snippet or a lead-in like "The run surface accepts:", which
            // would put shell noise where the routing vocabulary belongs.
            !in_fence
                && !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with('|')
                && !line.starts_with('-')
                && (line.ends_with('.') || line.contains(". "))
        })
        .unwrap_or_default();
    let sentence = match prose.find(". ") {
        Some(end) => &prose[..=end],
        None => prose,
    };
    sentence.chars().take(200).collect()
}

/// The compiled-in harness reference skills, in stable declaration order.
///
/// Each document contributes its outline first, then its sections in document
/// order, so the registry order stays derivable from the documents themselves.
pub fn harness_reference_skills() -> Vec<Skill> {
    let mut skills = Vec::new();
    for reference in &HARNESS_REFERENCES {
        let (preamble, sections) = split_sections(reference.body);
        skills.push(Skill::inline(
            reference.name,
            reference.description,
            outline(reference, preamble, &sections),
        ));
        for section in &sections {
            skills.push(Skill::inline(
                format!("{}.{}", reference.name, section.slug),
                format!(
                    "Smith {} reference, section \"{}\". {}",
                    reference.subject,
                    section.heading,
                    first_sentence(section.body)
                ),
                section.body.to_owned(),
            ));
        }
    }
    skills
}

/// The outline body: what the reference covers, and the exact skill name that
/// loads each section.
fn outline(
    reference: &HarnessReference,
    preamble: &str,
    sections: &[DocumentSection<'_>],
) -> String {
    let mut body = String::new();
    if !preamble.is_empty() {
        body.push_str(preamble);
        body.push_str("\n\n");
    }
    body.push_str(
        "This outline is the map, not the text. Activate the one section that \
         answers the question in front of you; each name below is a skill:\n\n",
    );
    for section in sections {
        body.push_str(&format!(
            "- `{}.{}` — {}\n",
            reference.name, section.slug, section.heading
        ));
    }
    body
}

/// Smith's default skill sources: the built-in references and task guides.
///
/// [`crate::factory::RuntimeRequest::new`] starts from this set so the TUI
/// and `smith -p` compose one identical index. Assigning
/// [`crate::factory::RuntimeRequest::skills`] replaces the set entirely; a
/// direct embedder receives no implicit built-in entries.
pub fn built_in_sources() -> SmithSkillSources {
    harness_reference_skills()
        .into_iter()
        .chain(harness_guide_skills())
        .fold(SmithSkillSources::new(), SmithSkillSources::with_built_in)
}

#[cfg(test)]
mod tests {
    use agent_runtime::ability::activation::Activated;
    use agent_runtime::registry::TrustClass;

    use super::*;
    use crate::skills::SmithSkillLayer;

    /// The body an activation would contribute, by skill name.
    fn body_of(name: &str) -> String {
        let resolved = built_in_sources().resolve().expect("built-ins resolve");
        let ability = resolved
            .abilities()
            .iter()
            .find(|ability| ability.name() == name)
            .unwrap_or_else(|| panic!("`{name}` is activatable"))
            .clone();
        match ability.materialize().expect("inline body materializes") {
            Activated::SkillInstructions(text) => text,
            other => panic!("`{name}` materialized as {other:?}"),
        }
    }

    #[test]
    fn built_in_references_resolve_as_activatable_host_policy_entries() {
        let resolved = built_in_sources().resolve().expect("built-ins resolve");
        let index = resolved.index();
        let sections: usize = HARNESS_REFERENCES
            .iter()
            .map(|reference| split_sections(reference.body).1.len())
            .sum();
        assert!(sections > 0, "the references must carry `##` sections");
        assert_eq!(
            index.len(),
            HARNESS_REFERENCES.len() + sections + HARNESS_GUIDES.len()
        );
        for entry in index {
            assert_eq!(entry.layer, SmithSkillLayer::BuiltIn);
            assert!(entry.activatable);
            assert_eq!(entry.trust, TrustClass::HostPolicy);
        }
    }

    /// A guide that pre-activates on an everyday coding word would spend its
    /// instructions on prompts that are not about Smith at all.
    #[test]
    fn guides_pre_activate_only_on_their_authored_keywords() {
        let resolved = built_in_sources().resolve().expect("built-ins resolve");
        for guide in &HARNESS_GUIDES {
            let entry = resolved
                .index()
                .iter()
                .find(|entry| entry.name() == guide.name)
                .expect("guide is indexed");
            let keywords = &entry.descriptor.card().keywords;
            // The card keeps its keywords normalized and sorted.
            let mut authored = guide
                .keywords
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            authored.sort();
            assert_eq!(keywords, &authored, "{}", guide.name);
            for common in [
                "file", "files", "turn", "model", "models", "image", "session", "edit",
            ] {
                assert!(
                    !keywords.iter().any(|keyword| keyword == common),
                    "{} pre-activates on `{common}`",
                    guide.name
                );
            }
        }
    }

    #[test]
    fn guides_activate_whole_and_fit_the_descriptor_token_budget() {
        use agent_runtime::ability::descriptor::ContextCost;

        for guide in &HARNESS_GUIDES {
            assert_eq!(body_of(guide.name).as_bytes(), guide.body.as_bytes());
            let estimate = ContextCost::estimate("", guide.body).instruction_tokens;
            let limit = if guide.name == "smith.guide.overview" {
                600
            } else {
                1_200
            };
            assert!(
                estimate <= limit,
                "{} estimates {estimate} tokens, over {limit}",
                guide.name
            );
        }
    }

    #[test]
    fn guide_toml_examples_use_the_config_file_model() {
        for guide in &HARNESS_GUIDES {
            let mut block = None;
            for line in guide.body.lines() {
                let line = line.trim();
                if line == "```toml" {
                    block = Some(String::new());
                } else if line.starts_with("```") {
                    if let Some(text) = block.take() {
                        // ConfigFile and its tables reject unknown fields.
                        toml::from_str::<smith_config::model::ConfigFile>(&text).unwrap_or_else(
                            |error| panic!("{} has invalid TOML: {error}", guide.name),
                        );
                    }
                } else if let Some(text) = &mut block {
                    text.push_str(line);
                    text.push('\n');
                }
            }
            assert!(block.is_none(), "{} has an unclosed TOML fence", guide.name);
        }
    }

    /// Sections are the point of the split: activating one must contribute
    /// exactly that section's shipped bytes and nothing else, so the
    /// documentation stays the single source of truth at section granularity
    /// the same way it used to at document granularity.
    #[test]
    fn activating_a_section_returns_exactly_that_section_of_the_document() {
        for reference in &HARNESS_REFERENCES {
            let (_, sections) = split_sections(reference.body);
            for section in &sections {
                let name = format!("{}.{}", reference.name, section.slug);
                assert_eq!(body_of(&name), section.body);
            }
        }
    }

    /// The outline exists so a reference can be surveyed for a fraction of
    /// what reading it costs. If an outline ever approached its document, the
    /// split would have stopped paying for itself.
    #[test]
    fn an_outline_names_every_section_and_stays_far_cheaper_than_the_document() {
        for reference in &HARNESS_REFERENCES {
            let (_, sections) = split_sections(reference.body);
            let outline = body_of(reference.name);
            for section in &sections {
                let name = format!("{}.{}", reference.name, section.slug);
                assert!(
                    outline.contains(&name),
                    "{} omits `{name}` and so cannot route to it",
                    reference.name
                );
                assert!(
                    outline.contains(section.heading),
                    "{} omits the heading `{}`",
                    reference.name,
                    section.heading
                );
            }
            assert!(
                outline.len() * 4 < reference.body.len(),
                "{}'s outline is {} bytes against a {}-byte document: too \
                 close to just activating the document",
                reference.name,
                outline.len(),
                reference.body.len()
            );
        }
    }

    /// Every activatable body has to fit a budget shared with the tool
    /// schemas, so no single one may dominate it. The whole
    /// `configuration.md` estimated near 10k tokens — well past this — which
    /// is what made a mid-turn activation overflow the budget and fail the
    /// turn.
    #[test]
    fn no_single_activation_can_dominate_the_capability_budget() {
        const CEILING_TOKENS: usize = 3_000;
        for skill in harness_reference_skills() {
            let body = match skill.source {
                agent_runtime::ability::SkillSource::Inline(ref body) => body,
                ref other => panic!("built-ins are inline, got {other:?}"),
            };
            let tokens = body.chars().count() / 4;
            assert!(
                tokens <= CEILING_TOKENS,
                "`{}` activates ~{tokens} tokens, over the {CEILING_TOKENS} ceiling",
                skill.name
            );
        }
    }

    /// Slugs land in skill names, which the skill registry validates.
    #[test]
    fn every_derived_section_name_is_a_legal_skill_name() {
        for skill in harness_reference_skills() {
            assert!(
                !skill.name.is_empty() && skill.name.chars().count() <= 96,
                "`{}` is not a legal length",
                skill.name
            );
            assert!(
                skill.name.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
                }),
                "`{}` has characters the registry rejects",
                skill.name
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
