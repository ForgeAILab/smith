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
///
/// Bumped to 5 to retain each root/child binding across model switches.
/// Version 4 added separately attributed synthetic cache usage. Version 3
/// added cache miss diagnostics: `cache_miss_count` and
/// `cache_rebilled_tokens` are optional so an older record means "no cache
/// evidence", never a verified zero. Version 2 added delegated usage:
/// `delegated_totals` and
/// `delegated_contributors` are new fields. Both carry `#[serde(default)]`
/// so [`read_all`] stays tolerant of version-1 lines, which simply have no
/// delegated usage to report.
pub const USAGE_RECORD_SCHEMA_VERSION: u32 = 5;

/// Durable binding attribution without catalog prices or conversation content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingUsageRecord {
    /// Absent for usage whose serving binding could not be resolved.
    pub provider: Option<String>,
    /// Serving model or the explanation for an unresolved binding.
    pub model: String,
    /// Disjoint counters keyed by the same labels as the root rollup.
    pub totals: std::collections::BTreeMap<String, u64>,
}

/// One session's bounded usage record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    /// User-started root conversation turns for new records. Earlier Smith
    /// builds counted provider attempts with input, even in schema v5; readers
    /// must use the session identity rather than this field to restore turns.
    pub turns: u32,
    /// Whether the counters are provider-reported.
    pub reported: bool,
    /// Per-counter totals, keyed by stable counter label.
    pub totals: std::collections::BTreeMap<String, u64>,
    /// Binding counters survive changes to the top-level, last-root identity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<BindingUsageRecord>,
    /// Provider counters from synthetic cache operations, excluded from root
    /// and delegated turn usage while retained in session spend.
    #[serde(default)]
    pub synthetic_totals: std::collections::BTreeMap<String, u64>,
    /// Synthetic counters partitioned by Runtime's stable typed purpose.
    #[serde(default)]
    pub synthetic_by_purpose:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>>,
    /// Advisor provider counters, separately attributed from root turns.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub advisor_totals: std::collections::BTreeMap<String, u64>,
    /// Context compactions observed.
    pub compactions: u32,
    /// Tokens those compactions reclaimed.
    pub reclaimed_tokens: u64,
    /// Per-counter usage delegated children reported, keyed by the same
    /// stable counter labels as `totals`. Kept separate rather than merged
    /// into `totals`, matching `SessionUsage`'s own separation.
    #[serde(default)]
    pub delegated_totals: std::collections::BTreeMap<String, u64>,
    /// Distinct children that reported delegated usage.
    #[serde(default)]
    pub delegated_contributors: u32,
    /// Positive canonical miss count, absent when the session supplied no
    /// cache-miss evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_miss_count: Option<u32>,
    /// Positive canonical re-billed tokens, absent when no miss evidence was
    /// available. These never enter `totals`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_rebilled_tokens: Option<u64>,
}

// A wire mirror lets every reader path migrate old lines, including callers
// using serde directly, without changing the public record's field access.
#[derive(Deserialize)]
struct SessionUsageRecordWire {
    schema_version: u32,
    session: String,
    provider: Option<String>,
    model: String,
    agent: String,
    turns: u32,
    reported: bool,
    totals: std::collections::BTreeMap<String, u64>,
    #[serde(default)]
    bindings: Vec<BindingUsageRecord>,
    #[serde(default)]
    synthetic_totals: std::collections::BTreeMap<String, u64>,
    #[serde(default)]
    synthetic_by_purpose:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>>,
    #[serde(default)]
    advisor_totals: std::collections::BTreeMap<String, u64>,
    compactions: u32,
    reclaimed_tokens: u64,
    #[serde(default)]
    delegated_totals: std::collections::BTreeMap<String, u64>,
    #[serde(default)]
    delegated_contributors: u32,
    #[serde(default)]
    cache_miss_count: Option<u32>,
    #[serde(default)]
    cache_rebilled_tokens: Option<u64>,
}

