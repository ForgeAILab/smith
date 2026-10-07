//! The `/capabilities` listing: what the session holds, may activate, and is denied.

use smith_client::local_result::LocalResult;
use smith_client::message_report::MessageReport;
use smith_runtime::capability_limits::{CapabilityRow, CapabilityStanding, catalog};
use smith_runtime::host::HostSession;

use super::CommandReport;

pub(super) fn command(host: &HostSession, session_denials: &[String]) -> CommandReport {
    CommandReport::Show(LocalResult::Message(Box::new(MessageReport::Notice {
        title: "capabilities".to_owned(),
        message: render(&catalog(host.session(), session_denials)),
    })))
}

/// Groups the catalog by standing and names who denied each denied entry.
pub(super) fn render(rows: &[CapabilityRow]) -> String {
    if rows.is_empty() {
        return "no capabilities are registered for this session".to_owned();
    }
    let mut lines = Vec::new();
    for (heading, standings) in [
        ("active", &[CapabilityStanding::Active][..]),
        ("available to activate", &[CapabilityStanding::Available]),
        (
            "denied",
            &[
                CapabilityStanding::DeniedBySession,
                CapabilityStanding::DeniedByProfile,
            ],
        ),
    ] {
        let group = rows
            .iter()
            .filter(|row| standings.contains(&row.standing))
            .collect::<Vec<_>>();
        if group.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("{heading} ({})", group.len()));
        for row in group {
            let detail = match row.standing {
                CapabilityStanding::DeniedBySession => "denied by this session",
                CapabilityStanding::DeniedByProfile => "denied by the profile",
                CapabilityStanding::Active | CapabilityStanding::Available => &brief(&row.summary),
            };
            lines.push(format!("  {} · {detail}", row.id));
        }
    }
    lines.push(String::new());
    lines.push(
        "`/capabilities deny ID` narrows this session · `/capabilities allow ID` undoes it"
            .to_owned(),
    );
    lines.join("\n")
}

/// A summary's first sentence, bounded: tool descriptions run to paragraphs
/// and a listing of fifty of them is unreadable.
fn brief(summary: &str) -> String {
    const LIMIT: usize = 72;
    let first = summary
        .split_once(". ")
        .map_or(summary, |(sentence, _)| sentence)
        .trim()
        .trim_end_matches('.');
    if first.chars().count() <= LIMIT {
        return first.to_owned();
    }
    let mut bounded = first.chars().take(LIMIT - 1).collect::<String>();
    bounded.push('…');
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, standing: CapabilityStanding) -> CapabilityRow {
        CapabilityRow {
            id: id.to_owned(),
            summary: format!("{id} summary"),
            standing,
        }
    }

    #[test]
    fn the_listing_groups_by_standing_and_names_who_denied() {
        let text = render(&[
            row("tool:read", CapabilityStanding::Active),
            row("skill:notes", CapabilityStanding::Available),
            row("tool:shell", CapabilityStanding::DeniedBySession),
            row("tool:edit", CapabilityStanding::DeniedByProfile),
        ]);
        assert!(
            text.contains("active (1)\n  tool:read · tool:read summary"),
            "{text}"
        );
        assert!(
            text.contains("available to activate (1)\n  skill:notes"),
            "{text}"
        );
        assert!(
            text.contains("tool:shell · denied by this session"),
            "{text}"
        );
        assert!(text.contains("tool:edit · denied by the profile"), "{text}");
    }

    #[test]
    fn a_long_summary_is_cut_to_its_first_bounded_sentence() {
        assert_eq!(
            brief("Run a command. It can reach the network."),
            "Run a command"
        );
        let long = brief(&"word ".repeat(40));
        assert_eq!(long.chars().count(), 72);
        assert!(long.ends_with('…'));
    }
}
