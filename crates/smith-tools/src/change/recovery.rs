use super::*;
pub(super) fn textual_reverse(edit: &EditMutation) -> String {
    textual_patch(edit.after.as_deref(), edit.before.as_deref())
}

pub(super) fn textual_forward(edit: &EditMutation) -> String {
    textual_patch(edit.before.as_deref(), edit.after.as_deref())
}

#[derive(Clone, Copy)]
enum PatchLine<'a> {
    Context(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

/// Lengths of the LCS against every prefix of `new`, in linear space.
fn lcs_lengths(old: &[&str], new: &[&str], reverse: bool) -> Vec<usize> {
    let mut lengths = vec![0; new.len() + 1];
    for row in 0..old.len() {
        let left = old[if reverse { old.len() - row - 1 } else { row }];
        let mut diagonal = 0;
        for column in 0..new.len() {
            let right = new[if reverse {
                new.len() - column - 1
            } else {
                column
            }];
            let above = lengths[column + 1];
            lengths[column + 1] = if left == right {
                diagonal + 1
            } else {
                above.max(lengths[column])
            };
            diagonal = above;
        }
    }
    lengths
}

/// A deterministic LCS walk, trimming equal ends before each Hirschberg
/// split. Recovery images may be megabytes, so unlike the small approval
/// differ this never allocates a quadratic table. Ties favor removals.
fn patch_lines<'a>(old: &[&'a str], new: &[&'a str], output: &mut Vec<PatchLine<'a>>) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    output.extend(old[..prefix].iter().map(|line| PatchLine::Context(line)));
    let old = &old[prefix..];
    let new = &new[prefix..];
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let tail = &old[old.len() - suffix..];
    let old = &old[..old.len() - suffix];
    let new = &new[..new.len() - suffix];

    if old.is_empty() {
        output.extend(new.iter().map(|line| PatchLine::Added(line)));
    } else if new.is_empty() {
        output.extend(old.iter().map(|line| PatchLine::Removed(line)));
    } else if old.len() == 1 {
        if let Some(matched) = new.iter().position(|line| line == &old[0]) {
            output.extend(new[..matched].iter().map(|line| PatchLine::Added(line)));
            output.push(PatchLine::Context(old[0]));
            output.extend(new[matched + 1..].iter().map(|line| PatchLine::Added(line)));
        } else {
            output.push(PatchLine::Removed(old[0]));
            output.extend(new.iter().map(|line| PatchLine::Added(line)));
        }
    } else {
        let middle = old.len() / 2;
        let split = {
            let left = lcs_lengths(&old[..middle], new, false);
            let right = lcs_lengths(&old[middle..], new, true);
            let mut split = 0;
            let mut best = 0;
            for column in 0..=new.len() {
                let length = left[column] + right[new.len() - column];
                if length > best {
                    best = length;
                    split = column;
                }
            }
            split
        };
        patch_lines(&old[..middle], &new[..split], output);
        patch_lines(&old[middle..], &new[split..], output);
    }
    output.extend(tail.iter().map(|line| PatchLine::Context(line)));
}

