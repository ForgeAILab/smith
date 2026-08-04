//! The anchored approval panel.
//!
//! Approvals are drawn in the layout above the composer rather than floated
//! over the transcript. A user deciding whether to allow a command is judging
//! it against the work that led there, and a box covering that work hides the
//! very context the question is about. Anchoring also means the panel competes
//! for rows like anything else, so a small terminal degrades by dropping
//! detail instead of by obscuring the session.
//!
//! What survives truncation is ordered deliberately: the exact prepared target
//! first, then the authority being requested, then everything else
//! (`DESIGN.md` §9). A user who can read only two rows still sees what would
//! run and what it could reach.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::diff::EditReview;
use crate::theme::{Theme, Tone, glyph};

use super::modal::{argument_lines, authority_warning, review_lines, security_resource_text};

/// The most rows an approval may take before the layout bounds it further.
///
/// Generous on purpose. A diff is the substance of an edit approval, and a
/// panel that shows the header but elides the change asks the user to approve
/// something they cannot see.
const MAX_APPROVAL_ROWS: u16 = 18;

/// Builds the panel's header and body once, so the row estimate and the render
/// cannot disagree.
///
/// They did disagree while the estimate counted an assumed header height: a
/// tool that also warns about its authority added a row the estimate never
/// knew about, and the body lost one at the bottom. Measuring the real lines
/// removes the class of bug rather than the instance.
fn compose(
    prompt: &smith_host::approval::ApprovalPrompt,
    review: Option<&EditReview>,
    theme: Theme,
) -> (Vec<Line<'static>>, Vec<Line<'static>>, usize) {
    let prepared = prompt.prepared();
    let mut head = Vec::new();

    // The exact target is first so a screen reader and the tightest supported
    // terminal both retain the fact a user cannot answer without.
    head.push(Line::from(vec![
        Span::styled(
            format!("{} approval required  ", glyph::APPROVAL),
            theme.style(Tone::Heading),
        ),
        Span::styled(
            security_resource_text(prepared.resource()),
            theme.style(Tone::Danger),
        ),
    ]));
    head.push(Line::from(vec![
        Span::styled(format!("  {}  ", prepared.tool()), theme.style(Tone::Dim)),
        Span::raw(prepared.display().title.clone()),
    ]));
    if let Some(detail) = &prepared.display().detail {
        head.push(Line::from(vec![
            Span::styled("  action  ", theme.style(Tone::Dim)),
            Span::raw(detail.clone()),
        ]));
    }
    if let Some(review) = review {
        head.push(Line::from(vec![
            Span::styled("  change  ", theme.style(Tone::Dim)),
            Span::raw(review.summary()),
        ]));
    }
    let permissions = prepared
        .required_permissions()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if !permissions.is_empty() {
        head.push(Line::from(vec![
            Span::styled("  grants  ", theme.style(Tone::Dim)),
            Span::styled(permissions.join(", "), theme.style(Tone::Danger)),
        ]));
    }
    if let Some(warning) = authority_warning(prepared) {
        head.push(Line::from(Span::styled(
            format!("  {} {warning}", glyph::WARNING),
            theme.style(Tone::Warning),
        )));
    }

    // A diff when there is one, the prepared arguments otherwise. Either way
    // the user sees the substance of what they are authorizing, not its name.
    let (body, elided) = match review {
        Some(review) => review_lines(review, theme),
        None => argument_lines(prepared.arguments(), theme),
    };
    (head, body, elided)
}

/// Rows this panel wants, before the layout bounds it.
pub(super) fn desired_approval_rows(
    prompt: &smith_host::approval::ApprovalPrompt,
    review: Option<&EditReview>,
) -> u16 {
    let (head, body, elided) = compose(prompt, review, Theme::default());
    // Plus the key bar, which is never given up, and a row for the elision
    // notice when the source already withheld something.
    let notice = usize::from(elided > 0);
    let rows = head
        .len()
        .saturating_add(body.len())
        .saturating_add(notice)
        .saturating_add(1);
    u16::try_from(rows)
        .unwrap_or(MAX_APPROVAL_ROWS)
        .min(MAX_APPROVAL_ROWS)
}

/// Draws the approval into its anchored rows.
pub(super) fn draw_approval(
    frame: &mut Frame<'_>,
    area: Rect,
    prompt: &smith_host::approval::ApprovalPrompt,
    review: Option<&EditReview>,
    theme: Theme,
) {
    let (mut lines, mut body, elided) = compose(prompt, review, theme);

    // Width-aware, because the keys are the one part that must survive: a
    // narrow terminal that clips "deny" leaves the user unable to refuse.
    let keys = if area.width >= 64 {
        Line::from(vec![
            Span::styled("  y", theme.style(Tone::Success)),
            Span::styled(" allow once   ", theme.style(Tone::Dim)),
            Span::styled("a", theme.style(Tone::Warning)),
            Span::styled(
                " allow this target for the session   ",
                theme.style(Tone::Dim),
            ),
            Span::styled("n", theme.style(Tone::Danger)),
            Span::styled(" deny", theme.style(Tone::Dim)),
        ])
    } else {
        Line::from(vec![
            Span::styled("  y", theme.style(Tone::Success)),
            Span::styled(" allow once  ", theme.style(Tone::Dim)),
            Span::styled("a", theme.style(Tone::Warning)),
            Span::styled(" allow target  ", theme.style(Tone::Dim)),
            Span::styled("n", theme.style(Tone::Danger)),
            Span::styled(" deny", theme.style(Tone::Dim)),
        ])
    };

    // The key bar is reserved out of the height before the body is fitted, and
    // the header is trimmed only after the body is gone. An approval a user
    // cannot answer is worse than one they cannot fully read, so the keys are
    // the last thing to go and the diff is the first.
    let height = usize::from(area.height);
    let reserved = height.saturating_sub(1);
    if lines.len() > reserved {
        lines.truncate(reserved);
    } else {
        let room = reserved - lines.len();
        // Whatever the source withheld plus whatever the height cannot hold.
        // Dropping body lines silently would let a panel look complete while
        // hiding part of the change the user is authorizing.
        let dropped = body.len().saturating_sub(room);
        let hidden = elided.saturating_add(dropped);
        if hidden > 0 && room > 0 {
            body.truncate(room.saturating_sub(1));
            body.push(Line::from(Span::styled(
                format!("  … {hidden} more lines not shown"),
                theme.style(Tone::Dim),
            )));
        } else {
            body.truncate(room);
        }
        lines.extend(body);
    }
    if height > 0 {
        lines.push(keys);
    }

    frame.render_widget(Paragraph::new(lines), area);
}
