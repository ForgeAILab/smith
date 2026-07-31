//! Smith-owned bounded memory selection over the generic contributor.
//!
//! This source is deliberately read-only: callers choose the records and
//! retention outside the turn loop, Smith applies deterministic relevance and
//! size policy, and Agent Runtime contributes the result as separately
//! revisioned context rather than rewriting canonical conversation history.

use std::collections::BTreeSet;
use std::fmt;

use agent_runtime::context::Sensitivity;
use agent_runtime::harness::{
    MAX_MEMORY_ID_CHARS, MAX_MEMORY_RECORD_CHARS, MAX_MEMORY_RECORDS, MAX_MEMORY_TOTAL_CHARS,
    MemoryQuery, MemoryRecord, MemorySource,
};
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::content::Role;
use agent_runtime_core::error::RuntimeError;
use async_trait::async_trait;

/// One host-selected Smith memory candidate.
#[derive(Clone, PartialEq, Eq)]
pub struct SmithMemoryRecord {
    id: String,
    content: String,
    sensitivity: Sensitivity,
    priority: i32,
    keywords: Vec<String>,
    revision: RegistryRevision,
}

impl fmt::Debug for SmithMemoryRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SmithMemoryRecord")
            .field("id", &self.id)
            .field("content_chars", &self.content.chars().count())
            .field("sensitivity", &self.sensitivity)
            .field("priority", &self.priority)
            .field("keyword_count", &self.keywords.len())
            .field("revision", &self.revision)
            .finish()
    }
}

impl SmithMemoryRecord {
    /// Creates one memory record. Empty keyword sets are always relevant.
    pub fn new(
        id: impl Into<String>,
        content: impl Into<String>,
        sensitivity: Sensitivity,
    ) -> Self {
        let id = id.into();
        let content = content.into();
        let revision = RegistryRevision::from_content(&content);
        Self {
            id,
            content,
            sensitivity,
            priority: 100,
            keywords: Vec::new(),
            revision,
        }
    }

    /// Sets lower-is-retained-first structural priority.
    #[must_use]
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Selects this record only when at least one normalized keyword appears
    /// in the latest user input.
    #[must_use]
    pub fn with_keywords<I, S>(mut self, keywords: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.keywords = normalize_terms(keywords);
        self
    }
}

/// Smith retrieval and disclosure limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmithMemoryPolicy {
    /// Whether this source contributes anything.
    pub enabled: bool,
    /// Maximum returned records.
    pub max_records: usize,
    /// Maximum aggregate returned characters.
    pub max_total_chars: usize,
}

impl Default for SmithMemoryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_records: 8,
            max_total_chars: 8_192,
        }
    }
}

impl SmithMemoryPolicy {
    /// Explicitly disables retrieval without deleting host-owned records.
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            max_records: 0,
            max_total_chars: 0,
        }
    }
}

/// Minimal deterministic Smith memory policy.
#[derive(Clone)]
pub struct SmithMemorySource {
    records: Vec<SmithMemoryRecord>,
    policy: SmithMemoryPolicy,
    revision: RegistryRevision,
}

impl fmt::Debug for SmithMemorySource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SmithMemorySource")
            .field("record_count", &self.records.len())
            .field("policy", &self.policy)
            .field("revision", &self.revision)
            .finish()
    }
}

impl SmithMemorySource {
    /// Validates host-selected records under the default policy.
    pub fn new(records: Vec<SmithMemoryRecord>) -> Result<Self, RuntimeError> {
        Self::with_policy(records, SmithMemoryPolicy::default())
    }

    /// Validates host-selected records under an explicit policy.
    pub fn with_policy(
        records: Vec<SmithMemoryRecord>,
        policy: SmithMemoryPolicy,
    ) -> Result<Self, RuntimeError> {
        validate_policy(policy)?;
        validate_records(&records)?;
        let mut revision_material = String::new();
        revision_material.push_str("smith-memory-source-v1\n");
        revision_material.push_str(&format!(
            "{}:{}:{}\n",
            policy.enabled, policy.max_records, policy.max_total_chars
        ));
        for record in &records {
            revision_material.push_str(&format!(
                "{}:{}:{}:{}:",
                record.id,
                record.revision,
                record.priority,
                sensitivity_name(record.sensitivity)
            ));
            for keyword in &record.keywords {
                revision_material.push_str(keyword);
                revision_material.push(',');
            }
            revision_material.push('\n');
        }
        Ok(Self {
            records,
            policy,
            revision: RegistryRevision::from_content(revision_material),
        })
    }

    /// Configured Smith retrieval policy.
    pub const fn policy(&self) -> SmithMemoryPolicy {
        self.policy
    }
}

#[async_trait]
impl MemorySource for SmithMemorySource {
    fn id(&self) -> &str {
        "smith"
    }

    fn revision(&self) -> RegistryRevision {
        self.revision.clone()
    }

