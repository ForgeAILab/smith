use std::path::Path;

#[derive(Debug)]
pub(crate) struct FixedClock;

impl agent_runtime_core::clock::Clock for FixedClock {
    fn now(&self) -> agent_runtime_core::clock::Timestamp {
        agent_runtime_core::clock::Timestamp(1_750_000_000_000)
    }
}

#[derive(Default)]
pub(crate) struct Normalizer {
    replacements: Vec<(String, String)>,
    dynamic: bool,
    headless_flow: Option<bool>,
}

impl Normalizer {
    pub(crate) fn new(home: &Path, project: &Path) -> Self {
        let mut this = Self {
            dynamic: true,
            ..Self::default()
        };
        for (path, placeholder) in [(project, "<PROJECT>"), (home, "<HOME>")] {
            let canonical = path.canonicalize().ok();
            for spelling in canonical.iter().map(|path| path.as_path()).chain([path]) {
                let spelling = spelling.display().to_string();
                // macOS reaches `/var` and `/tmp` through `/private`, so a
                // canonical path also appears in output without that prefix.
                if let Some(alias) = spelling.strip_prefix("/private") {
                    this.add(alias.to_owned(), placeholder);
                }
                this.add(spelling, placeholder);
            }
        }
        let project_id = smith_runtime::host::project_id(project).expect("project identity");
        this.add(project_id.to_string(), "<PROJECT_ID>");
        this
    }

    pub(crate) fn profile(&mut self, revision: &str) {
        self.add(revision.to_owned(), "<PROFILE_REVISION>");
        self.add(
            crate::resources::bounded_text(revision, 12),
            "<PROFILE_REVISION>…",
        );
    }

    pub(crate) fn child_session(&mut self, session: &str, index: usize) {
        self.add(session.to_owned(), &format!("<CHILD_SESSION_ID:{index}>"));
    }

    pub(crate) fn session(&mut self, session: &str) {
        self.add(session.to_owned(), "<SESSION_ID>");
    }

    // Enable flow IDs; optionally mask the terminal shutdown projection race.
    pub(crate) fn headless_flow(&mut self, mask_shutdown: bool) {
        self.headless_flow = Some(mask_shutdown);
    }

    fn add(&mut self, value: String, placeholder: &str) {
        if !value.is_empty() && !self.replacements.iter().any(|(known, _)| known == &value) {
            self.replacements.push((value, placeholder.to_owned()));
        }
    }