impl<'de> Deserialize<'de> for SessionUsageRecord {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = SessionUsageRecordWire::deserialize(deserializer)?;
        let mut record = Self {
            schema_version: wire.schema_version,
            session: wire.session,
            provider: wire.provider,
            model: wire.model,
            agent: wire.agent,
            turns: wire.turns,
            reported: wire.reported,
            totals: wire.totals,
            bindings: wire.bindings,
            synthetic_totals: wire.synthetic_totals,
            synthetic_by_purpose: wire.synthetic_by_purpose,
            advisor_totals: wire.advisor_totals,
            compactions: wire.compactions,
            reclaimed_tokens: wire.reclaimed_tokens,
            delegated_totals: wire.delegated_totals,
            delegated_contributors: wire.delegated_contributors,
            cache_miss_count: wire.cache_miss_count,
            cache_rebilled_tokens: wire.cache_rebilled_tokens,
        };
        if (1..=4).contains(&record.schema_version) && record.bindings.is_empty() {
            record.attribute_legacy_totals();
        }
        Ok(record)
    }
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
        let mut record = Self {
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
            bindings: Vec::new(),
            synthetic_totals: usage
                .synthetic_totals
                .iter()
                .map(|(kind, value)| (counter_label(*kind).to_owned(), *value))
                .collect(),
            synthetic_by_purpose: usage
                .synthetic_by_purpose
                .iter()
                .map(|(purpose, totals)| {
                    (
                        purpose.as_str().to_owned(),
                        totals
                            .iter()
                            .map(|(kind, value)| (counter_label(*kind).to_owned(), *value))
                            .collect(),
                    )
                })
                .collect(),
            advisor_totals: usage
                .advisor_totals
                .iter()
                .map(|(kind, value)| (counter_label(*kind).to_owned(), *value))
                .collect(),
            compactions: usage.compactions,
            reclaimed_tokens: usage.reclaimed_tokens,
            delegated_totals: usage
                .delegated_totals
                .iter()
                .map(|(kind, value)| (counter_label(*kind).to_owned(), *value))
                .collect(),
            delegated_contributors: usage.delegated_contributors,
            cache_miss_count: (usage.cache_miss_count > 0).then_some(usage.cache_miss_count),
            cache_rebilled_tokens: (usage.cache_rebilled_tokens > 0)
                .then_some(usage.cache_rebilled_tokens),
        };
        for binding in &usage.bindings {
            if binding.totals.is_empty() {
                continue;
            }
            let totals = binding
                .totals
                .iter()
                .map(|(kind, value)| (counter_label(*kind).to_owned(), *value));
            if let Some(existing) = record
                .bindings
                .iter_mut()
                .find(|entry| entry.provider == binding.provider && entry.model == binding.model)
            {
                for (kind, value) in totals {
                    let total = existing.totals.entry(kind).or_default();
                    *total = total.saturating_add(value);
                }
            } else {
                record.bindings.push(BindingUsageRecord {
                    provider: binding.provider.clone(),
                    model: binding.model.clone(),
                    totals: totals.collect(),
                });
            }
        }
        if record.bindings.is_empty() {
            record.attribute_legacy_totals();
        }
        record
    }

    fn attribute_legacy_totals(&mut self) {
        if !self.totals.is_empty() {
            self.bindings.push(BindingUsageRecord {
                provider: self.provider.clone(),
                model: self.model.clone(),
                totals: self.totals.clone(),
            });
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
            ..SessionUsage::default()
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
    fn synthetic_cache_usage_round_trips_without_entering_root_totals() {
        let mut usage = usage();
        usage
            .synthetic_totals
            .insert(CounterKind::InputCached, 80_000);
        usage.synthetic_by_purpose.insert(
            agent_runtime_core::provider::ProviderAttemptPurpose::CacheKeepalive,
            std::collections::BTreeMap::from([(CounterKind::InputCached, 80_000)]),
        );
        let record = SessionUsageRecord::new(
            "session-1",
            Some("openai".to_owned()),
            "gpt-5.6",
            "build",
            &usage,
        );

        assert_eq!(record.schema_version, 5);
        assert_eq!(record.totals["cached"], 90_000);
        assert_eq!(record.synthetic_totals["cached"], 80_000);
        assert_eq!(
            record.synthetic_by_purpose["cache_keepalive"]["cached"],
            80_000
        );
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
            synthetic_totals: std::collections::BTreeMap::new(),
            synthetic_by_purpose: std::collections::BTreeMap::new(),
            compactions: 0,
            reclaimed_tokens: 0,
            ..SessionUsage::default()
        };
        assert_eq!(
            usage.render().expect("a summary"),
            "1 turn · input 860 · output 13"
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
            ..SessionUsage::default()
        };
        assert_eq!(
            usage.render().expect("a summary"),
            "3 turns · input 1.2k · cached 90k · cache-write 2k · output 400"
        );
    }

    #[test]
    fn cache_miss_diagnostics_are_persisted_outside_usage_totals() {
        let usage = SessionUsage {
            cache_miss_count: 2,
            cache_rebilled_tokens: 105_000,
            ..usage()
        };
        let record = SessionUsageRecord::new("session-1", None, "model", "build", &usage);
        let encoded = serde_json::to_string(&record).expect("encode");
        assert!(encoded.contains("cache_miss_count"));
        assert!(encoded.contains("cache_rebilled_tokens"));
        assert_eq!(record.totals.get("cache_miss_count"), None);
        assert_eq!(record.totals.get("cache_rebilled_tokens"), None);
        assert_eq!(record.cache_miss_count, Some(2));
        assert_eq!(record.cache_rebilled_tokens, Some(105_000));
    }

    #[test]
    fn an_empty_session_renders_nothing() {
        assert!(SessionUsage::default().render().is_none());
    }

    #[test]
    fn delegated_usage_renders_a_merged_total_then_root_and_agents_sub_lines() {
        let mut totals = std::collections::BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 860);
        totals.insert(CounterKind::Output, 13);
        let mut delegated_totals = std::collections::BTreeMap::new();
        delegated_totals.insert(CounterKind::InputUncached, 140);
        delegated_totals.insert(CounterKind::Output, 7);
        let usage = SessionUsage {
            turns: 1,
            reported: true,
            totals,
            synthetic_totals: std::collections::BTreeMap::new(),
            synthetic_by_purpose: std::collections::BTreeMap::new(),
            compactions: 0,
            reclaimed_tokens: 0,
            delegated_totals,
            delegated_contributors: 4,
            advisor_totals: Default::default(),
            advisor_price: None,
            cache_miss_count: 0,
            cache_rebilled_tokens: 0,
            bindings: Vec::new(),
        };
        // The merged line names no turn count. A child's turns live with the
        // delegation coordinator, so the only one available here is the
        // root's — and printing it beside merged tokens would claim those
        // turns spent those tokens.
        assert_eq!(
            usage.render().expect("a summary"),
            "total · input 1k · output 20\n\
             \u{20}\u{20}root: 1 turn · input 860 · output 13\n\
             \u{20}\u{20}agents: 4 agents · input 140 · output 7"
        );
        assert_eq!(usage.total_tokens(), 873);
        assert_eq!(usage.merged_total_tokens(), 1_020);
        assert!(!usage.is_empty());
    }

    #[test]
    fn a_root_compaction_is_not_repeated_against_the_merged_total() {
        let mut totals = std::collections::BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 860);
        let mut delegated_totals = std::collections::BTreeMap::new();
        delegated_totals.insert(CounterKind::InputUncached, 140);
        let usage = SessionUsage {
            turns: 2,
            reported: true,
            totals,
            synthetic_totals: std::collections::BTreeMap::new(),
            synthetic_by_purpose: std::collections::BTreeMap::new(),
            compactions: 1,
            reclaimed_tokens: 40_000,
            delegated_totals,
            delegated_contributors: 1,
            advisor_totals: Default::default(),
            advisor_price: None,
            cache_miss_count: 0,
            cache_rebilled_tokens: 0,
            bindings: Vec::new(),
        };
        let rendered = usage.render().expect("a summary");
        assert_eq!(
            rendered.matches("1 compaction reclaiming 40k").count(),
            1,
            "a compaction is a root context event and is attributed once: {rendered}"
        );
        assert!(
            rendered
                .lines()
                .next()
                .is_some_and(|line| !line.contains("compaction")),
            "the merged line carries counters only: {rendered}"
        );
    }

    #[test]
    fn a_delegated_only_session_is_not_empty_even_without_root_usage() {
        let mut delegated_totals = std::collections::BTreeMap::new();
        delegated_totals.insert(CounterKind::InputUncached, 500);
        let usage = SessionUsage {
            delegated_totals,
            delegated_contributors: 1,
            ..SessionUsage::default()
        };
        assert!(!usage.is_empty());
        let rendered = usage
            .render()
            .expect("a delegated-only session still renders");
        assert!(rendered.contains("agents: 1 agent ·"), "{rendered}");
    }

    #[test]
    fn a_dormant_recovered_child_never_reaches_the_delegated_totals() {
        // A recovered child with no live stream in this process is not a
        // contributor: `App::apply_child` is never called for it, so
        // nothing ever reaches `SessionUsage::delegated_totals` on its
        // behalf. This is pinned at the record layer: a record built from a
        // no-delegation `SessionUsage` carries an empty delegated section
        // regardless of how many children the session's panel lists.
        let record = SessionUsageRecord::new(
            "session-1",
            Some("local".to_owned()),
            "example-model",
            "build",
            &usage(),
        );
        assert!(record.delegated_totals.is_empty());
        assert_eq!(record.delegated_contributors, 0);
    }

    #[test]
    fn a_version_one_record_reads_back_with_no_delegated_usage() {
        // A record written before delegated accounting existed has no
        // `delegated_totals`/`delegated_contributors` keys at all.
        let legacy = serde_json::json!({
            "schema_version": 1,
            "session": "session-old",
            "provider": "local",
            "model": "example-model",
            "agent": "build",
            "turns": 2,
            "reported": true,
            "totals": {"input": 500},
            "compactions": 0,
            "reclaimed_tokens": 0,
        });
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = default_path(dir.path());
        std::fs::write(&path, format!("{legacy}\n")).expect("seed a legacy line");

        let all = read_all(&path);
        assert_eq!(all.len(), 1);
        assert!(all[0].delegated_totals.is_empty());
        assert_eq!(all[0].delegated_contributors, 0);
        assert_eq!(all[0].cache_miss_count, None);
        assert_eq!(all[0].cache_rebilled_tokens, None);
    }

    #[test]
    fn a_version_two_record_keeps_delegated_usage_without_fabricating_cache_evidence() {
        let legacy = serde_json::json!({
            "schema_version": 2,
            "session": "session-v2",
            "provider": "local",
            "model": "example-model",
            "agent": "build",
            "turns": 3,
            "reported": true,
            "totals": {"input": 500},
            "compactions": 1,
            "reclaimed_tokens": 200,
            "delegated_totals": {"cached": 90},
            "delegated_contributors": 1
        });
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = default_path(dir.path());
        std::fs::write(&path, format!("{legacy}\n")).expect("seed a version-two line");

        let all = read_all(&path);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].delegated_totals.get("cached"), Some(&90));
        assert_eq!(all[0].delegated_contributors, 1);
        assert_eq!(all[0].cache_miss_count, None);
        assert_eq!(all[0].cache_rebilled_tokens, None);
    }

    #[test]
    fn a_version_four_record_attributes_totals_to_its_top_level_binding() {
        let legacy = serde_json::json!({
            "schema_version": 4, "session": "old", "provider": "zai",
            "model": "glm-5.3", "agent": "build", "turns": 1,
            "reported": true, "totals": { "input": 11_000, "output": 200 },
            "compactions": 0, "reclaimed_tokens": 0,
            "delegated_totals": { "input": 500 }, "delegated_contributors": 1
        });
        let record: SessionUsageRecord = serde_json::from_value(legacy.clone()).expect("version 4");
        assert_eq!(record.bindings.len(), 1);
        assert_eq!(record.bindings[0].provider.as_deref(), Some("zai"));
        assert_eq!(record.bindings[0].model, "glm-5.3");
        assert_eq!(record.bindings[0].totals, record.totals);
        let dir = tempfile::tempdir().expect("temp dir");
        let path = default_path(dir.path());
        std::fs::write(&path, format!("{legacy}\n")).expect("legacy log");
        assert_eq!(read_all(&path), vec![record]);
    }

    #[test]
    fn version_five_keeps_binding_counters_and_last_root_identity_without_prices() {
        use crate::status::{BindingUsage, Status};
        use agent_runtime_core::usage::UsageDelta;
        let mut status = Status::new("glm-5.3", "project");
        status.switch_model(Some("zai".into()), "glm-5.3");
        status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 11_000));
        status.switch_model(Some("google".into()), "gemini-3.8-flash");
        status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 12_000));
        let mut usage = status.session_usage();
        let mut child = BindingUsage::new(Some("chatgpt".into()), "gpt-6.1-sol", None);
        child.record(&UsageDelta::new().with(CounterKind::Output, 300), true);
        usage.bindings.push(child);
        usage.delegated_totals.insert(CounterKind::Output, 300);
        usage.delegated_contributors = 1;
        let record = SessionUsageRecord::new(
            "session",
            status.provider.clone(),
            &status.model,
            "build",
            &usage,
        );
        assert_eq!(record.schema_version, 5);
        assert_eq!(record.provider.as_deref(), Some("google"));
        assert_eq!(record.model, "gemini-3.8-flash");
        assert_eq!(record.bindings.len(), 3);
        assert_eq!(record.bindings[0].totals["input"], 11_000);
        assert_eq!(record.bindings[1].totals["input"], 12_000);
        assert_eq!(record.bindings[2].totals["output"], 300);
        let encoded = serde_json::to_string(&record).expect("encode");
        assert!(!encoded.contains("price"));
        let decoded: SessionUsageRecord = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, record);
    }
}
