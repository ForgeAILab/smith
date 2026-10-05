use super::*;

/// Retains bounded, control-free detail so folding can be reversed on demand.
pub(super) fn bound_result_preview(raw: &str) -> Option<String> {
    if raw.trim().is_empty() {
        return None;
    }
    Some(
        bound_local_result(raw.to_owned())
            .lines()
            .map(sanitize_preview_line)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn sanitize_preview_line(line: &str) -> String {
    let mut sanitized = String::new();
    for character in line.chars() {
        if character == '\t' {
            sanitized.push(' ');
            continue;
        }
        // The same unsafe-control set the display projector strips: C0/C1
        // plus zero-width and bidi override codepoints.
        if character.is_control()
            || matches!(
                character,
                '\u{200b}'..='\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}'
                    | '\u{feff}'
            )
        {
            continue;
        }
        sanitized.push(character);
    }
    sanitized
}

pub(super) fn bound_local_result(content: String) -> String {
    let mut bounded = String::with_capacity(content.len().min(MAX_LOCAL_RESULT_BYTES));
    let mut lines = 1;
    let mut truncated = false;
    for character in content.chars() {
        if bounded.len() + character.len_utf8() > MAX_LOCAL_RESULT_BYTES
            || (character == '\n' && lines >= MAX_LOCAL_RESULT_LINES)
        {
            truncated = true;
            break;
        }
        bounded.push(character);
        if character == '\n' {
            lines += 1;
        }
    }
    if truncated {
        if !bounded.ends_with('\n') {
            bounded.push('\n');
        }
        bounded.push_str("[local result truncated at the display limit]");
    }
    bounded
}

pub(super) fn bound_message_report(mut report: MessageReport) -> MessageReport {
    let (title, message) = match &mut report {
        MessageReport::Notice { title, message }
        | MessageReport::Empty { title, message }
        | MessageReport::Error { title, message } => (title, message),
    };
    *title = title
        .replace(['\r', '\n'], " ")
        .chars()
        .take(MAX_LOCAL_RESULT_TITLE_CHARS)
        .collect();
    if message.trim().is_empty() {
        return MessageReport::Empty {
            title: std::mem::take(title),
            message: "No output.".to_owned(),
        };
    }
    *message = bound_local_result(std::mem::take(message));
    report
}

/// Applies the existing local-output limits without reconstructing patch roles.
pub(super) fn bound_diff_report(mut report: DiffReport) -> DiffReport {
    report.title = report
        .title
        .replace(['\r', '\n'], " ")
        .chars()
        .take(MAX_LOCAL_RESULT_TITLE_CHARS)
        .collect();
    match &mut report.outcome {
        DiffOutcome::Empty => {}
        DiffOutcome::Error(message) => {
            *message = bound_local_result(std::mem::take(message));
        }
        DiffOutcome::Patch(patch) => {
            let mut bytes = 0;
            let mut lines = 1;
            let mut last_character = None;
            let mut truncated_at = None;
            for (index, line) in patch.iter_mut().enumerate() {
                let mut end = 0;
                for (offset, character) in line.text.char_indices() {
                    if bytes + character.len_utf8() > MAX_LOCAL_RESULT_BYTES
                        || (character == '\n' && lines >= MAX_LOCAL_RESULT_LINES)
                    {
                        truncated_at = Some(index);
                        break;
                    }
                    bytes += character.len_utf8();
                    if character == '\n' {
                        lines += 1;
                    }
                    last_character = Some(character);
                    end = offset + character.len_utf8();
                }
                if truncated_at.is_some() {
                    line.text.truncate(end);
                    break;
                }
            }
            if let Some(index) = truncated_at {
                patch.truncate(index + 1);
                if last_character != Some('\n') {
                    patch[index].text.push('\n');
                }
                patch.push(DiffLine {
                    kind: DiffLineKind::Context,
                    text: "[local result truncated at the display limit]".to_owned(),
                });
            }
        }
    }
    report
}
