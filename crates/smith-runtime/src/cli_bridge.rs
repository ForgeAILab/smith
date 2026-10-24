//! Pure selection and guidance for the capabilities a CLI receives from Smith.
//!
//! The caller supplies the authorized view after registration and session gates.
//! Selection only narrows that view; it cannot restore an absent capability.

#[cfg(unix)]
mod endpoint;
#[cfg(all(test, unix))]
mod endpoint_tests;
#[cfg(all(test, unix))]
mod failure_tests;
#[cfg(all(test, unix))]
mod live_tests;
#[cfg(unix)]
mod server;
#[cfg(unix)]
mod transport;

#[cfg(unix)]
pub(crate) use endpoint::{BridgeDispatch, BridgeEndpoint, RuntimeBridgeDispatch};

use std::collections::{BTreeMap, BTreeSet};

use agent_runtime::harness::{
    ARTIFACT_READ_TOOL_NAME, CAPABILITY_ACTIVATE_TOOL_NAME, CAPABILITY_SEARCH_TOOL_NAME,
    CREATE_GOAL_TOOL_NAME, GET_GOAL_TOOL_NAME, UPDATE_GOAL_TOOL_NAME, WRITE_TODOS_TOOL_NAME,
};
use agent_runtime_core::tool::ToolSpec;

use crate::capability_limits::CORE_TOOL_NAMES;
use crate::prompt::cli_bridge_instruction_fragments;
use crate::tool_output::ARTIFACT_LIST_TOOL_NAME;

pub(crate) const BRIDGE_SERVER_NAME: &str = "smith";

const MAX_TOOL_NAME_BYTES: usize = 128;
const MAX_INSTRUCTION_CHARS: usize = 4_000;

/// Names the host excludes when constructing an external session.
pub(crate) fn excluded_tool_names() -> impl Iterator<Item = &'static str> {
    CORE_TOOL_NAMES.into_iter().chain([
        WRITE_TODOS_TOOL_NAME,
        GET_GOAL_TOOL_NAME,
        CREATE_GOAL_TOOL_NAME,
        UPDATE_GOAL_TOOL_NAME,
    ])
}

