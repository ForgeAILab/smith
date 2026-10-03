//! Width-aware assistant Markdown, shared by live attempts and history.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::theme::{Theme, Tone, glyph};

use super::wrap::wrap_lines;

pub(super) fn render_assistant_lines(text: &str, theme: Theme, width: u16) -> Vec<Line<'static>> {
    let mut renderer = Markdown {
        theme,
        width: width.saturating_sub(2).max(1),
        rows: Vec::new(),
    };
    let source: Vec<_> = text.lines().collect();
    let mut fence: Option<Fence> = None;
    let mut lists: Vec<ListIndent> = Vec::new();
    let mut index = 0;
    while index < source.len() {
        let raw = source[index];
        let line = container(raw);
        index += 1;
        if let Some(open) = &fence {
            let code = fence_body(raw, open);
            if closing_fence(code.trim_start(), open) {
                fence = None;
            } else {
                let prefix = format!("{}  ", open.prefix);
                renderer.push(
                    vec![Span::styled(code, theme.style(Tone::Code))],
                    &prefix,
                    &prefix,
                );
            }
            continue;
        }

        if lists.last().is_some_and(|item| item.quotes != line.quotes) {
            lists.clear();
        }
        let mut prefix = line.prefix();
        let mut continuation = prefix.clone();
        let mut body = line.body;
        let mut code_indent = line.indent;
        let mut quote_depth = line
            .quotes
            .chars()
            .filter(|character| *character == '│')
            .count();
        if !horizontal_rule(body)
            && let Some((marker, rest)) = list_item(body)
        {
            while lists.last().is_some_and(|item| item.indent >= line.indent) {
                lists.pop();
            }
            prefix.push_str(&marker);
            continuation.push_str(&" ".repeat(marker.width()));
            code_indent += marker.width();
            lists.push(ListIndent {
                quotes: line.quotes.clone(),
                indent: line.indent,
                content_indent: line.indent + marker.width(),
                prefix: continuation.clone(),
            });
            body = rest;
            // A quote can be the body of an item as well as its container.
            let quoted = container(body);
            if !quoted.quotes.is_empty() {
                let quote_prefix = quoted.prefix();
                prefix.push_str(&quote_prefix);
                continuation.push_str(&quote_prefix);
                code_indent += quoted.indent;
                quote_depth += quoted
                    .quotes
                    .chars()
                    .filter(|character| *character == '│')
                    .count();
                body = quoted.body;
            }
        } else if body.is_empty() {
            lists.clear();
        } else if heading(body, theme).is_none()
            && fence_start(body).is_none()
            && !horizontal_rule(body)
            && let Some(item) = lists
                .iter()
                .rev()
                .find(|item| item.content_indent <= line.indent)
                .or_else(|| lists.first())
            && line.indent < item.content_indent
        {
            // Markdown allows an item's paragraph to continue without an
            // explicit marker or indentation in the source.
            prefix = item.prefix.clone();
            continuation = prefix.clone();
        }

        if let Some((marker, length, label)) = fence_start(body) {
            if !label.is_empty() {
                renderer.push(
                    vec![Span::styled(label.to_owned(), theme.style(Tone::Dim))],
                    &prefix,
                    &continuation,
                );
            }
            fence = Some(Fence {
                marker,
                length,
                prefix: continuation,
                indent: code_indent,
                quote_depth,
            });
        } else if let Some(headers) = table_cells(body)
            && let Some(separator) = source.get(index).map(|raw| container(raw))
            && separator.quotes == line.quotes
            && separator.indent == line.indent
            && table_separator(separator.body, headers.len())
        {
            index += 1;
            let mut records = Vec::new();
            while let Some(raw) = source.get(index) {
                let row = container(raw);
                if row.quotes != line.quotes || row.indent != line.indent {
                    break;
                }
                let Some(cells) = table_cells(row.body) else {
                    break;
                };
                if cells.len() > headers.len() {
                    break;
                }
                records.push(cells);
                index += 1;
            }
            renderer.table(&headers, &records, &prefix, &continuation);
        } else if horizontal_rule(body) {
            let length = usize::from(renderer.width)
                .saturating_sub(prefix.width())
                .max(1);
            renderer.push(
                vec![Span::styled("─".repeat(length), theme.style(Tone::Dim))],
                &prefix,
                &continuation,
            );
        } else {
            let (body, base) = heading(body, theme).unwrap_or((body, theme.style(Tone::Default)));
            renderer.push(inline(body, base, theme, 0), &prefix, &continuation);
        }
    }

    for (index, row) in renderer.rows.iter_mut().enumerate() {
        row.spans.insert(
            0,
            Span::styled(
                if index == 0 {
                    format!("{} ", glyph::BULLET)
                } else {
                    "  ".to_owned()
                },
                theme.style(Tone::Dim),
            ),
        );
    }
    renderer.rows
}