    // Rewrite scalar spans in place: never round-trip a result through a JSON map.
    // Canonical runtime counters, deterministic IDs, artifact content digests, and key order remain intact.
    pub(crate) fn normalize(&mut self, captured: &str) -> String {
        let mut text = captured.to_owned();
        if self.dynamic {
            for token in captured.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-') {
                if token.starts_with("a-")
                    && token.len() == 66
                    && token[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    let index = self
                        .replacements
                        .iter()
                        .filter(|(_, placeholder)| placeholder.starts_with("<ARTIFACT_ID:"))
                        .count()
                        + 1;
                    self.add(token.to_owned(), &format!("<ARTIFACT_ID:{index}>"));
                }
            }
            let mut spans = Vec::new();
            let bytes = captured.as_bytes();
            let mut at = 0;
            while at < bytes.len() {
                if bytes[at] != b'"' {
                    at += 1;
                    continue;
                }
                let end = string_end(bytes, at);
                if end <= at + 1 {
                    break;
                }
                let key = &captured[at + 1..end.saturating_sub(1)];
                let mut value_at = end;
                while bytes.get(value_at).is_some_and(u8::is_ascii_whitespace) {
                    value_at += 1;
                }
                if bytes.get(value_at) != Some(&b':') {
                    at = end;
                    continue;
                }
                value_at += 1;
                while bytes.get(value_at).is_some_and(u8::is_ascii_whitespace) {
                    value_at += 1;
                }
                if bytes.get(value_at) == Some(&b'"') {
                    let value_end = string_end(bytes, value_at);
                    let value = &captured[value_at + 1..value_end.saturating_sub(1)];
                    if matches!(
                        key,
                        "context"
                            | "snapshot"
                            | "view"
                            | "profile"
                            | "cache_plan"
                            | "cache_identity"
                            | "digest"
                            | "registry_snapshot"
                            | "scoped_view"
                            | "activation_revision"
                            | "harness_revision"
                            | "hash"
                            | "preparation_fingerprint"
                            | "argument_fingerprint"
                    ) && matches!(value.len(), 32 | 64)
                        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        let index = self
                            .replacements
                            .iter()
                            .filter(|(_, placeholder)| placeholder.starts_with("<FINGERPRINT:"))
                            .count()
                            + 1;
                        self.add(value.to_owned(), &format!("<FINGERPRINT:{index}>"));
                    }
                } else if matches!(
                    key,
                    "duration_ms"
                        | "elapsed_ms"
                        | "latency_ms"
                        | "idle_compaction_latency_ms"
                        | "backoff_ms"
                        | "delay_ms"
                ) {
                    let mut value_end = value_at;
                    while bytes.get(value_end).is_some_and(u8::is_ascii_digit) {
                        value_end += 1;
                    }
                    if value_end > value_at {
                        spans.push((value_at, value_end));
                    }
                }
                at = end;
            }
            for (start, end) in spans.into_iter().rev() {
                text.replace_range(start..end, "\"<DURATION>\"");
            }
            for line in captured.lines().filter(|line| line.contains("identity")) {
                for token in line.split_whitespace() {
                    if matches!(token.len(), 32 | 64)
                        && token.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        let index = self
                            .replacements
                            .iter()
                            .filter(|(_, placeholder)| placeholder.starts_with("<FINGERPRINT:"))
                            .count()
                            + 1;
                        self.add(token.to_owned(), &format!("<FINGERPRINT:{index}>"));
                    }
                }
            }
        }
        // Human timestamps contain a local offset even with an injected clock.
        for millis in [1_750_000_000_000, 1_750_000_600_000] {
            text = text.replace(
                &smith_client::time_display::local_timestamp(millis),
                "<TIMESTAMP>",
            );
        }
        for line in text.clone().lines() {
            if let Some((_, suffix)) = line.split_once(" · deadline ") {
                let timestamp = suffix.split(" · ").next().expect("deadline value");
                text = text.replace(timestamp, "<TIMESTAMP>");
            }
        }
        let mut replacements = self.replacements.iter().collect::<Vec<_>>();
        replacements.sort_by_key(|(value, _)| std::cmp::Reverse(value.len()));
        for (value, placeholder) in replacements {
            text = text.replace(value, placeholder);
        }
        if let Some(mask_shutdown) = self.headless_flow {
            normalize_headless_capture(&text, mask_shutdown)
        } else {
            text
        }
    }
}

fn string_end(bytes: &[u8], start: usize) -> usize {
    let mut at = start + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'"' => return at + 1,
            _ => at += 1,
        }
    }
    bytes.len()
}

type CaptureEdit = (std::ops::Range<usize>, String);

// Flow-only, after the existing normalizer so its fingerprint numbering is
// unchanged. These two registries start fresh for every captured file.
fn normalize_headless_capture(captured: &str, mask_shutdown: bool) -> String {
    let mut edits = Vec::new();
    let mut interactions = Vec::new();
    for (start, _) in captured.match_indices("interaction-") {
        let end = start + "interaction-".len() + 32;
        if captured
            .get(start + "interaction-".len()..end)
            .is_some_and(|hex| hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            && !captured
                .as_bytes()
                .get(end)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
            && (start == 0
                || !captured
                    .as_bytes()
                    .get(start - 1)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'-'))
        {
            interactions.push(start..end);
        }
    }
    numbered_edits(captured, interactions, "INTERACTION_ID", &mut edits);

    let mut fingerprints = Vec::new();
    let mut offset = 0;
    for line in captured.split_inclusive('\n') {
        if line.starts_with("smith: approval required for tool ")
            && let Some((prefix, suffix)) = line.split_once(" · fingerprint ")
            && let Some(value) = suffix.split_whitespace().next()
            && approval_fingerprint(value)
        {
            let start = offset + prefix.len() + " · fingerprint ".len();
            fingerprints.push(start..start + value.len());
        }
        if captured.trim_start().starts_with('{') {
            let root = offset + line.len() - line.trim_start().len()..offset + line.len();
            if let Some(value) = json_path(
                captured,
                root.clone(),
                &["approval_required", "preparation_fingerprint"],
            ) && captured.as_bytes().get(value.start) == Some(&b'"')
                && approval_fingerprint(&captured[value.start + 1..value.end - 1])
            {
                fingerprints.push(value.start + 1..value.end - 1);
            }
            if mask_shutdown {
                shutdown_projection_edits(captured, root, &mut edits);
            }
        }
        offset += line.len();
    }
    numbered_edits(captured, fingerprints, "APPROVAL_FINGERPRINT", &mut edits);
    edits.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
    let mut normalized = captured.to_owned();
    for (span, replacement) in edits {
        normalized.replace_range(span, &replacement);
    }
    normalized
}