    async fn retrieve(&self, query: &MemoryQuery) -> Result<Vec<MemoryRecord>, RuntimeError> {
        if !self.policy.enabled {
            return Ok(Vec::new());
        }
        let terms = query
            .history
            .iter()
            .rev()
            .find(|message| message.role == Role::User)
            .map(|message| normalize_text(&message.joined_text()))
            .unwrap_or_default();
        let mut candidates = self
            .records
            .iter()
            .filter(|record| {
                record.keywords.is_empty()
                    || record
                        .keywords
                        .iter()
                        .any(|keyword| terms.contains(keyword))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut total_chars = 0usize;
        let mut selected = Vec::new();
        for record in candidates.into_iter().take(self.policy.max_records) {
            let chars = record.content.chars().count();
            if total_chars.saturating_add(chars) > self.policy.max_total_chars {
                continue;
            }
            total_chars += chars;
            selected.push(MemoryRecord {
                id: record.id.clone(),
                revision: record.revision.clone(),
                content: record.content.clone(),
                sensitivity: record.sensitivity,
                priority: record.priority,
            });
        }
        Ok(selected)
    }
}

fn validate_policy(policy: SmithMemoryPolicy) -> Result<(), RuntimeError> {
    if policy.max_records > MAX_MEMORY_RECORDS {
        return Err(RuntimeError::config(format!(
            "Smith memory policy exceeds the generic {MAX_MEMORY_RECORDS}-record bound"
        )));
    }
    if policy.max_total_chars > MAX_MEMORY_TOTAL_CHARS {
        return Err(RuntimeError::config(format!(
            "Smith memory policy exceeds the generic {MAX_MEMORY_TOTAL_CHARS}-character bound"
        )));
    }
    Ok(())
}

fn validate_records(records: &[SmithMemoryRecord]) -> Result<(), RuntimeError> {
    if records.len() > MAX_MEMORY_RECORDS {
        return Err(RuntimeError::config(format!(
            "Smith memory source exceeds the {MAX_MEMORY_RECORDS}-record bound"
        )));
    }
    let mut ids = BTreeSet::new();
    for record in records {
        if record.id.trim().is_empty()
            || record.id.chars().count() > MAX_MEMORY_ID_CHARS
            || !ids.insert(record.id.as_str())
        {
            return Err(RuntimeError::config(
                "Smith memory ids must be unique and within the generic bound",
            ));
        }
        let chars = record.content.chars().count();
        if chars == 0 || chars > MAX_MEMORY_RECORD_CHARS {
            return Err(RuntimeError::config(format!(
                "Smith memory record `{}` must contain 1..={MAX_MEMORY_RECORD_CHARS} characters",
                record.id
            )));
        }
    }
    Ok(())
}

fn normalize_terms<I, S>(terms: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut terms = terms
        .into_iter()
        .map(Into::into)
        .flat_map(|term: String| {
            term.split(|character: char| {
                !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
            })
            .map(str::to_owned)
            .collect::<Vec<_>>()
        })
        .map(|term| term.trim().to_lowercase())
        .filter(|term| term.chars().count() >= 2)
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms.truncate(32);
    terms
}

fn normalize_text(text: &str) -> BTreeSet<String> {
    normalize_terms([text]).into_iter().collect()
}

const fn sensitivity_name(sensitivity: Sensitivity) -> &'static str {
    match sensitivity {
        Sensitivity::Public => "public",
        Sensitivity::Internal => "internal",
        Sensitivity::Sensitive => "sensitive",
        Sensitivity::Secret => "secret",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_runtime_core::content::Message;
    use agent_runtime_core::ids::{SessionId, TurnId};

    use super::*;

    fn query(text: &str) -> MemoryQuery {
        MemoryQuery {
            session: SessionId::new("session"),
            turn: TurnId::new("turn"),
            history: Arc::from(vec![Message::user(text)]),
        }
    }

    #[tokio::test]
    async fn retrieval_is_relevant_bounded_and_priority_ordered() {
        let source = SmithMemorySource::with_policy(
            vec![
                SmithMemoryRecord::new("late", "late", Sensitivity::Internal)
                    .with_priority(20)
                    .with_keywords(["rust"]),
                SmithMemoryRecord::new("first", "first", Sensitivity::Sensitive)
                    .with_priority(1)
                    .with_keywords(["rust"]),
                SmithMemoryRecord::new("other", "other", Sensitivity::Public)
                    .with_keywords(["python"]),
            ],
            SmithMemoryPolicy {
                enabled: true,
                max_records: 2,
                max_total_chars: 16,
            },
        )
        .unwrap();
        let records = source
            .retrieve(&query("Review the Rust code"))
            .await
            .unwrap();
        assert_eq!(
            records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "late"]
        );
        assert_eq!(records[0].sensitivity, Sensitivity::Sensitive);
    }

    #[tokio::test]
    async fn disabled_policy_returns_no_content() {
        let source = SmithMemorySource::with_policy(
            vec![SmithMemoryRecord::new(
                "preference",
                "secret preference",
                Sensitivity::Sensitive,
            )],
            SmithMemoryPolicy::disabled(),
        )
        .unwrap();
        assert!(
            source
                .retrieve(&query("anything"))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn debug_never_contains_memory_content() {
        let source = SmithMemorySource::new(vec![SmithMemoryRecord::new(
            "preference",
            "DO_NOT_RENDER_MEMORY_CONTENT",
            Sensitivity::Secret,
        )])
        .unwrap();
        assert!(!format!("{source:?}").contains("DO_NOT_RENDER_MEMORY_CONTENT"));
    }
}