struct Markdown {
    theme: Theme,
    width: u16,
    rows: Vec<Line<'static>>,
}

impl Markdown {
    fn push(&mut self, spans: Vec<Span<'static>>, first: &str, continuation: &str) {
        // Even deeply nested input leaves room for a whole wide glyph. The
        // normal terminal minimum is 40, but source indentation is unbounded.
        let budget = usize::from(self.width).saturating_sub(2);
        let first = bounded_prefix(first, budget);
        let continuation = bounded_prefix(continuation, budget);
        self.push_prefixed(
            spans,
            vec![Span::styled(first, self.theme.style(Tone::Dim))],
            &continuation,
        );
    }

    fn push_prefixed(
        &mut self,
        spans: Vec<Span<'static>>,
        first: Vec<Span<'static>>,
        continuation: &str,
    ) {
        let indent = Line::from(first.clone()).width().max(continuation.width());
        let rows = wrap_lines(
            &[Line::from(spans)],
            self.width.saturating_sub(indent as u16).max(1),
        );
        for (index, mut row) in rows.into_iter().enumerate() {
            let mut prefix = if index == 0 {
                first.clone()
            } else {
                vec![Span::styled(
                    continuation.to_owned(),
                    self.theme.style(Tone::Dim),
                )]
            };
            prefix.retain(|span| !span.content.is_empty());
            prefix.append(&mut row.spans);
            row.spans = prefix;
            self.rows.push(row);
        }
    }

    fn table(
        &mut self,
        headers: &[String],
        records: &[Vec<String>],
        first: &str,
        continuation: &str,
    ) {
        let header_spans: Vec<_> = headers
            .iter()
            .map(|cell| inline(cell, self.theme.style(Tone::Heading), self.theme, 0))
            .collect();
        let record_spans: Vec<Vec<_>> = records
            .iter()
            .map(|row| {
                (0..headers.len())
                    .map(|column| {
                        inline(
                            row.get(column).map_or("", String::as_str),
                            self.theme.style(Tone::Default),
                            self.theme,
                            0,
                        )
                    })
                    .collect()
            })
            .collect();
        // Measure rendered cells: markup disappears, but visible link targets
        // must still count when deciding whether the table fits.
        let mut widths: Vec<_> = header_spans
            .iter()
            .map(|spans| Line::from(spans.clone()).width())
            .collect();
        for row in &record_spans {
            for (width, cell) in widths.iter_mut().zip(row) {
                *width = (*width).max(Line::from(cell.clone()).width());
            }
        }
        let table_width = widths.iter().sum::<usize>() + headers.len().saturating_sub(1) * 3;
        let available =
            usize::from(self.width).saturating_sub(first.width().max(continuation.width()));
        if table_width > available {
            if records.is_empty() {
                for (index, header) in header_spans.into_iter().enumerate() {
                    self.push(
                        header,
                        if index == 0 { first } else { continuation },
                        continuation,
                    );
                }
                return;
            }
            for (index, row) in record_spans.into_iter().enumerate() {
                if index > 0 {
                    self.push(Vec::new(), continuation, continuation);
                }
                for (column, (header, value)) in header_spans.iter().zip(row).enumerate() {
                    let prefix = if index == 0 && column == 0 {
                        first
                    } else {
                        continuation
                    };
                    let mut key = header.clone();
                    key.push(Span::styled(": ", self.theme.style(Tone::Dim)));
                    let key_text = Line::from(key.clone()).to_string();
                    let hanging = format!("{continuation}{}", " ".repeat(key_text.width()));
                    // A long key wraps too; it must not consume the value's
                    // entire line budget as an enormous hanging indent.
                    if prefix.width() + key_text.width() + 8 <= usize::from(self.width) {
                        key.insert(
                            0,
                            Span::styled(prefix.to_owned(), self.theme.style(Tone::Dim)),
                        );
                        self.push_prefixed(value, key, &hanging);
                    } else {
                        key.extend(value);
                        self.push(key, prefix, continuation);
                    }
                }
            }
            return;
        }
        let header = table_row(&header_spans, &widths, self.theme);
        self.push(header, first, continuation);
        let rule = widths
            .iter()
            .map(|width| "─".repeat(*width))
            .collect::<Vec<_>>()
            .join("─┼─");
        self.push(
            vec![Span::styled(rule, self.theme.style(Tone::Dim))],
            continuation,
            continuation,
        );
        for row in record_spans {
            self.push(
                table_row(&row, &widths, self.theme),
                continuation,
                continuation,
            );
        }
    }
}

