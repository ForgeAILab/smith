//! Terminal status around the shared session accounting projection.

use std::ops::{Deref, DerefMut};

pub use smith_client::status::{
    CapabilityStatus, Confidence, ContextPlanStatus, ContextPlanUpdate, CostLabel, PriceReference,
    PriceTable, SessionCost, SessionUsage, TokenCount, TurnUsage, counter_label, render_elapsed,
    render_terminal_elapsed,
};

/// What the agent is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Activity {
    /// Waiting for input.
    #[default]
    Idle,
    /// No parent provider turn is open while direct child work remains pending.
    ParkedAwaitingChild,
    /// A turn is running.
    Working,
    /// A turn is being cancelled.
    Interrupting,
    /// The session has shut down.
    Ended,
}

impl Activity {
    /// The word shown beside the spinner. Paired with the glyph, never
    /// replaced by it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "ready",
            Self::ParkedAwaitingChild => "waiting for child",
            Self::Working => "working",
            Self::Interrupting => "interrupting",
            Self::Ended => "ended",
        }
    }
}

/// The active pool account, as the footer shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountStatus {
    /// The credential reference, never its value.
    pub label: String,
    /// Server-reported consumption, absent when nothing measured it.
    pub used_percent: Option<f64>,
}

/// Declared MCP servers that have not settled yet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct McpStatus {
    /// Servers still becoming ready.
    pub connecting: usize,
    /// Servers that tried and failed.
    pub failed: usize,
}

impl McpStatus {
    /// Whether anything is worth reporting at all.
    ///
    /// Every settled, working server reports nothing: the quiet state is the
    /// one a session spends nearly all its time in.
    pub fn is_quiet(self) -> bool {
        self.connecting == 0 && self.failed == 0
    }

    /// The footer segment, or `None` while everything is settled and working.
    pub fn render_footer(self) -> Option<String> {
        if self.is_quiet() {
            return None;
        }
        let mut parts = Vec::new();
        if self.connecting > 0 {
            parts.push(format!("{} connecting", self.connecting));
        }
        if self.failed > 0 {
            parts.push(format!("{} failed", self.failed));
        }
        Some(format!("mcp {}", parts.join(" · ")))
    }
}

/// The header's model of session status.
#[derive(Debug, Clone)]
pub struct Status {
    /// The credential-pool account serving attempts, when the provider
    /// declares a pool. Absent for a single-credential provider, which has no
    /// account to disambiguate.
    pub account: Option<AccountStatus>,
    /// What the agent is doing.
    pub activity: Activity,
    /// Effective approval mode supplied by the host, absent until resolved.
    pub approval_mode: Option<String>,
    /// How many declared MCP servers are still connecting, and how many failed.
    ///
    /// A server that is starting or broken is operational state the user needs
    /// while it lasts and never afterwards, so it lives here rather than in the
    /// transcript: a connection settling must not interrupt a conversation.
    pub mcp: McpStatus,
    projection: smith_client::status::Status,
}

impl Status {
    /// The account segment for the footer, when the provider declares a pool.
    ///
    /// Deliberately its own segment rather than part of the context or token
    /// counters: a rate-limit window is a server-reported percentage of an
    /// account's plan, and the counters are Smith's disjoint token
    /// measurement of one session. Adjacent, never merged.
    pub fn render_account_footer(&self) -> Option<String> {
        let account = self.account.as_ref()?;
        Some(match account.used_percent {
            Some(percent) => format!("{} {}%", account.label, percent.round() as i64),
            // Unknown stays unknown: the account is named, its consumption is
            // not guessed at.
            None => account.label.clone(),
        })
    }

    /// A status for a session that has not yet run a turn.
    pub fn new(model: impl Into<String>, project: impl Into<String>) -> Self {
        Self {
            account: None,
            activity: Activity::Idle,
            approval_mode: None,
            mcp: McpStatus::default(),
            projection: smith_client::status::Status::new(model, project),
        }
    }
}

impl Deref for Status {
    type Target = smith_client::status::Status;

    fn deref(&self) -> &Self::Target {
        &self.projection
    }
}

impl DerefMut for Status {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.projection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_footer_shows_the_reference_and_its_meter() {
        let mut status = Status::new("gpt-5.3", "~/work");
        assert_eq!(status.render_account_footer(), None, "no pool, no segment");

        status.account = Some(AccountStatus {
            label: "keychain:smith/work".to_owned(),
            used_percent: Some(82.4),
        });
        assert_eq!(
            status.render_account_footer().as_deref(),
            Some("keychain:smith/work 82%")
        );
    }

    #[test]
    fn an_unmeasured_account_is_named_without_a_number() {
        let mut status = Status::new("gpt-5.3", "~/work");
        status.account = Some(AccountStatus {
            label: "keychain:smith/work".to_owned(),
            used_percent: None,
        });
        // Naming the account is useful; guessing its consumption is not.
        assert_eq!(
            status.render_account_footer().as_deref(),
            Some("keychain:smith/work")
        );
    }

    #[test]
    fn the_account_meter_is_separate_from_the_context_footer() {
        let mut status = Status::new("gpt-5.3", "~/work");
        status.account = Some(AccountStatus {
            label: "keychain:smith/work".to_owned(),
            used_percent: Some(82.0),
        });
        // A plan percentage and a token count are different measurements and
        // must never end up in one segment.
        assert!(!status.render_context_footer().contains("82%"));
        assert!(!status.render_context_footer().contains("keychain"));
    }
}