/// A selection decision the endpoint can report without recomputing the rule.
/// Name failures are diagnostics; ordinary exclusions describe session policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BridgeOmission {
    Excluded {
        name: String,
    },
    NoAuthorizedSkills {
        name: String,
    },
    NoSupportingTool {
        name: String,
    },
    NameTooLong {
        name: String,
        bytes: usize,
    },
    /// One entry per collision group, with names in registration order.
    NameCollision {
        normalized: String,
        names: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BridgedSet {
    specs: Vec<ToolSpec>,
    omissions: Vec<BridgeOmission>,
}

impl BridgedSet {
    /// Filters the authorized view while preserving each served spec verbatim.
    pub(crate) fn new(specs: Vec<ToolSpec>, has_authorized_skills: bool) -> Self {
        let mut omissions = Vec::new();
        let mut candidates = Vec::new();
        for spec in specs {
            if excluded_tool_names().any(|name| name == spec.name) {
                omissions.push(BridgeOmission::Excluded { name: spec.name });
            } else if !has_authorized_skills && is_registry_support(&spec.name) {
                omissions.push(BridgeOmission::NoAuthorizedSkills { name: spec.name });
            } else {
                candidates.push(spec);
            }
        }
        remove_unsupported_artifact_reader(&mut candidates, &mut omissions);

        // Names rejected for length do not participate in collision groups.
        candidates.retain(|spec| {
            if spec.name.len() <= MAX_TOOL_NAME_BYTES {
                true
            } else {
                omissions.push(BridgeOmission::NameTooLong {
                    name: spec.name.clone(),
                    bytes: spec.name.len(),
                });
                false
            }
        });
        let mut groups = BTreeMap::<String, Vec<String>>::new();
        for spec in &candidates {
            groups
                .entry(normalize_name(&spec.name))
                .or_default()
                .push(spec.name.clone());
        }
        let mut reported = BTreeSet::new();
        candidates.retain(|spec| {
            let normalized = normalize_name(&spec.name);
            let names = &groups[&normalized];
            if names.len() == 1 {
                return true;
            }
            if reported.insert(normalized.clone()) {
                omissions.push(BridgeOmission::NameCollision {
                    normalized,
                    names: names.clone(),
                });
            }
            false
        });

        // Invalid names can remove the reader's last supporting capability.
        remove_unsupported_artifact_reader(&mut candidates, &mut omissions);
        Self {
            specs: candidates,
            omissions,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.specs.iter().any(|spec| spec.name == name)
    }

    pub(crate) fn iter(&self) -> std::slice::Iter<'_, ToolSpec> {
        self.specs.iter()
    }

    pub(crate) fn omissions(&self) -> &[BridgeOmission] {
        &self.omissions
    }
}

fn is_registry_support(name: &str) -> bool {
    matches!(
        name,
        CAPABILITY_SEARCH_TOOL_NAME | CAPABILITY_ACTIVATE_TOOL_NAME
    )
}

fn remove_unsupported_artifact_reader(
    specs: &mut Vec<ToolSpec>,
    omissions: &mut Vec<BridgeOmission>,
) {
    // The lister only describes artifacts; like the reader, it needs some
    // other bridged tool whose output could have produced one.
    if specs
        .iter()
        .any(|spec| spec.name != ARTIFACT_READ_TOOL_NAME && spec.name != ARTIFACT_LIST_TOOL_NAME)
    {
        return;
    }
    specs.retain(|spec| {
        omissions.push(BridgeOmission::NoSupportingTool {
            name: spec.name.clone(),
        });
        false
    });
}

fn normalize_name(name: &str) -> String {
    agent_runtime_core::tool::provider_tool_name(name)
}

/// Instructions and the tool names whose whole fragments exceeded the budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BridgeGuidance {
    pub(crate) text: String,
    /// Dropped names retain the fragments' authored order.
    // Task 5 reports guidance diagnostics with the selection diagnostics.
    #[allow(dead_code)]
    pub(crate) dropped: Vec<&'static str>,
}

pub(crate) fn instructions(bridged: &BridgedSet) -> BridgeGuidance {
    let fragments = cli_bridge_instruction_fragments()
        .into_iter()
        .filter(|(name, _)| bridged.contains(name))
        .collect::<Vec<_>>();
    assemble_instructions(&fragments)
}

fn framing() -> String {
    format!(
        "These tools are provided by Smith, the host this CLI is running under. \
         They appear as tools of the `{BRIDGE_SERVER_NAME}` MCP server. Where the guidance \
         below names a tool, it means the tool of that name on that server."
    )
}