fn table_row(cells: &[Vec<Span<'static>>], widths: &[usize], theme: Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (index, (cell, width)) in cells.iter().zip(widths).enumerate() {
        if index > 0 {
            spans.push(Span::styled(" │ ", theme.style(Tone::Dim)));
        }
        spans.extend(cell.iter().cloned());
        if index + 1 < cells.len() {
            let padding = width.saturating_sub(Line::from(cell.clone()).width());
            spans.push(Span::raw(" ".repeat(padding)));
        }
    }
    spans
}

fn bounded_prefix(prefix: &str, budget: usize) -> String {
    prefix.chars().take(budget).collect()
}

struct Container<'a> {
    quotes: String,
    indent: usize,
    body: &'a str,
}

impl Container<'_> {
    fn prefix(&self) -> String {
        format!("{}{}", self.quotes, " ".repeat(self.indent))
    }
}

fn container(raw: &str) -> Container<'_> {
    let (mut indent, mut body) = take_indent(raw);
    let mut quotes = String::new();
    while let Some(rest) = body.strip_prefix('>') {
        quotes.push_str("│ ");
        let (spaces, rest) = take_indent(rest.strip_prefix(' ').unwrap_or(rest));
        indent += spaces;
        body = rest;
    }
    Container {
        quotes,
        indent,
        body,
    }
}

fn take_indent(raw: &str) -> (usize, &str) {
    let mut columns = 0;
    let mut bytes = 0;
    for character in raw.chars() {
        match character {
            ' ' => columns += 1,
            '\t' => columns += 4 - columns % 4,
            _ => break,
        }
        bytes += character.len_utf8();
    }
    (columns, &raw[bytes..])
}

struct ListIndent {
    quotes: String,
    indent: usize,
    content_indent: usize,
    prefix: String,
}

fn list_item(raw: &str) -> Option<(String, &str)> {
    let end = if raw.starts_with(['-', '+', '*']) {
        1
    } else {
        let digits = raw.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=9).contains(&digits) || !matches!(raw.as_bytes().get(digits), Some(b'.' | b')')) {
            return None;
        }
        digits + 1
    };
    if !raw.as_bytes().get(end).is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    let (spaces, body) = take_indent(&raw[end..]);
    let marker = if end == 1 { "-" } else { &raw[..end] };
    Some((format!("{marker}{}", " ".repeat(spaces)), body))
}

struct Fence {
    marker: char,
    length: usize,
    prefix: String,
    indent: usize,
    quote_depth: usize,
}

fn fence_body(raw: &str, open: &Fence) -> String {
    let mut rest = raw;
    let mut removed_indent = 0;
    for _ in 0..open.quote_depth {
        let (spaces, body) = take_indent(rest);
        let Some(quoted) = body.strip_prefix('>') else {
            break;
        };
        removed_indent += spaces;
        rest = quoted.strip_prefix(' ').unwrap_or(quoted);
    }
    let (spaces, body) = take_indent(rest);
    let indent = spaces.saturating_sub(open.indent.saturating_sub(removed_indent));
    format!("{}{body}", " ".repeat(indent))
}

fn fence_start(raw: &str) -> Option<(char, usize, &str)> {
    let marker = raw.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let length = raw
        .chars()
        .take_while(|character| *character == marker)
        .count();
    if length < 3 {
        return None;
    }
    let info = raw[length..].trim();
    if marker == '`' && info.contains('`') {
        return None;
    }
    Some((marker, length, info.split_whitespace().next().unwrap_or("")))
}