/// One source of recovery patch text for preview, cancellation and apply.
/// Keep newline terminators in the comparison so EOF-only edits are visible.
pub(super) fn textual_patch(old: Option<&[u8]>, new: Option<&[u8]>) -> String {
    const CONTEXT: usize = 3;
    let old_text = old.map(String::from_utf8_lossy).unwrap_or_default();
    let new_text = new.map(String::from_utf8_lossy).unwrap_or_default();
    let old_lines = old_text.split_inclusive('\n').collect::<Vec<_>>();
    let new_lines = new_text.split_inclusive('\n').collect::<Vec<_>>();
    let mut lines = Vec::new();
    patch_lines(&old_lines, &new_lines, &mut lines);

    let mut hunks: Vec<std::ops::Range<usize>> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if matches!(line, PatchLine::Context(_)) {
            continue;
        }
        let start = index.saturating_sub(CONTEXT);
        let end = (index + CONTEXT + 1).min(lines.len());
        if let Some(last) = hunks.last_mut()
            && start <= last.end
        {
            last.end = end;
        } else {
            hunks.push(start..end);
        }
    }
    // Even empty-file creation/deletion is a reviewable file operation.
    if old.is_none() || new.is_none() {
        hunks.clear();
        hunks.push(0..lines.len());
    }

    let mut output = String::new();
    let (mut old_offset, mut new_offset, mut scanned) = (0, 0, 0);
    for hunk in hunks {
        for line in &lines[scanned..hunk.start] {
            old_offset += usize::from(!matches!(line, PatchLine::Added(_)));
            new_offset += usize::from(!matches!(line, PatchLine::Removed(_)));
        }
        let body = &lines[hunk.clone()];
        let old_count = body
            .iter()
            .filter(|line| !matches!(line, PatchLine::Added(_)))
            .count();
        let new_count = body
            .iter()
            .filter(|line| !matches!(line, PatchLine::Removed(_)))
            .count();
        let old_start = old_offset + usize::from(old_count > 0);
        let new_start = new_offset + usize::from(new_count > 0);
        let old_range = if old_count == 1 {
            old_start.to_string()
        } else {
            format!("{old_start},{old_count}")
        };
        let new_range = if new_count == 1 {
            new_start.to_string()
        } else {
            format!("{new_start},{new_count}")
        };
        output.push_str(&format!("@@ -{old_range} +{new_range} @@\n"));
        for line in body {
            let (prefix, text) = match line {
                PatchLine::Context(text) => (' ', *text),
                PatchLine::Removed(text) => ('-', *text),
                PatchLine::Added(text) => ('+', *text),
            };
            output.push(prefix);
            output.push_str(text);
            if !text.ends_with('\n') {
                output.push_str("\n\\ No newline at end of file\n");
            }
        }
        old_offset += old_count;
        new_offset += new_count;
        scanned = hunk.end;
    }
    output
}

pub(super) fn redo_direction(set: &TurnChangeSet) -> Option<RedoDirection> {
    if !set.has_exact_mutations() {
        return None;
    }
    let is_revert = set
        .exact_mutations()
        .all(|edit| edit.call_id == "recovery:revert");
    if is_revert {
        (!set.undone).then_some(RedoDirection::ReapplyRevertedChange)
    } else {
        set.undone.then_some(RedoDirection::ReapplyUndoneTurn)
    }
}

/// The reverse patch `/undo` would apply, with the deltas it leaves alone
/// named first.
///
/// Preview and cancellation journal a fingerprint of this exact text, so both
/// callers must render it the same way.
pub(super) fn undo_preview_text(set: &TurnChangeSet, root: Option<&Path>) -> String {
    let mut output = format!("Smith turn {}\n", set.turn);
    if let Some(note) = ambiguous_note(set) {
        output.push_str(&note);
    }
    for edit in set.exact_mutations() {
        output.push_str(&format!(
            "\n--- current {}\n+++ restore {}\n",
            preview_path(&edit.path, root).display(),
            preview_path(&edit.path, root).display()
        ));
        output.push_str(&textual_reverse(edit));
    }
    output
}

/// Names what the turn changed outside Smith's own editing tools.
///
/// Recovery never reconstructs these: a Git delta observed around a shell
/// command does not prove which bytes the command wrote. Saying so beside the
/// reverse patch is what keeps a partial undo honest.
fn ambiguous_note(set: &TurnChangeSet) -> Option<String> {
    let tools = set.ambiguous_tools();
    if tools.is_empty() {
        return None;
    }
    Some(format!(
        "this turn also changed the workspace through {}; those changes are not \
         attributable file by file and are left untouched — use /diff and /revert\n",
        tools.join(", ")
    ))
}

fn preview_path<'a>(path: &'a Path, root: Option<&Path>) -> &'a Path {
    root.and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path)
}

pub(super) fn redo_preview_text(
    set: &TurnChangeSet,
    direction: RedoDirection,
    root: Option<&Path>,
) -> String {
    let mut output = format!("Smith turn {} redo\n", set.turn);
    if let Some(note) = ambiguous_note(set) {
        output.push_str(&note);
    }
    for edit in set.exact_mutations() {
        output.push_str(&format!(
            "\n--- current {}\n+++ reapply {}\n",
            preview_path(&edit.path, root).display(),
            preview_path(&edit.path, root).display()
        ));
        match direction {
            RedoDirection::ReapplyUndoneTurn => output.push_str(&textual_forward(edit)),
            RedoDirection::ReapplyRevertedChange => output.push_str(&textual_reverse(edit)),
        }
    }
    output
}