fn assemble_instructions(fragments: &[(&'static str, &str)]) -> BridgeGuidance {
    let framing = framing();
    let mut count = fragments.len();
    let mut chars = framing.chars().count()
        + fragments
            .iter()
            .map(|(_, body)| 2 + body.chars().count())
            .sum::<usize>();
    while count > 0 && chars > MAX_INSTRUCTION_CHARS {
        count -= 1;
        chars -= 2 + fragments[count].1.chars().count();
    }
    let mut text = String::new();
    if count > 0 {
        text.push_str(&framing);
        for (_, body) in &fragments[..count] {
            text.push_str("\n\n");
            text.push_str(body);
        }
    }
    BridgeGuidance {
        text,
        dropped: fragments[count..].iter().map(|(name, _)| *name).collect(),
    }
}

#[cfg(test)]
mod tests {
    use agent_runtime::context::FragmentContent;
    use agent_runtime::harness::{
        CreateGoalTool, GetGoalTool, QUESTIONNAIRE_TOOL_NAME, UpdateGoalTool, WriteTodosTool,
    };
    use agent_runtime_core::tool::{Tool, ToolEffects};
    use serde_json::json;

    use super::*;
    use crate::advisor::ADVISOR_TOOL_NAME;
    use crate::delegation::AGENT_TOOL_NAME;

    fn spec(name: &str) -> ToolSpec {
        ToolSpec::new(
            name,
            format!("Test tool {name}"),
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
            ToolEffects::read_only(),
        )
    }

    fn select(names: &[&str], skills: bool) -> BridgedSet {
        BridgedSet::new(names.iter().map(|name| spec(name)).collect(), skills)
    }

    fn names(bridged: &BridgedSet) -> Vec<&str> {
        bridged.iter().map(|spec| spec.name.as_str()).collect()
    }

    #[test]
    fn authorized_session_selection_matrix() {
        let root = vec![
            "read",
            "list",
            "search",
            "edit",
            "shell",
            "task_output",
            "task_stop",
            WRITE_TODOS_TOOL_NAME,
            GET_GOAL_TOOL_NAME,
            CREATE_GOAL_TOOL_NAME,
            UPDATE_GOAL_TOOL_NAME,
            QUESTIONNAIRE_TOOL_NAME,
            AGENT_TOOL_NAME,
            ADVISOR_TOOL_NAME,
            ARTIFACT_READ_TOOL_NAME,
            ARTIFACT_LIST_TOOL_NAME,
            CAPABILITY_SEARCH_TOOL_NAME,
            CAPABILITY_ACTIVATE_TOOL_NAME,
            "mcp__docs__lookup",
            "generate_image",
        ];
        let root_served = vec![
            QUESTIONNAIRE_TOOL_NAME,
            AGENT_TOOL_NAME,
            ADVISOR_TOOL_NAME,
            ARTIFACT_READ_TOOL_NAME,
            ARTIFACT_LIST_TOOL_NAME,
            CAPABILITY_SEARCH_TOOL_NAME,
            CAPABILITY_ACTIVATE_TOOL_NAME,
            "mcp__docs__lookup",
            "generate_image",
        ];
        let cases = [
            ("root", root.clone(), true, root_served.clone()),
            (
                "no skills",
                root.clone(),
                false,
                root_served
                    .iter()
                    .copied()
                    .filter(|name| !is_registry_support(name))
                    .collect(),
            ),
            (
                "child",
                vec!["mcp__docs__lookup", ARTIFACT_READ_TOOL_NAME],
                false,
                vec!["mcp__docs__lookup", ARTIFACT_READ_TOOL_NAME],
            ),
            (
                "read-only",
                vec![
                    "read",
                    "list",
                    "search",
                    QUESTIONNAIRE_TOOL_NAME,
                    AGENT_TOOL_NAME,
                    ADVISOR_TOOL_NAME,
                    ARTIFACT_READ_TOOL_NAME,
                    "mcp__docs__lookup",
                ],
                false,
                vec![
                    QUESTIONNAIRE_TOOL_NAME,
                    AGENT_TOOL_NAME,
                    ADVISOR_TOOL_NAME,
                    ARTIFACT_READ_TOOL_NAME,
                    "mcp__docs__lookup",
                ],
            ),
            (
                "delegation off",
                root.iter()
                    .copied()
                    .filter(|name| *name != AGENT_TOOL_NAME)
                    .collect(),
                true,
                root_served
                    .iter()
                    .copied()
                    .filter(|name| *name != AGENT_TOOL_NAME)
                    .collect(),
            ),
            (
                "no advisor",
                root.iter()
                    .copied()
                    .filter(|name| *name != ADVISOR_TOOL_NAME)
                    .collect(),
                true,
                root_served
                    .iter()
                    .copied()
                    .filter(|name| *name != ADVISOR_TOOL_NAME)
                    .collect(),
            ),
            (
                "only excluded and unsupported",
                CORE_TOOL_NAMES
                    .into_iter()
                    .chain([
                        ARTIFACT_READ_TOOL_NAME,
                        CAPABILITY_SEARCH_TOOL_NAME,
                        CAPABILITY_ACTIVATE_TOOL_NAME,
                    ])
                    .collect(),
                false,
                vec![],
            ),
            (
                "skills authorize registry",
                vec![CAPABILITY_SEARCH_TOOL_NAME, CAPABILITY_ACTIVATE_TOOL_NAME],
                true,
                vec![CAPABILITY_SEARCH_TOOL_NAME, CAPABILITY_ACTIVATE_TOOL_NAME],
            ),
            (
                "only artifact reader",
                vec![ARTIFACT_READ_TOOL_NAME],
                false,
                vec![],
            ),
            (
                "only artifact tools",
                vec![ARTIFACT_READ_TOOL_NAME, ARTIFACT_LIST_TOOL_NAME],
                false,
                vec![],
            ),
            ("everything denied", vec![], false, vec![]),
            ("empty input with skills", vec![], true, vec![]),
        ];
        for (label, input, skills, expected) in cases {
            let bridged = select(&input, skills);
            assert_eq!(names(&bridged), expected, "{label}");
            assert_eq!(bridged.is_empty(), expected.is_empty(), "{label}");
            assert_eq!(
                bridged.iter().cloned().collect::<Vec<_>>(),
                expected.iter().map(|name| spec(name)).collect::<Vec<_>>(),
                "{label}"
            );
            for name in &input {
                assert_eq!(
                    bridged.contains(name),
                    expected.contains(name),
                    "{label}: {name}"
                );
            }
        }
    }

    #[test]
    fn policy_omissions_report_exact_reasons() {
        for (input, skills, expected) in [
            (
                vec!["read", WRITE_TODOS_TOOL_NAME, GET_GOAL_TOOL_NAME],
                false,
                vec![
                    BridgeOmission::Excluded {
                        name: "read".into(),
                    },
                    BridgeOmission::Excluded {
                        name: WRITE_TODOS_TOOL_NAME.into(),
                    },
                    BridgeOmission::Excluded {
                        name: GET_GOAL_TOOL_NAME.into(),
                    },
                ],
            ),
            (
                vec![CAPABILITY_SEARCH_TOOL_NAME, CAPABILITY_ACTIVATE_TOOL_NAME],
                false,
                vec![
                    BridgeOmission::NoAuthorizedSkills {
                        name: CAPABILITY_SEARCH_TOOL_NAME.into(),
                    },
                    BridgeOmission::NoAuthorizedSkills {
                        name: CAPABILITY_ACTIVATE_TOOL_NAME.into(),
                    },
                ],
            ),
            (
                vec![ARTIFACT_READ_TOOL_NAME],
                true,
                vec![BridgeOmission::NoSupportingTool {
                    name: ARTIFACT_READ_TOOL_NAME.into(),
                }],
            ),
            (
                vec![ARTIFACT_READ_TOOL_NAME, ARTIFACT_LIST_TOOL_NAME],
                true,
                vec![
                    BridgeOmission::NoSupportingTool {
                        name: ARTIFACT_READ_TOOL_NAME.into(),
                    },
                    BridgeOmission::NoSupportingTool {
                        name: ARTIFACT_LIST_TOOL_NAME.into(),
                    },
                ],
            ),
            (vec!["registry.future_tool"], false, vec![]),
        ] {
            assert_eq!(select(&input, skills).omissions(), expected);
        }
        assert_eq!(
            names(&select(&["registry.future_tool"], false)),
            ["registry.future_tool"]
        );
    }

    #[test]
    fn excluded_names_reuse_core_and_registered_tool_constants() {
        let registered = CORE_TOOL_NAMES
            .into_iter()
            .map(str::to_owned)
            .chain([
                WriteTodosTool::new().spec().name,
                GetGoalTool::new().spec().name,
                CreateGoalTool::new().spec().name,
                UpdateGoalTool::new().spec().name,
            ])
            .collect::<Vec<_>>();
        assert_eq!(excluded_tool_names().collect::<Vec<_>>(), registered);
    }

    #[test]
    fn registered_names_and_full_specs_pass_through_unchanged() {
        for (input, skills) in [
            (vec![CAPABILITY_SEARCH_TOOL_NAME], true),
            (vec!["mcp__docs__lookup"], false),
            (vec!["module.future_tool"], false),
        ] {
            let expected = input.iter().map(|name| spec(name)).collect::<Vec<_>>();
            let bridged = BridgedSet::new(expected.clone(), skills);
            assert_eq!(bridged.iter().cloned().collect::<Vec<_>>(), expected);
            assert_eq!(bridged.omissions(), []);
        }
    }

    #[test]
    fn tool_name_length_is_measured_in_bytes() {
        for (name, served) in [
            ("a".repeat(128), true),
            ("a".repeat(129), false),
            ("é".repeat(64), true),
            (format!("{}a", "é".repeat(64)), false),
        ] {
            let bridged = select(&[&name], false);
            let (expected_specs, expected_omissions) = if served {
                (vec![spec(&name)], vec![])
            } else {
                (
                    vec![],
                    vec![BridgeOmission::NameTooLong {
                        name: name.clone(),
                        bytes: name.len(),
                    }],
                )
            };
            assert_eq!(bridged.iter().cloned().collect::<Vec<_>>(), expected_specs);
            assert_eq!(bridged.omissions(), expected_omissions);
        }
    }

    #[test]
    fn collisions_omit_every_member_and_report_each_group_once() {
        for input in [vec!["a.b", "a_b"], vec!["a.b", "a_b", "a/b"]] {
            let bridged = select(&input, false);
            assert_eq!(names(&bridged), Vec::<&str>::new());
            assert_eq!(
                bridged.omissions(),
                [BridgeOmission::NameCollision {
                    normalized: "a_b".into(),
                    names: input.iter().map(|name| (*name).into()).collect(),
                }]
            );
        }
    }

    #[test]
    fn normalization_replaces_each_non_ascii_byte_and_preserves_allowed_bytes() {
        for (input, expected) in [
            ("registry.search", "registry_search"),
            ("mcp__docs__lookup", "mcp__docs__lookup"),
            ("AZaz09_-", "AZaz09_-"),
            ("é.x", "___x"),
            ("a b/c:d", "a_b_c_d"),
        ] {
            assert_eq!(normalize_name(input), expected);
        }
    }

    #[test]
    fn ineligible_names_do_not_collide_with_served_tools() {
        let bridged = select(&[CAPABILITY_SEARCH_TOOL_NAME, "registry_search"], false);
        assert_eq!(names(&bridged), ["registry_search"]);
        assert_eq!(
            bridged.omissions(),
            [BridgeOmission::NoAuthorizedSkills {
                name: CAPABILITY_SEARCH_TOOL_NAME.into(),
            }]
        );
    }

    #[test]
    fn artifact_reader_cannot_survive_the_last_valid_capability() {
        let long = "a".repeat(129);
        for (input, expected) in [
            (
                vec![ARTIFACT_READ_TOOL_NAME, &long],
                vec![
                    BridgeOmission::NameTooLong {
                        name: long.clone(),
                        bytes: 129,
                    },
                    BridgeOmission::NoSupportingTool {
                        name: ARTIFACT_READ_TOOL_NAME.into(),
                    },
                ],
            ),
            (
                vec![ARTIFACT_READ_TOOL_NAME, "a.b", "a_b"],
                vec![
                    BridgeOmission::NameCollision {
                        normalized: "a_b".into(),
                        names: vec!["a.b".into(), "a_b".into()],
                    },
                    BridgeOmission::NoSupportingTool {
                        name: ARTIFACT_READ_TOOL_NAME.into(),
                    },
                ],
            ),
        ] {
            let bridged = select(&input, false);
            assert_eq!(names(&bridged), Vec::<&str>::new());
            assert_eq!(bridged.omissions(), expected);
        }
        let bridged = select(
            &[ARTIFACT_READ_TOOL_NAME, CAPABILITY_SEARCH_TOOL_NAME],
            true,
        );
        assert_eq!(
            names(&bridged),
            [ARTIFACT_READ_TOOL_NAME, CAPABILITY_SEARCH_TOOL_NAME]
        );
        assert_eq!(bridged.omissions(), []);
    }

    #[test]
    fn support_tools_participate_in_name_collisions() {
        let bridged = select(
            &[
                ARTIFACT_READ_TOOL_NAME,
                CAPABILITY_SEARCH_TOOL_NAME,
                "registry_search",
            ],
            true,
        );
        assert_eq!(names(&bridged), Vec::<&str>::new());
        assert_eq!(
            bridged.omissions(),
            [
                BridgeOmission::NameCollision {
                    normalized: "registry_search".into(),
                    names: vec![CAPABILITY_SEARCH_TOOL_NAME.into(), "registry_search".into()],
                },
                BridgeOmission::NoSupportingTool {
                    name: ARTIFACT_READ_TOOL_NAME.into()
                },
            ]
        );

        let bridged = select(
            &[ARTIFACT_READ_TOOL_NAME, "artifact_read", AGENT_TOOL_NAME],
            false,
        );
        assert_eq!(names(&bridged), [AGENT_TOOL_NAME]);
        assert_eq!(
            bridged.omissions(),
            [BridgeOmission::NameCollision {
                normalized: "artifact_read".into(),
                names: vec![ARTIFACT_READ_TOOL_NAME.into(), "artifact_read".into()],
            }]
        );
    }

    #[test]
    fn selection_and_diagnostics_are_stable_across_runs() {
        let long = "x".repeat(129);
        let input = [
            "z.z",
            "z_z",
            "read",
            "mcp__docs__lookup",
            "a.b",
            "a_b",
            &long,
        ];
        let expected = BridgedSet {
            specs: vec![spec("mcp__docs__lookup")],
            omissions: vec![
                BridgeOmission::Excluded {
                    name: "read".into(),
                },
                BridgeOmission::NameTooLong {
                    name: long.clone(),
                    bytes: 129,
                },
                BridgeOmission::NameCollision {
                    normalized: "z_z".into(),
                    names: vec!["z.z".into(), "z_z".into()],
                },
                BridgeOmission::NameCollision {
                    normalized: "a_b".into(),
                    names: vec!["a.b".into(), "a_b".into()],
                },
            ],
        };
        assert_eq!(select(&input, false), expected);
        assert_eq!(select(&input, false), select(&input, false));
    }

    #[test]
    fn guidance_contains_exactly_the_bridged_fragments_in_authored_order() {
        assert_eq!(BRIDGE_SERVER_NAME, "smith");
        assert_eq!(
            framing(),
            "These tools are provided by Smith, the host this CLI is running under. \
            They appear as tools of the `smith` MCP server. Where the guidance \
            below names a tool, it means the tool of that name on that server."
        );
        let fragments = cli_bridge_instruction_fragments();
        assert_eq!(
            fragments.map(|(name, _)| name),
            [QUESTIONNAIRE_TOOL_NAME, AGENT_TOOL_NAME, ADVISOR_TOOL_NAME]
        );
        for mask in 0usize..8 {
            let selected = fragments
                .iter()
                .enumerate()
                .filter(|(index, _)| mask & (1usize << *index) != 0)
                .map(|(_, fragment)| *fragment)
                .collect::<Vec<_>>();
            // Registration order does not change guidance order.
            let tools = selected
                .iter()
                .rev()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>();
            let guidance = instructions(&select(&tools, false));
            let expected_text = if selected.is_empty() {
                String::new()
            } else {
                std::iter::once(framing())
                    .chain(selected.iter().map(|(_, body)| (*body).to_owned()))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            assert_eq!(
                guidance,
                BridgeGuidance {
                    text: expected_text,
                    dropped: vec![]
                }
            );
            for (name, body) in fragments {
                assert_eq!(
                    guidance.text.matches(body).count(),
                    usize::from(tools.contains(&name))
                );
            }
            assert!(guidance.text.chars().count() <= 4_000);
        }
        assert_eq!(
            instructions(&select(&["mcp__docs__lookup"], false)),
            BridgeGuidance {
                text: String::new(),
                dropped: vec![],
            }
        );
    }

    #[test]
    fn guidance_names_only_capabilities_that_are_bridged() {
        for (tool, excluded) in [
            (AGENT_TOOL_NAME, vec!["advisor"]),
            (ADVISOR_TOOL_NAME, vec!["agent"]),
            (QUESTIONNAIRE_TOOL_NAME, vec!["agent", "advisor"]),
        ] {
            let guidance = instructions(&select(&[tool], false));
            for name in excluded {
                assert!(
                    !guidance
                        .text
                        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                        .any(|word| word == name),
                    "{}",
                    guidance.text
                );
            }
        }
    }

    #[test]
    fn guidance_excludes_the_system_prompt_project_instructions_and_memory() {
        let guidance = instructions(&select(
            &[QUESTIONNAIRE_TOOL_NAME, AGENT_TOOL_NAME, ADVISOR_TOOL_NAME],
            false,
        ));
        // These bodies come directly from prompt.rs's stable policy constants.
        for fragment in crate::prompt::stable_fragments() {
            let FragmentContent::Text(body) = fragment.content else {
                panic!("stable policy must be text")
            };
            assert_eq!(guidance.text.matches(&body).count(), 0, "{}", fragment.id);
        }
        let context = crate::prompt::DynamicPromptContext {
            project_instructions: Some(
                crate::project_instructions::ProjectInstructionsSnapshot::from_body(
                    "private-project-instructions-bridge-test",
                )
                .expect("bounded instructions"),
            ),
            memory: Some("private-memory-index-bridge-test".into()),
            ..crate::prompt::DynamicPromptContext::default()
        };
        for fragment in crate::prompt::dynamic_fragments(&context) {
            let FragmentContent::Text(body) = fragment.content else {
                panic!("dynamic policy must be text")
            };
            assert_eq!(guidance.text.matches(&body).count(), 0, "{}", fragment.id);
        }
        assert_eq!(
            guidance
                .text
                .matches("private-project-instructions-bridge-test")
                .count(),
            0
        );
        assert_eq!(
            guidance
                .text
                .matches("private-memory-index-bridge-test")
                .count(),
            0
        );
    }

    #[test]
    fn guidance_follows_the_served_set_after_name_checks() {
        let bridged = select(
            &[QUESTIONNAIRE_TOOL_NAME, "ask.user", AGENT_TOOL_NAME],
            false,
        );
        assert_eq!(names(&bridged), [AGENT_TOOL_NAME]);
        let [(_, questionnaire), (_, delegation), (_, advisor)] =
            cli_bridge_instruction_fragments();
        let guidance = instructions(&bridged);
        assert_eq!(
            guidance,
            BridgeGuidance {
                text: format!("{}\n\n{delegation}", framing()),
                dropped: vec![],
            }
        );
        assert_eq!(guidance.text.matches(questionnaire).count(), 0);
        assert_eq!(guidance.text.matches(advisor).count(), 0);
    }

    #[test]
    fn guidance_drops_whole_trailing_fragments_and_counts_characters() {
        let exact = "é".repeat(MAX_INSTRUCTION_CHARS - framing().chars().count() - 2);
        let guidance =
            assemble_instructions(&[("first", &exact), ("second", "tail"), ("third", "more")]);
        assert_eq!(
            guidance,
            BridgeGuidance {
                text: format!("{}\n\n{exact}", framing()),
                dropped: vec!["second", "third"],
            }
        );
        assert_eq!(guidance.text.chars().count(), 4_000);
        let too_long = format!("{exact}é");
        assert_eq!(
            assemble_instructions(&[("first", &too_long), ("second", "short")]),
            BridgeGuidance {
                text: String::new(),
                dropped: vec!["first", "second"],
            }
        );
        assert_eq!(
            assemble_instructions(&[]),
            BridgeGuidance {
                text: String::new(),
                dropped: vec![]
            }
        );
    }
}