fn closing_fence(raw: &str, open: &Fence) -> bool {
    let length = raw
        .chars()
        .take_while(|character| *character == open.marker)
        .count();
    length >= open.length && raw[length..].trim().is_empty()
}

fn heading(raw: &str, theme: Theme) -> Option<(&str, Style)> {
    let marks = raw.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&marks)
        || !raw
            .as_bytes()
            .get(marks)
            .is_some_and(u8::is_ascii_whitespace)
    {
        return None;
    }
    let style = match marks {
        1 => theme
            .style(Tone::Heading)
            .add_modifier(Modifier::UNDERLINED),
        2 => theme.style(Tone::Heading),
        3 => theme.style(Tone::Heading).add_modifier(Modifier::ITALIC),
        _ => theme.style(Tone::Default).add_modifier(Modifier::ITALIC),
    };
    Some((raw[marks..].trim_start(), style))
}

fn horizontal_rule(raw: &str) -> bool {
    let mut marks = raw.chars().filter(|character| !character.is_whitespace());
    let Some(marker @ ('-' | '*' | '_')) = marks.next() else {
        return false;
    };
    let mut count = 1;
    for character in marks {
        if character != marker {
            return false;
        }
        count += 1;
    }
    count >= 3
}

fn table_cells(raw: &str) -> Option<Vec<String>> {
    let mut cells = Vec::new();
    let mut start = 0;
    let mut code_length = 0;
    let mut escaped = false;
    let mut index = 0;
    while index < raw.len() {
        let character = raw[index..].chars().next()?;
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '`' {
            let length = raw[index..]
                .bytes()
                .take_while(|byte| *byte == b'`')
                .count();
            if code_length == 0 {
                code_length = length;
            } else if code_length == length {
                code_length = 0;
            }
            index += length;
            continue;
        } else if character == '|' && code_length == 0 {
            cells.push(raw[start..index].trim().to_owned());
            start = index + 1;
        }
        index += character.len_utf8();
    }
    if cells.is_empty() {
        return None;
    }
    cells.push(raw[start..].trim().to_owned());
    if raw.trim_start().starts_with('|') {
        cells.remove(0);
    }
    if raw.trim_end().ends_with('|') && cells.last().is_some_and(String::is_empty) {
        cells.pop();
    }
    (!cells.is_empty()).then_some(cells)
}

fn table_separator(raw: &str, columns: usize) -> bool {
    table_cells(raw).is_some_and(|cells| {
        cells.len() == columns
            && cells.iter().all(|cell| {
                let marks = cell.trim_matches(':');
                marks.len() >= 3 && marks.bytes().all(|byte| byte == b'-')
            })
    })
}

fn inline(raw: &str, base: Style, theme: Theme, depth: usize) -> Vec<Span<'static>> {
    // Nested emphasis and labels are bounded independently of provider text.
    if depth >= 16 {
        return vec![Span::styled(raw.to_owned(), base)];
    }
    let mut spans = Vec::new();
    let mut rest = raw;
    while !rest.is_empty() {
        if let Some(escaped) = rest.strip_prefix('\\')
            && let Some(character) = escaped.chars().next().filter(char::is_ascii_punctuation)
        {
            append(&mut spans, character.to_string(), base);
            rest = &escaped[character.len_utf8()..];
            continue;
        }
        if let Some((label, target, tail)) = link(rest) {
            let label = inline(label, base.patch(theme.style(Tone::Link)), theme, depth + 1);
            let label_text = Line::from(label.clone()).to_string();
            spans.extend(label);
            if label_text != target {
                // There is no terminal capability detection for OSC 8 yet.
                // Keeping the target in cells makes it selectable everywhere.
                append(&mut spans, format!(" ({target})"), theme.style(Tone::Dim));
            }
            rest = tail;
            continue;
        }
        let first = rest.as_bytes()[0];
        if matches!(first, b'`' | b'*' | b'_') {
            let count = rest.bytes().take_while(|byte| *byte == first).count();
            let body = &rest[count..];
            let is_code = first == b'`';
            if (is_code || count <= 3 && body.chars().next().is_some_and(|ch| !ch.is_whitespace()))
                && let Some((opening, end)) = closing_inline(body, first, count, !is_code)
            {
                let end = count + end;
                if is_code {
                    append(
                        &mut spans,
                        rest[opening..end].to_owned(),
                        base.patch(theme.style(Tone::Code))
                            .add_modifier(Modifier::DIM),
                    );
                } else {
                    let modifier = match opening {
                        1 => Modifier::ITALIC,
                        2 => Modifier::BOLD,
                        _ => Modifier::BOLD | Modifier::ITALIC,
                    };
                    spans.extend(inline(
                        &rest[opening..end],
                        base.add_modifier(modifier),
                        theme,
                        depth + 1,
                    ));
                }
                rest = &rest[end + opening..];
                continue;
            }
            // An unmatched double marker is indivisible; treating its second
            // star as a fresh opener would turn an unfinished bold into italic.
            append(&mut spans, rest[..count].to_owned(), base);
            rest = body;
            continue;
        }
        let end = rest
            .char_indices()
            .skip(1)
            .find(|(_, ch)| matches!(ch, '\\' | '[' | '`' | '*' | '_'))
            .map_or(rest.len(), |(index, _)| index);
        append(&mut spans, rest[..end].to_owned(), base);
        rest = &rest[end..];
    }
    spans
}

