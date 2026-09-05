//! The built-in catalog of installed coding agents Smith can run turns on.
//!
//! A CLI agent is selected the same way any other model is — by id — so it
//! needs no declaration to use:
//!
//! ```text
//! cli/claude-code/sonnet
//! cli/codex/gpt-6-astra
//! ```
//!
//! The model lists are compiled in rather than configured. A vendor's model
//! aliases change rarely, asking the CLI would cost a process launch every
//! time a picker opens, and requiring every user to write a TOML table before
//! they can try an installed agent is the friction this namespace exists to
//! remove. `[harness.<kind>]` stays available for the things that genuinely
//! vary per machine: a non-standard executable path, extra arguments, the
//! environment, and whether the CLI may run its own tools.

/// Prefix marking a model id as an installed CLI agent rather than a provider
/// model.
pub const CLI_MODEL_PREFIX: &str = "cli/";

/// One installed coding agent Smith knows how to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CliAgentCatalogEntry {
    /// Stable kind name, used in both the model id and `[harness.<kind>]`.
    pub kind: &'static str,
    /// Program name looked up on `PATH` when no executable is configured.
    pub program: &'static str,
    /// Models this agent accepts, offered wherever models are listed.
    pub models: &'static [&'static str],
    /// Human name for the agent, used as the model row's name.
    pub description: &'static str,
}

/// Every installed agent Smith can drive.
pub const CLI_AGENTS: &[CliAgentCatalogEntry] = &[
    CliAgentCatalogEntry {
        kind: "claude-code",
        program: "claude",
        // Aliases rather than dated model names: the CLI resolves an alias to
        // the current model, so this list does not go stale every release.
        models: &["sonnet", "opus", "haiku", "fable"],
        description: "Claude Code",
    },
    CliAgentCatalogEntry {
        kind: "codex",
        program: "codex",
        models: &["gpt-6-astra", "gpt-5.6-sol"],
        description: "Codex",
    },
];

/// Context limits Smith plans against for a CLI agent turn.
///
/// The CLI owns its own context — it decides what to send and when to
/// compact — so these are not the model's real limits and are not used to
/// truncate anything. They exist because the runtime refuses to start without
/// limits, and requiring every user to write a `[models]` table for an agent
/// whose context Smith does not manage would be friction with nothing behind
/// it.
pub const CLI_AGENT_CONTEXT_TOKENS: u32 = 200_000;
/// Input budget recorded for a CLI agent turn. See
/// [`CLI_AGENT_CONTEXT_TOKENS`].
pub const CLI_AGENT_MAX_INPUT_TOKENS: u32 = 180_000;
/// Output budget recorded for a CLI agent turn. See
/// [`CLI_AGENT_CONTEXT_TOKENS`].
pub const CLI_AGENT_MAX_OUTPUT_TOKENS: u32 = 32_000;

/// Looks up one agent by kind.
pub fn catalog_entry(kind: &str) -> Option<&'static CliAgentCatalogEntry> {
    CLI_AGENTS.iter().find(|entry| entry.kind == kind)
}

/// Splits `cli/<kind>/<model>` into its parts.
///
/// Returns `None` for anything that is not a CLI agent id, so an ordinary
/// provider model passes through untouched.
pub fn parse_cli_model_id(id: &str) -> Option<(&'static CliAgentCatalogEntry, String)> {
    let rest = id.strip_prefix(CLI_MODEL_PREFIX)?;
    let (kind, model) = rest.split_once('/')?;
    let entry = catalog_entry(kind)?;
    if model.is_empty() {
        return None;
    }
    Some((entry, model.to_owned()))
}

/// Builds the id naming one agent's model.
pub fn cli_model_id(kind: &str, model: &str) -> String {
    format!("{CLI_MODEL_PREFIX}{kind}/{model}")
}

/// Every selectable CLI agent model id, in catalog order.
pub fn cli_model_ids() -> Vec<String> {
    CLI_AGENTS
        .iter()
        .flat_map(|entry| {
            entry
                .models
                .iter()
                .map(move |model| cli_model_id(entry.kind, model))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cli_model_id_round_trips_through_its_parts() {
        let id = cli_model_id("claude-code", "sonnet");
        assert_eq!(id, "cli/claude-code/sonnet");

        let (entry, model) = parse_cli_model_id(&id).expect("a catalog entry");
        assert_eq!(entry.kind, "claude-code");
        assert_eq!(entry.program, "claude");
        assert_eq!(model, "sonnet");
    }

    #[test]
    fn a_provider_model_is_not_mistaken_for_a_cli_agent() {
        assert!(parse_cli_model_id("gemini-3.6-flash").is_none());
        assert!(parse_cli_model_id("openrouter/anthropic/claude").is_none());
        // Right prefix, unknown agent.
        assert!(parse_cli_model_id("cli/emacs/sonnet").is_none());
        // Right agent, no model.
        assert!(parse_cli_model_id("cli/codex/").is_none());
    }

    #[test]
    fn every_catalog_entry_offers_at_least_one_model() {
        for entry in CLI_AGENTS {
            assert!(
                !entry.models.is_empty(),
                "`{}` would appear in a picker with nothing to choose",
                entry.kind
            );
        }
        assert!(cli_model_ids().contains(&"cli/codex/gpt-6-astra".to_owned()));
    }
}
