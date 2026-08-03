//! An append-only record of what each session spent.
//!
//! Smith's compaction and cache behavior is now driven by thresholds — a
//! trigger percentage, a notice threshold, a stable-prefix ordering. None of
//! those can be evaluated from a single session, and the previous six-turn
//! summarization default survived unexamined for exactly that reason: nothing
//! recorded whether it helped.
//!
//! One line per session, counts and identities only. No prompt text, no tool
//! arguments, no file contents, no paths from the workspace — so this inherits
//! the journal's redaction guarantees by carrying nothing that could need
//! redacting in the first place.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::status::{SessionUsage, counter_label};

/// Record wire version.
pub const USAGE_RECORD_SCHEMA_VERSION: u32 = 1;

/// One session's bounded usage record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUsageRecord {
    /// Wire schema version.
    pub schema_version: u32,
    /// Session identity.
    pub session: String,
    /// Serving provider, when one resolved.
    pub provider: Option<String>,
    /// Model identity.
    pub model: String,
    /// Active agent profile.
    pub agent: String,
    /// Turns that produced provider usage.
    pub turns: u32,
    /// Whether the counters are provider-reported.
    pub reported: bool,
    /// Per-counter totals, keyed by stable counter label.
    pub totals: std::collections::BTreeMap<String, u64>,
    /// Context compactions observed.
    pub compactions: u32,
    /// Tokens those compactions reclaimed.
    pub reclaimed_tokens: u64,
}

impl SessionUsageRecord {
    /// Builds a record from an observed session.
    pub fn new(
        session: impl Into<String>,
        provider: Option<String>,
        model: impl Into<String>,
        agent: impl Into<String>,
        usage: &SessionUsage,
    ) -> Self {
        Self {
            schema_version: USAGE_RECORD_SCHEMA_VERSION,
            session: session.into(),
            provider,
            model: model.into(),
            agent: agent.into(),
            turns: usage.turns,
            reported: usage.reported,
            totals: usage
                .totals
                .iter()
                .map(|(kind, value)| (counter_label(*kind).to_owned(), *value))
                .collect(),
            compactions: usage.compactions,
            reclaimed_tokens: usage.reclaimed_tokens,
        }
    }
}

/// The standard log path under a user state root.
pub fn default_path(state_root: &Path) -> PathBuf {
    state_root.join("usage.jsonl")
}

/// Appends one record, creating the parent directory if needed.
///
/// Analytics must never be able to fail a session, so every error is returned
/// for the caller to ignore deliberately rather than propagated into shutdown.
pub fn append(path: &Path, record: &SessionUsageRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_string(record)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())
}

/// Reads every record, skipping any line that no longer parses.
pub fn read_all(path: &Path) -> Vec<SessionUsageRecord> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use agent_runtime_core::usage::CounterKind;

    use super::*;

    fn usage() -> SessionUsage {
        let mut totals = std::collections::BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 12_000);
        totals.insert(CounterKind::InputCached, 90_000);
        totals.insert(CounterKind::Output, 3_400);
        SessionUsage {
            turns: 7,
            reported: true,
            totals,
            compactions: 1,
            reclaimed_tokens: 40_000,
        }
    }

    #[test]
    fn a_record_round_trips_through_the_log() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = default_path(dir.path());
        let record = SessionUsageRecord::new(
            "session-1",
            Some("local".to_owned()),
            "example-model",
            "build",
            &usage(),
        );

        append(&path, &record).expect("append");
        append(&path, &record).expect("append again");

        let all = read_all(&path);
        assert_eq!(all.len(), 2, "the log is append-only");
        assert_eq!(all[0], record);
        assert_eq!(all[0].totals["cached"], 90_000);
        assert_eq!(all[0].compactions, 1);
    }

    #[test]
    fn a_record_carries_no_conversation_content() {
        let record = SessionUsageRecord::new(
            "session-1",
            Some("local".to_owned()),
            "example-model",
            "build",
            &usage(),
        );
        let encoded = serde_json::to_string(&record).expect("encode");
        // Counts and identities are the whole payload. Anything that could
        // require redaction should never have reached this file.
        for field in ["message", "content", "prompt", "arguments", "path"] {
            assert!(!encoded.contains(field), "`{field}` leaked into {encoded}");
        }
    }

    #[test]
    fn an_unreported_session_is_not_recorded_as_confident() {
        let usage = SessionUsage {
            reported: false,
            ..usage()
        };
        let record = SessionUsageRecord::new("s", None, "example-model", "build", &usage);
        assert!(!record.reported);
        assert!(
            usage.render().expect("a summary").contains("estimated"),
            "an unreported session must say so"
        );
    }

    #[test]
    fn a_short_session_renders_the_expected_line() {
        let mut totals = std::collections::BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 860);
        totals.insert(CounterKind::Output, 13);
        let usage = SessionUsage {
            turns: 1,
            reported: true,
            totals,
            compactions: 0,
            reclaimed_tokens: 0,
        };
        assert_eq!(
            usage.render().expect("a summary"),
            "1 turn(s) · input 860 · output 13"
        );
    }

    #[test]
    fn cache_counters_are_named_separately() {
        // Cache reads and writes are the numbers worth watching: they are the
        // direct evidence that the stable-prefix ordering is doing its job.
        let mut totals = std::collections::BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 1_200);
        totals.insert(CounterKind::InputCached, 90_000);
        totals.insert(CounterKind::CacheWrite, 2_000);
        totals.insert(CounterKind::Output, 400);
        let usage = SessionUsage {
            turns: 3,
            reported: true,
            totals,
            compactions: 0,
            reclaimed_tokens: 0,
        };
        assert_eq!(
            usage.render().expect("a summary"),
            "3 turn(s) · input 1.2k · cached 90k · cache-write 2k · output 400"
        );
    }

    #[test]
    fn an_empty_session_renders_nothing() {
        assert!(SessionUsage::default().render().is_none());
    }
}