fn append(spans: &mut Vec<Span<'static>>, text: String, style: Style) {
    if let Some(previous) = spans.last_mut().filter(|span| span.style == style) {
        previous.content.to_mut().push_str(&text);
    } else {
        spans.push(Span::styled(text, style));
    }
}

fn closing_inline(raw: &str, marker: u8, count: usize, emphasis: bool) -> Option<(usize, usize)> {
    let mut openers = vec![count];
    let mut literal_inner_close = None;
    let mut index = 0;
    while index < raw.len() {
        let character = raw[index..].chars().next()?;
        if emphasis && character == '\\' {
            index += 1;
            index += raw[index..].chars().next().map_or(0, char::len_utf8);
            continue;
        }
        if raw.as_bytes()[index] == marker {
            let length = raw[index..]
                .bytes()
                .take_while(|byte| *byte == marker)
                .count();
            if !emphasis && length == count {
                return Some((count, index));
            }
            if emphasis && length <= 3 {
                let can_close = raw[..index]
                    .chars()
                    .next_back()
                    .is_some_and(|ch| !ch.is_whitespace());
                let can_open = raw[index + length..]
                    .chars()
                    .next()
                    .is_some_and(|ch| !ch.is_whitespace());
                if can_close && length == count && literal_inner_close.is_none() {
                    // An unmatched inner marker remains literal rather than
                    // preventing an otherwise complete outer span from closing.
                    literal_inner_close = Some((count, index));
                }
                let mut remaining = length;
                if can_close {
                    // A closing run belongs to the innermost opener first:
                    // `*a **b***` uses its first two stars for the inner bold.
                    while remaining > 0 {
                        let opening = *openers.last()?;
                        if remaining < opening {
                            if opening == 3 {
                                // A triple opener can close in two stages;
                                // its remaining markers enclose the outer span.
                                *openers.last_mut()? -= remaining;
                                remaining = 0;
                            }
                            break;
                        }
                        openers.pop();
                        remaining -= opening;
                        if openers.is_empty() {
                            return Some((opening, index + length - opening));
                        }
                    }
                }
                if can_open && remaining > 0 {
                    openers.push(remaining);
                }
            }
            index += length;
        } else if emphasis && character == '`' {
            let length = raw[index..]
                .bytes()
                .take_while(|byte| *byte == b'`')
                .count();
            if let Some((_, end)) = closing_inline(&raw[index + length..], b'`', length, false) {
                index += length + end + length;
            } else {
                index += length;
            }
        } else {
            index += character.len_utf8();
        }
    }
    literal_inner_close
}

fn link(raw: &str) -> Option<(&str, &str, &str)> {
    let label = raw.strip_prefix('[')?;
    let label_end = balanced_end(label, '[', ']')?;
    let target = label[label_end + 1..].strip_prefix('(')?;
    let target_end = balanced_end(target, '(', ')')?;
    let url = target[..target_end].trim();
    if url.is_empty() || url.chars().any(char::is_whitespace) {
        return None;
    }
    Some((&label[..label_end], url, &target[target_end + 1..]))
}

fn balanced_end(raw: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    let mut escaped = false;
    for (index, character) in raw.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == open {
            depth += 1;
        } else if character == close {
            if depth == 0 {
                return Some(index);
            }
            depth -= 1;
        }
    }
    None
}