fn approval_fingerprint(value: &str) -> bool {
    (matches!(value.len(), 32 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || (value.starts_with("<FINGERPRINT:") && value.ends_with('>'))
}

fn numbered_edits(
    captured: &str,
    mut spans: Vec<std::ops::Range<usize>>,
    label: &str,
    edits: &mut Vec<CaptureEdit>,
) {
    spans.sort_by_key(|span| span.start);
    let mut values = Vec::new();
    for span in spans {
        let value = &captured[span.clone()];
        let index = values
            .iter()
            .position(|known| *known == value)
            .unwrap_or_else(|| {
                values.push(value);
                values.len() - 1
            });
        edits.push((span, format!("<{label}:{}>", index + 1)));
    }
}

// Masks a product race: headless shutdown can cancel the cache worker before
// it reduces the final PlanUpdated or TurnCompleted (cache_controller.rs:631;
// host.rs:558). Only mask exact_state.plan's revision and failed counters,
// lifecycle.last_event_sequence, exact_state.watermark, last_persisted_watermark,
// idle_compaction.interval_id, cache.idle_compaction_interval_id, and
// retained_recent_turns in terminal projections. Race edits leave runtime_event
// envelopes and validations.*.watermark verbatim.
fn shutdown_projection_edits(
    captured: &str,
    root: std::ops::Range<usize>,
    edits: &mut Vec<CaptureEdit>,
) {
    let kind = json_path(captured, root.clone(), &["type"]).map(|span| &captured[span]);
    let controller = match kind {
        Some("\"result\"") => json_path(captured, root.clone(), &["cache", "controller"]),
        Some("\"cache_controller\"") => json_path(captured, root.clone(), &["controller"]),
        _ => return,
    };
    if let Some(controller) = controller {
        scalar_edit(
            captured,
            controller.clone(),
            &["lifecycle", "last_event_sequence"],
            "\"<SHUTDOWN_SEQ>\"",
            edits,
        );
        if let Some(idle) = json_path(captured, controller, &["idle_compaction"]) {
            interval_edit(captured, idle, "interval_id", "attempted", edits);
        }
    }
    if kind == Some("\"result\"")
        && let Some(capsule) = json_path(captured, root, &["resume_capsule"])
    {
        for (key, placeholder) in [
            ("revision", "\"<SHUTDOWN_PLAN_REVISION>\""),
            ("failed", "\"<SHUTDOWN_PLAN_FAILED>\""),
        ] {
            scalar_edit(
                captured,
                capsule.clone(),
                &["exact_state", "plan", key],
                placeholder,
                edits,
            );
        }
        scalar_edit(
            captured,
            capsule.clone(),
            &["exact_state", "watermark"],
            "\"<SHUTDOWN_SEQ>\"",
            edits,
        );
        scalar_edit(
            captured,
            capsule.clone(),
            &["last_persisted_watermark"],
            "\"<SHUTDOWN_SEQ>\"",
            edits,
        );
        scalar_edit(
            captured,
            capsule.clone(),
            &["retained_recent_turns"],
            "[\"<SHUTDOWN_RECENT_TURNS>\"]",
            edits,
        );
        if let Some(cache) = json_path(captured, capsule, &["cache"]) {
            interval_edit(
                captured,
                cache,
                "idle_compaction_interval_id",
                "idle_compaction_attempted",
                edits,
            );
        }
    }
}

fn scalar_edit(
    captured: &str,
    root: std::ops::Range<usize>,
    path: &[&str],
    replacement: &str,
    edits: &mut Vec<CaptureEdit>,
) {
    if let Some(span) = json_path(captured, root, path) {
        edits.push((span, replacement.to_owned()));
    }
}

fn interval_edit(
    captured: &str,
    object: std::ops::Range<usize>,
    key: &str,
    following: &str,
    edits: &mut Vec<CaptureEdit>,
) {
    if let Some(member) = json_member(captured, object.clone(), key) {
        edits.push((member.value, "\"<SHUTDOWN_INTERVAL_ID>\"".to_owned()));
    } else if let Some(member) = json_member(captured, object, following) {
        // Insert at the serializer's existing field position; never reorder
        // any key or round-trip the output through a JSON map.
        edits.push((
            member.key_start..member.key_start,
            format!("\"{key}\":\"<SHUTDOWN_INTERVAL_ID>\","),
        ));
    }
}

struct JsonMember {
    key_start: usize,
    value: std::ops::Range<usize>,
}

fn json_path(
    captured: &str,
    mut object: std::ops::Range<usize>,
    path: &[&str],
) -> Option<std::ops::Range<usize>> {
    for key in path {
        object = json_member(captured, object, key)?.value;
    }
    Some(object)
}

fn json_member(captured: &str, object: std::ops::Range<usize>, key: &str) -> Option<JsonMember> {
    let bytes = captured.as_bytes();
    if bytes.get(object.start) != Some(&b'{') {
        return None;
    }
    let mut at = object.start + 1;
    while at < object.end {
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if bytes.get(at) != Some(&b'"') {
            return None;
        }
        let key_start = at;
        let end = string_end(bytes, at);
        at = end;
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if bytes.get(at) != Some(&b':') {
            return None;
        }
        at += 1;
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        let value = at..json_value_end(bytes, at);
        if captured.get(key_start + 1..end - 1) == Some(key) {
            return Some(JsonMember { key_start, value });
        }
        at = value.end;
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if bytes.get(at) != Some(&b',') {
            return None;
        }
        at += 1;
    }
    None
}

fn json_value_end(bytes: &[u8], start: usize) -> usize {
    match bytes.get(start) {
        Some(b'"') => string_end(bytes, start),
        Some(b'{' | b'[') => {
            let mut depth = 1;
            let mut at = start + 1;
            while at < bytes.len() {
                match bytes[at] {
                    b'"' => {
                        at = string_end(bytes, at);
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return at + 1;
                        }
                    }
                    _ => {}
                }
                at += 1;
            }
            bytes.len()
        }
        _ => (start..bytes.len())
            .find(|&at| matches!(bytes[at], b',' | b'}' | b']') || bytes[at].is_ascii_whitespace())
            .unwrap_or(bytes.len()),
    }
}

pub(crate) fn compare_or_update(relative: &str, captured: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative);
    let record = "SMITH_UPDATE_FIXTURES=1 cargo test -p smith-cli --locked --bin smith fixtures_";
    if std::env::var_os("SMITH_UPDATE_FIXTURES").is_some_and(|value| value == "1") {
        std::fs::create_dir_all(path.parent().expect("fixture directory"))
            .expect("create fixture directory");
        std::fs::write(&path, captured)
            .unwrap_or_else(|error| panic!("writing {}: {error}", path.display()));
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "reading {}: {error}\nRe-record with: {record}",
            path.display()
        )
    });
    let expected_lines = expected.split('\n').collect::<Vec<_>>();
    let actual_lines = captured.split('\n').collect::<Vec<_>>();
    let mut diff = String::new();
    for index in 0..expected_lines.len().max(actual_lines.len()) {
        let before = expected_lines.get(index);
        let after = actual_lines.get(index);
        if before != after {
            diff.push_str(&format!("@@ line {} @@\n", index + 1));
            if let Some(before) = before {
                diff.push_str(&format!("-{before}\n"));
            }
            if let Some(after) = after {
                diff.push_str(&format!("+{after}\n"));
            }
        }
    }
    assert_eq!(
        expected,
        captured,
        "fixture {}\n{diff}\nRe-record with: {record}",
        path.display()
    );
}
#[test]
fn fixtures_normalization_keeps_values_and_order() {
    let mut normalizer = Normalizer {
        dynamic: true,
        ..Normalizer::default()
    };
    normalizer.session("session-random");
    let captured = r#"{"z":120,"session":"session-random","context":"0123456789abcdef0123456789abcdef","delay_ms":17,"used_percent":75,"digest":{"hex":"abcd"},"a":2}"#;
    assert_eq!(
        normalizer.normalize(captured),
        r#"{"z":120,"session":"<SESSION_ID>","context":"<FINGERPRINT:1>","delay_ms":"<DURATION>","used_percent":75,"digest":{"hex":"abcd"},"a":2}"#
    );
    assert_eq!(
        normalizer.normalize("tokens: 120 · cost $0.002 · 75%"),
        "tokens: 120 · cost $0.002 · 75%"
    );
    assert_eq!(
        normalizer.normalize(r#"trailing quote ""#),
        r#"trailing quote ""#
    );

    let early = r#"{"type":"result","cache":{"controller":{"lifecycle":{"last_event_sequence":14,"tokens":80},"idle_compaction":{"attempted":false}}},"resume_capsule":{"exact_state":{"watermark":14,"validations":{"call-shell":{"watermark":28}}},"retained_recent_turns":[],"cache":{"idle_compaction_attempted":false},"last_persisted_watermark":14},"usage":120,"cost":0.002}"#;
    let late = r#"{"type":"result","cache":{"controller":{"lifecycle":{"last_event_sequence":15,"tokens":80},"idle_compaction":{"interval_id":"root-turn:turn-1","attempted":false}}},"resume_capsule":{"exact_state":{"watermark":15,"validations":{"call-shell":{"watermark":28}}},"retained_recent_turns":[{"turn":"turn-1","role":"assistant"}],"cache":{"idle_compaction_interval_id":"root-turn:turn-1","idle_compaction_attempted":false},"last_persisted_watermark":15},"usage":120,"cost":0.002}"#;
    let canonical = r#"{"type":"result","cache":{"controller":{"lifecycle":{"last_event_sequence":"<SHUTDOWN_SEQ>","tokens":80},"idle_compaction":{"interval_id":"<SHUTDOWN_INTERVAL_ID>","attempted":false}}},"resume_capsule":{"exact_state":{"watermark":"<SHUTDOWN_SEQ>","validations":{"call-shell":{"watermark":28}}},"retained_recent_turns":["<SHUTDOWN_RECENT_TURNS>"],"cache":{"idle_compaction_interval_id":"<SHUTDOWN_INTERVAL_ID>","idle_compaction_attempted":false},"last_persisted_watermark":"<SHUTDOWN_SEQ>"},"usage":120,"cost":0.002}"#;
    assert_eq!(normalizer.normalize(early), early, "flow rules are opt-in");
    normalizer.headless_flow(true);
    assert_eq!(normalizer.normalize(early), canonical);
    assert_eq!(normalizer.normalize(late), canonical);

    let event = r#"{"type":"runtime_event","event":{"seq":15,"id":"event-15","payload":{"watermark":15,"cache":{"controller":{"lifecycle":{"last_event_sequence":15}}}}}}"#;
    let early_controller = r#"{"type":"cache_controller","controller":{"lifecycle":{"last_event_sequence":14},"idle_compaction":{"attempted":false}}}"#;
    let late_controller = r#"{"type":"cache_controller","controller":{"lifecycle":{"last_event_sequence":15},"idle_compaction":{"interval_id":"root-turn:turn-1","attempted":false}}}"#;
    let canonical_controller = r#"{"type":"cache_controller","controller":{"lifecycle":{"last_event_sequence":"<SHUTDOWN_SEQ>"},"idle_compaction":{"interval_id":"<SHUTDOWN_INTERVAL_ID>","attempted":false}}}"#;
    assert_eq!(
        normalizer.normalize(event),
        event,
        "runtime events remain verbatim"
    );
    for controller in [early_controller, late_controller] {
        assert_eq!(
            normalizer.normalize(&format!("{event}\n{controller}\n{early}\n")),
            format!("{event}\n{canonical_controller}\n{canonical}\n")
        );
    }

    let interactions = r#"{"type":"runtime_event","event":{"seq":14,"payload":{"request":"interaction-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}}
{"type":"result","interaction_required":{"request_id":"interaction-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"same":"interaction-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}"#;
    assert_eq!(
        normalizer.normalize(interactions),
        r#"{"type":"runtime_event","event":{"seq":14,"payload":{"request":"<INTERACTION_ID:1>"}}}
{"type":"result","interaction_required":{"request_id":"<INTERACTION_ID:2>"},"same":"<INTERACTION_ID:1>"}"#
    );
    assert_eq!(
        normalizer.normalize(
            "smith: interaction required for request `interaction-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`"
        ),
        "smith: interaction required for request `<INTERACTION_ID:1>`",
        "numbering restarts for each captured file"
    );
    let approval = "smith: approval required for tool `edit` · fingerprint e532172a1e26eb2788a5cb05f37293a9 · values protected";
    assert_eq!(
        normalizer.normalize(approval),
        "smith: approval required for tool `edit` · fingerprint <APPROVAL_FINGERPRINT:1> · values protected"
    );
    assert_eq!(
            normalizer.normalize(r#"{"type":"result","approval_required":{"preparation_fingerprint":"e532172a1e26eb2788a5cb05f37293a9"}}"#),
            r#"{"type":"result","approval_required":{"preparation_fingerprint":"<APPROVAL_FINGERPRINT:1>"}}"#
        );
    let second_approval = "smith: approval required for tool `edit` · fingerprint 0123456789abcdef0123456789abcdef · values protected";
    assert_eq!(
        normalizer.normalize(&format!("{approval}\n{second_approval}\n{approval}")),
        "smith: approval required for tool `edit` · fingerprint <APPROVAL_FINGERPRINT:1> · values protected\nsmith: approval required for tool `edit` · fingerprint <APPROVAL_FINGERPRINT:2> · values protected\nsmith: approval required for tool `edit` · fingerprint <APPROVAL_FINGERPRINT:1> · values protected"
    );
    normalizer.headless_flow(false);
    assert_eq!(
        normalizer.normalize(early),
        early,
        "unaffected flows retain shutdown fields"
    );
}

#[test]
fn fixtures_normalization_masks_only_shutdown_capsule_plan_counters() {
    let early = r#"{"type":"result","lifecycle":{"plan":{"revision":2,"counts":{"cancelled":1}}},"resume_capsule":{"exact_state":{"plan":{"revision":1,"pending":0,"completed":1,"failed":0}}},"usage":120}"#;
    let late = r#"{"type":"result","lifecycle":{"plan":{"revision":2,"counts":{"cancelled":1}}},"resume_capsule":{"exact_state":{"plan":{"revision":2,"pending":0,"completed":1,"failed":1}}},"usage":120}"#;
    let canonical = r#"{"type":"result","lifecycle":{"plan":{"revision":2,"counts":{"cancelled":1}}},"resume_capsule":{"exact_state":{"plan":{"revision":"<SHUTDOWN_PLAN_REVISION>","pending":0,"completed":1,"failed":"<SHUTDOWN_PLAN_FAILED>"}}},"usage":120}"#;
    let event = r#"{"type":"runtime_event","event":{"seq":36,"payload":{"event":"plan_updated","revision":2,"counts":{"completed":1,"cancelled":1}}}}"#;
    let mut normalizer = Normalizer::default();
    assert_eq!(normalizer.normalize(early), early);
    normalizer.headless_flow(true);
    for result in [early, late] {
        assert_eq!(normalizer.normalize(result), canonical);
        assert_eq!(
            normalizer.normalize(&format!("{event}\n{result}\n")),
            format!("{event}\n{canonical}\n")
        );
    }
    normalizer.headless_flow(false);
    assert_eq!(normalizer.normalize(early), early);
    assert_eq!(normalizer.normalize(late), late);
}
