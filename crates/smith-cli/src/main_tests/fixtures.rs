// Before-refactor recordings. The headless tests use the same comparison and normalization.
pub(crate) mod fixture_support {
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
                if let Ok(canonical) = path.canonicalize() {
                    this.add(canonical.display().to_string(), placeholder);
                }
                this.add(path.display().to_string(), placeholder);
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
        // Runtime counters, deterministic IDs, artifact content digests, and key order remain intact.
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
    // it reduces TurnCompleted (cache_controller.rs:630; host.rs:558). Only mask
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

    fn json_member(
        captured: &str,
        object: std::ops::Range<usize>,
        key: &str,
    ) -> Option<JsonMember> {
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
                .find(|&at| {
                    matches!(bytes[at], b',' | b'}' | b']') || bytes[at].is_ascii_whitespace()
                })
                .unwrap_or(bytes.len()),
        }
    }

    pub(crate) fn compare_or_update(relative: &str, captured: &str) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative);
        let record =
            "SMITH_UPDATE_FIXTURES=1 cargo test -p smith-cli --locked --bin smith fixtures_";
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
            normalizer.normalize("smith: interaction required for request `interaction-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`"),
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
}

async fn fixture_local_host(
    home: &std::path::Path,
    project: &std::path::Path,
    provider: Arc<dyn agent_runtime_core::provider::Provider>,
    sources: smith_runtime::skills::SmithSkillSources,
) -> HostSession {
    let config = resolve(&ResolveRequest::new(project).with_home_dir(home))
        .expect("resolution")
        .config;
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(ProjectWorkspace::new(project).expect("workspace"))),
        approval: Some(Arc::new(agent_runtime_core::approval::AllowAll)),
        provider: Some(provider),
        skills: sources,
        clock: Some(Arc::new(fixture_support::FixedClock)),
        ..RuntimeRequest::new(config, HostSurface::Terminal)
    };
    let host = Box::pin(smith_runtime::host::start(
        HostSessionRequest::new(runtime, project).checkpoint_keys(Arc::new(TestCheckpointKeys)),
    ))
    .await
    .expect("host");
    host.set_goal_continuation_enabled(false);
    host
}

fn fixture_local_app(planned: bool, usage: bool) -> App {
    let mut app = App::new("example-model", "<PROJECT>");
    if planned {
        let totals = BTreeMap::from([
            (
                agent_runtime_core::manifest::SegmentKind::new("system_instruction"),
                200,
            ),
            (
                agent_runtime_core::manifest::SegmentKind::new("tool_schema"),
                500,
            ),
            (
                agent_runtime_core::manifest::SegmentKind::new("history"),
                1_300,
            ),
        ]);
        app.status.record_context_plan(ContextPlanUpdate {
            fingerprint: "context-test",
            cache_fingerprint: "cache-test",
            input_tokens: 2_000,
            input_budget_tokens: 123_904,
            reserved_tokens: 4_096,
            segment_count: 3,
            totals: &totals,
            confidence: EstimationConfidence::Estimated,
        });
        app.status.record_registry("registry-test", 6);
        app.status.record_scoped_view("view-test", 4);
        app.status.record_retrieval(
            "resolver-test",
            vec!["tool:read".into(), "tool:search".into()],
        );
        app.status.record_activation(1, vec!["tool:read".into()]);
        app.status.record_compaction(250);
    }
    if usage {
        app.status.record_usage(
            &agent_runtime_core::usage::UsageDelta::new()
                .with(CounterKind::InputUncached, 1_000)
                .with(CounterKind::InputCached, 800)
                .with(CounterKind::Output, 120),
        );
    }
    app
}

fn fixture_raw_and_view(app: &App, normalizer: &mut fixture_support::Normalizer) -> (String, App) {
    let mut raw = String::new();
    let mut view = App::new("example-model", "<PROJECT>");
    view.children = app.children.clone();
    if let Some(child) = &app.inspected_child {
        let detail = app.inspected_detail().expect("child status card");
        let content = smith_client::agent_report::render_plain(
            &smith_client::agent_report::AgentReport::Inspector(detail.clone()),
        );
        raw.push_str(&format!(
            "title: agent {child}\nstate: Inspector\nbody:\n{content}\n"
        ));
        view.inspect_child(child.clone());
        view.set_inspected_detail(child, Some(fixture_agent_snapshot_view(detail, normalizer)));
    }
    for block in app.transcript.blocks() {
        match block {
            Block::Local(result) => {
                let title = result.title();
                let state = match result {
                    LocalResult::Agent(report)
                        if matches!(
                            report.as_ref(),
                            smith_client::agent_report::AgentReport::Resume(
                                smith_client::agent_report::AgentResumeReport::RequiresIdle
                                    | smith_client::agent_report::AgentResumeReport::Started { .. }
                            )
                        ) => "Notice".to_owned(),
                    LocalResult::Review(report)
                        if matches!(
                            report.as_ref(),
                            smith_client::review_report::ReviewReport::Empty
                                | smith_client::review_report::ReviewReport::Start(
                                    smith_client::review_report::ReviewStartReport::Started { .. }
                                        | smith_client::review_report::ReviewStartReport::Queued { .. }
                                )
                        ) => "Notice".to_owned(),
                    LocalResult::Recovery(report) if report.is_notice() => "Notice".to_owned(),
                    _ => format!("{:?}", result.state()),
                };
                let content = match result {
                    LocalResult::Status(report) => smith_client::status_report::render_plain(report),
                    LocalResult::Diagnostics(report) => smith_client::diagnostics_report::render_plain(report),
                    LocalResult::Context(report) => smith_client::context_report::render_plain(report),
                    LocalResult::Help(report) => smith_client::help_report::render_plain(report),
                    LocalResult::Timeline(report) => {
                        smith_client::timeline_report::render_plain(report)
                    }
                    LocalResult::Goal(report) => smith_client::goal_report::render_plain(report),
                    LocalResult::Agent(report) => smith_client::agent_report::render_plain(report),
                    LocalResult::Mcp(report) => smith_client::mcp_report::render_plain(report),
                    LocalResult::Skills(report) => smith_client::skills_report::render_plain(report),
                    LocalResult::Diff(report) => smith_client::diff_report::render_plain(report),
                    LocalResult::Review(report) => smith_client::review_report::render_plain(report),
                    LocalResult::Recovery(report) => smith_client::recovery_report::render_plain(report),
                    LocalResult::Text { body, .. } => body.clone(),
                };
                raw.push_str(&format!(
                    "title: {title}\nstate: {state}\nbody:\n{content}\n"
                ));
                // Draw a normalized typed report, never a prose round-trip.
                let normalized = match result {
                    LocalResult::Status(report) => LocalResult::Status(Box::new(
                        fixture_status_view(report, normalizer),
                    )),
                    LocalResult::Diagnostics(report) => LocalResult::Diagnostics(Box::new(
                        fixture_diagnostics_view(report, normalizer),
                    )),
                    LocalResult::Context(report) => LocalResult::Context(Box::new(
                        fixture_context_view(report, normalizer),
                    )),
                    LocalResult::Help(report) => LocalResult::Help(Box::new(
                        fixture_help_view(report, normalizer),
                    )),
                    LocalResult::Timeline(report) => LocalResult::Timeline(Box::new(
                        fixture_timeline_view(report, normalizer),
                    )),
                    LocalResult::Goal(report) => LocalResult::Goal(Box::new(
                        fixture_goal_view(report, normalizer),
                    )),
                    LocalResult::Agent(report) => LocalResult::Agent(Box::new(
                        fixture_agent_view(report, normalizer),
                    )),
                    LocalResult::Mcp(report) => LocalResult::Mcp(Box::new(
                        fixture_mcp_view(report, normalizer),
                    )),
                    LocalResult::Skills(report) => LocalResult::Skills(Box::new(
                        fixture_skills_view(report, normalizer),
                    )),
                    LocalResult::Diff(report) => LocalResult::Diff(Box::new(
                        fixture_diff_view(report, normalizer),
                    )),
                    LocalResult::Review(report) => LocalResult::Review(Box::new(
                        fixture_review_view(report, normalizer),
                    )),
                    LocalResult::Recovery(report) => LocalResult::Recovery(Box::new(
                        fixture_recovery_view(report, normalizer),
                    )),
                    LocalResult::Text { title, body, state } => LocalResult::Text {
                        title: normalizer.normalize(title),
                        body: normalizer.normalize(body),
                        state: *state,
                    },
                };
                view.transcript.push_local(normalized);
            }
            Block::Error { message } => {
                raw.push_str(&format!("title: error\nstate: Error\nbody:\n{message}\n"));
                view.transcript.push_error(normalizer.normalize(message));
            }
            Block::Notice { source, text } => {
                raw.push_str(&format!("title: {source}\nstate: Notice\nbody:\n{text}\n"));
                view.transcript
                    .push_notice(normalizer.normalize(source), normalizer.normalize(text));
            }
            _ => panic!("unexpected local fixture block: {block:?}"),
        }
    }
    use smith_tui::Overlay;
    let overlay = match &app.overlay {
        Some(Overlay::UndoConfirm { report }) => {
            use smith_client::recovery_report::RecoveryReport;

            let report = RecoveryReport::UndoConfirmation((**report).clone());
            let content = smith_client::recovery_report::render_plain(&report);
            raw.push_str(&format!(
                "title: undo\nstate: Confirmation\nbody:\n{content}\n"
            ));
            let RecoveryReport::UndoConfirmation(preview) = fixture_recovery_view(&report, normalizer)
            else {
                panic!("expected an undo confirmation");
            };
            view.overlay = Some(Overlay::UndoConfirm {
                report: Box::new(preview),
            });
            None
        }
        Some(Overlay::RedoConfirm { report }) => {
            use smith_client::recovery_report::RecoveryReport;

            let report = RecoveryReport::RedoConfirmation((**report).clone());
            let content = smith_client::recovery_report::render_plain(&report);
            raw.push_str(&format!(
                "title: redo\nstate: Confirmation\nbody:\n{content}\n"
            ));
            let RecoveryReport::RedoConfirmation(preview) = fixture_recovery_view(&report, normalizer)
            else {
                panic!("expected a redo confirmation");
            };
            view.overlay = Some(Overlay::RedoConfirm {
                report: Box::new(preview),
            });
            None
        }
        Some(Overlay::RevertConfirm { report }) => {
            use smith_client::recovery_report::RecoveryReport;

            let report = RecoveryReport::RevertConfirmation((**report).clone());
            let content = smith_client::recovery_report::render_plain(&report);
            raw.push_str(&format!(
                "title: revert\nstate: Confirmation\nbody:\n{content}\n"
            ));
            let RecoveryReport::RevertConfirmation(preview) = fixture_recovery_view(&report, normalizer)
            else {
                panic!("expected a revert confirmation");
            };
            view.overlay = Some(Overlay::RevertConfirm {
                report: Box::new(preview),
            });
            None
        }
        Some(Overlay::ReviewConfirm { report }) => {
            use smith_client::review_report::ReviewReport;

            let report = ReviewReport::Confirmation((**report).clone());
            let content = smith_client::review_report::render_plain(&report);
            raw.push_str(&format!(
                "title: review\nstate: Confirmation\nbody:\n{content}\n"
            ));
            let ReviewReport::Confirmation(preview) = fixture_review_view(&report, normalizer)
            else {
                panic!("expected a review confirmation");
            };
            view.overlay = Some(Overlay::ReviewConfirm {
                report: Box::new(preview),
            });
            None
        }
        Some(Overlay::McpTrustConfirm { server, content }) => Some((
            "mcp trust",
            content,
            Overlay::McpTrustConfirm {
                server: server.clone(),
                content: normalizer.normalize(content),
            },
        )),
        Some(Overlay::SkillTrustConfirm { skill, content }) => Some((
            "skills trust",
            content,
            Overlay::SkillTrustConfirm {
                skill: skill.clone(),
                content: normalizer.normalize(content),
            },
        )),
        Some(Overlay::ResourcePicker {
            picker,
            target,
            restore_on_escape,
        }) => {
            raw.push_str(&format!(
                "title: {}\nstate: Picker\nbody:\n{}\n",
                picker.title, picker.empty_guidance
            ));
            for entry in &picker.entries {
                raw.push_str(&format!(
                    "{} · {} · {} · active {} · disabled {:?}\n",
                    entry.id, entry.label, entry.detail, entry.active, entry.disabled_reason
                ));
            }
            view.overlay = Some(Overlay::ResourcePicker {
                picker: picker.clone(),
                target: *target,
                restore_on_escape: restore_on_escape.clone(),
            });
            None
        }
        None => None,
        other => panic!("unexpected local fixture overlay: {other:?}"),
    };
    if let Some((title, content, overlay)) = overlay {
        raw.push_str(&format!(
            "title: {title}\nstate: Confirmation\nbody:\n{content}\n"
        ));
        view.overlay = Some(overlay);
    }
    assert!(!raw.is_empty(), "command produced no captured local result");
    (normalizer.normalize(&raw), view)
}

fn fixture_status_view(
    report: &smith_client::status_report::StatusReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::status_report::StatusReport {
    let mut report = report.clone();
    for value in [
        &mut report.session,
        &mut report.profile,
        &mut report.provider,
        &mut report.model,
        &mut report.permission,
        &mut report.reasoning,
        &mut report.reasoning_controls,
        &mut report.prompt_cache,
        &mut report.cache_maintenance,
        &mut report.resume_checkpoint,
        &mut report.project,
        &mut report.git,
        &mut report.usage,
        &mut report.cost,
    ] {
        *value = normalizer.normalize(value);
    }
    match &mut report.goal {
        smith_client::status_report::StatusGoal::None => {}
        smith_client::status_report::StatusGoal::Unavailable(error) => {
            *error = normalizer.normalize(error);
        }
        smith_client::status_report::StatusGoal::Active(goal) => {
            for value in [
                &mut goal.objective,
                &mut goal.status,
                &mut goal.tokens,
                &mut goal.budget,
                &mut goal.active_elapsed,
                &mut goal.reason,
                &mut goal.id,
            ] {
                *value = normalizer.normalize(value);
            }
        }
    }
    report
}

fn fixture_diagnostics_view(
    report: &smith_client::diagnostics_report::DiagnosticsReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::diagnostics_report::DiagnosticsReport {
    use smith_client::diagnostics_report::DiagnosticsRow;

    let mut report = report.clone();
    for section in &mut report.sections {
        for row in &mut section.rows {
            match row {
                DiagnosticsRow::Field { label, value } => {
                    *label = normalizer.normalize(label);
                    *value = normalizer.normalize(value);
                }
                DiagnosticsRow::Line(line) => *line = normalizer.normalize(line),
            }
        }
    }
    report
}

fn fixture_context_view(
    report: &smith_client::context_report::ContextReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::context_report::ContextReport {
    let mut report = report.clone();
    for value in [
        &mut report.summary,
        &mut report.free_input.value,
        &mut report.reserve.value,
        &mut report.model_window,
        &mut report.counting,
        &mut report.tool_context,
        &mut report.provider_input,
        &mut report.cache_read,
        &mut report.cache,
        &mut report.reasoning,
        &mut report.reasoning_controls,
    ] {
        *value = normalizer.normalize(value);
    }
    for window in &mut report.available_windows {
        window.name = normalizer.normalize(&window.name);
    }
    for category in &mut report.categories {
        category.label = normalizer.normalize(&category.label);
        category.value = normalizer.normalize(&category.value);
    }
    match &mut report.compaction {
        smith_client::context_report::ContextCompaction::Enabled { recovery_target } => {
            *recovery_target = normalizer.normalize(recovery_target);
        }
        smith_client::context_report::ContextCompaction::Applied {
            summary,
            recovery_target,
        } => {
            *summary = normalizer.normalize(summary);
            *recovery_target = normalizer.normalize(recovery_target);
        }
    }
    report
}

fn fixture_help_view(
    report: &smith_client::help_report::HelpReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::help_report::HelpReport {
    let mut report = report.clone();
    report.introduction = normalizer.normalize(&report.introduction);
    for command in report
        .getting_started
        .iter_mut()
        .chain(&mut report.primary)
        .chain(&mut report.advanced)
    {
        for value in [
            &mut command.name,
            &mut command.argument_hint,
            &mut command.description,
        ] {
            *value = normalizer.normalize(value);
        }
    }
    for guidance in &mut report.composer {
        *guidance = normalizer.normalize(guidance);
    }
    report
}

fn fixture_agent_summary_view(
    summary: &mut smith_client::agent_report::AgentSummary,
    normalizer: &mut fixture_support::Normalizer,
) {
    summary.child = normalizer.normalize(&summary.child);
    if let smith_client::agent_report::ChildState::Stopped { reason } = &mut summary.state {
        *reason = normalizer.normalize(reason);
    }
}

fn fixture_agent_snapshot_view(
    snapshot: &smith_client::agent_report::AgentSnapshot,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::agent_report::AgentSnapshot {
    let mut snapshot = snapshot.clone();
    fixture_agent_summary_view(&mut snapshot.summary, normalizer);
    for value in [&mut snapshot.session, &mut snapshot.workspace] {
        *value = normalizer.normalize(value);
    }
    for value in [&mut snapshot.incompatibility, &mut snapshot.last_result]
        .into_iter()
        .flatten()
    {
        *value = normalizer.normalize(value);
    }
    snapshot
}

fn fixture_agent_view(
    report: &smith_client::agent_report::AgentReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::agent_report::AgentReport {
    use smith_client::agent_report::{AgentReport, AgentResumeReport};

    let mut report = report.clone();
    match &mut report {
        AgentReport::Empty | AgentReport::Unavailable | AgentReport::Parent => {}
        AgentReport::Missing(child) => *child = normalizer.normalize(child),
        AgentReport::List(children) => {
            for child in children {
                fixture_agent_summary_view(child, normalizer);
            }
        }
        AgentReport::Inspector(snapshot) => {
            *snapshot = fixture_agent_snapshot_view(snapshot, normalizer);
        }
        AgentReport::Resume(resume) => match resume {
            AgentResumeReport::RequiresIdle | AgentResumeReport::Unavailable => {}
            AgentResumeReport::Missing { child }
            | AgentResumeReport::Incompatible { child }
            | AgentResumeReport::Started { child } => *child = normalizer.normalize(child),
            AgentResumeReport::Failed { child, error } => {
                *child = normalizer.normalize(child);
                *error = normalizer.normalize(error);
            }
        },
    }
    report
}

fn fixture_mcp_view(
    report: &smith_client::mcp_report::McpReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::mcp_report::McpReport {
    use smith_client::mcp_report::{McpReport, McpServerState};

    let mut report = report.clone();
    match &mut report {
        McpReport::Empty { .. } | McpReport::Unavailable => {}
        McpReport::Error(error) => *error = normalizer.normalize(error),
        McpReport::Trusted { server, digest } => {
            *server = normalizer.normalize(server);
            *digest = normalizer.normalize(digest);
        }
        McpReport::Servers(servers) => {
            for server in servers {
                for value in [&mut server.name, &mut server.transport, &mut server.source] {
                    *value = normalizer.normalize(value);
                }
                if let McpServerState::Failed { reason } = &mut server.state {
                    *reason = normalizer.normalize(reason);
                }
                for rejected in &mut server.rejected {
                    *rejected = normalizer.normalize(rejected);
                }
                for value in &mut server.values {
                    value.name = normalizer.normalize(&value.name);
                    if let Some(credential) = &mut value.credential {
                        *credential = normalizer.normalize(credential);
                    }
                }
            }
        }
    }
    report
}

fn fixture_skills_view(
    report: &smith_client::skills_report::SkillsReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::skills_report::SkillsReport {
    use smith_client::skills_report::SkillsReport;

    let mut report = report.clone();
    match &mut report {
        SkillsReport::Empty => {}
        SkillsReport::Error(error) => *error = normalizer.normalize(error),
        SkillsReport::Trusted { skill, digest } => {
            *skill = normalizer.normalize(skill);
            *digest = normalizer.normalize(digest);
        }
        SkillsReport::Indexed { groups, problems } => {
            for group in groups {
                for entry in &mut group.entries {
                    entry.name = normalizer.normalize(&entry.name);
                    entry.description = normalizer.normalize(&entry.description);
                }
            }
            for problem in problems {
                for value in [&mut problem.name, &mut problem.reason, &mut problem.path] {
                    *value = normalizer.normalize(value);
                }
            }
        }
    }
    report
}

fn fixture_diff_view(
    report: &smith_client::diff_report::DiffReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::diff_report::DiffReport {
    use smith_client::diff_report::DiffOutcome;

    let mut report = report.clone();
    report.title = normalizer.normalize(&report.title);
    match &mut report.outcome {
        DiffOutcome::Empty => {}
        DiffOutcome::Error(message) => *message = normalizer.normalize(message),
        DiffOutcome::Patch(lines) => {
            for line in lines {
                line.text = normalizer.normalize(&line.text);
            }
        }
    }
    report
}

fn fixture_recovery_view(
    report: &smith_client::recovery_report::RecoveryReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::recovery_report::RecoveryReport {
    use smith_client::recovery_report::{RecoveryApplied, RecoveryReport};

    let mut report = report.clone();
    match &mut report {
        RecoveryReport::UndoConfirmation(preview) | RecoveryReport::RedoConfirmation(preview) => {
            for line in &mut preview.patch {
                line.text = normalizer.normalize(&line.text);
            }
        }
        RecoveryReport::RevertConfirmation(preview) => {
            preview.scope = normalizer.normalize(&preview.scope);
            preview.fingerprint = normalizer.normalize(&preview.fingerprint);
            for line in &mut preview.patch {
                line.text = normalizer.normalize(&line.text);
            }
        }
        RecoveryReport::PreviewError { message, .. }
        | RecoveryReport::ApplyError { message, .. } => *message = normalizer.normalize(message),
        RecoveryReport::Applied(RecoveryApplied::Revert { scope }) => {
            *scope = normalizer.normalize(scope);
        }
        RecoveryReport::RevertUsage
        | RecoveryReport::Applied(RecoveryApplied::Undo | RecoveryApplied::Redo)
        | RecoveryReport::Cancelled(_) => {}
    }
    report
}

fn fixture_review_view(
    report: &smith_client::review_report::ReviewReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::review_report::ReviewReport {
    use smith_client::review_report::{ReviewReport, ReviewStartReport};

    let mut report = report.clone();
    match &mut report {
        ReviewReport::Empty => {}
        ReviewReport::Error(message) => *message = normalizer.normalize(message),
        ReviewReport::Confirmation(preview) => {
            preview.scope = normalizer.normalize(&preview.scope);
            preview.title = normalizer.normalize(&preview.title);
            for line in &mut preview.patch {
                line.text = normalizer.normalize(&line.text);
            }
        }
        ReviewReport::Start(start) => match start {
            ReviewStartReport::Started { child } | ReviewStartReport::Queued { child } => {
                *child = normalizer.normalize(child);
            }
            ReviewStartReport::Failed(error) => *error = normalizer.normalize(error),
            ReviewStartReport::Unavailable | ReviewStartReport::AtCapacity { .. } => {}
        },
    }
    report
}

fn fixture_goal_view(
    report: &smith_client::goal_report::GoalReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::goal_report::GoalReport {
    use smith_client::goal_report::GoalReport;

    let mut report = report.clone();
    match &mut report {
        GoalReport::Empty | GoalReport::Cleared => {}
        GoalReport::Unavailable(error) => {
            *error = normalizer.normalize(error);
        }
        GoalReport::Snapshot(goal) => {
            for value in [
                &mut goal.objective,
                &mut goal.status,
                &mut goal.usage_provenance,
                &mut goal.active_elapsed,
                &mut goal.id,
            ] {
                *value = normalizer.normalize(value);
            }
            if let Some(reason) = &mut goal.stopped_reason {
                reason.code = normalizer.normalize(&reason.code);
                if let Some(detail) = &mut reason.detail {
                    *detail = normalizer.normalize(detail);
                }
            }
        }
    }
    report
}

fn fixture_timeline_view(
    report: &smith_client::timeline_report::TimelineReport,
    normalizer: &mut fixture_support::Normalizer,
) -> smith_client::timeline_report::TimelineReport {
    use smith_client::timeline_report::{TimelineChildEvent, TimelineEntry, TimelineReport};

    let mut report = report.clone();
    match &mut report {
        TimelineReport::Empty => {}
        TimelineReport::Unavailable(error) => {
            *error = normalizer.normalize(error);
        }
        TimelineReport::Entries(entries) => {
            for entry in entries {
                match entry {
                    TimelineEntry::RootTurn { turn, finish, .. } => {
                        *turn = normalizer.normalize(turn);
                        *finish = normalizer.normalize(finish);
                    }
                    TimelineEntry::RootManifest {
                        turn,
                        provider,
                        model,
                        ..
                    } => {
                        for value in [turn, provider, model] {
                            *value = normalizer.normalize(value);
                        }
                    }
                    TimelineEntry::ChildEvent { child, event } => {
                        *child = normalizer.normalize(child);
                        match event {
                            TimelineChildEvent::Started { workspace, .. } => {
                                *workspace = normalizer.normalize(workspace);
                            }
                            TimelineChildEvent::Stopped { reason } => {
                                *reason = normalizer.normalize(reason);
                            }
                            TimelineChildEvent::NeedsInput
                            | TimelineChildEvent::Completed
                            | TimelineChildEvent::Failed => {}
                        }
                    }
                    TimelineEntry::ChildSnapshot {
                        child,
                        session,
                        durability,
                        state,
                        turns,
                        ..
                    } => {
                        for value in [child, session, durability, state, turns] {
                            *value = normalizer.normalize(value);
                        }
                    }
                    TimelineEntry::Recovery { detail, .. } => {
                        *detail = normalizer.normalize(detail);
                    }
                }
            }
        }
    }
    report
}

fn fixture_screen(app: &App, width: u16) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 512))
        .expect("test terminal");
    terminal
        .draw(|frame| smith_tui::draw(frame, app, smith_tui::Theme::new().without_color()))
        .expect("frame");
    let buffer = terminal.backend().buffer();
    let rows = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>();
    format!("{}\n", rows.join("\n").trim_matches('\n'))
}

async fn fixture_command(
    name: &str,
    command: &str,
    mut app: App,
    host: &HostSession,
    project: &std::path::Path,
    mcp: Option<&crate::mcp::McpContext>,
    skills: &crate::skills::SkillContext,
) {
    let parsed = smith_client::commands::parse(command).expect("fixture command parses");
    app.composer.replace(command.to_owned());
    let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    match action {
        Some(Action::Command(actual)) => {
            assert_eq!(
                smith_client::commands::Command::Host(actual.clone()),
                parsed.command
            );
            Box::pin(handle_local_command(
                &mut app, host, project, mcp, skills, actual,
            ))
            .await;
        }
        None => {}
        other => panic!("fixture needs a local result, got {other:?}"),
    }
    let mut normalizer = fixture_support::Normalizer::new(
        host.paths()
            .expect("paths")
            .directory()
            .parent()
            .and_then(std::path::Path::parent)
            .and_then(std::path::Path::parent)
            .expect("isolated home"),
        project,
    );
    normalizer.session(host.session().id().as_str());
    normalizer.profile(&host.runtime().policy().agent_profile_revision);
    if let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
    {
        for (index, child) in coordinator.list().iter().enumerate() {
            normalizer.child_session(child.session.as_str(), index + 1);
        }
    }
    fixture_record(name, &app, &mut normalizer);
}

fn fixture_record(name: &str, app: &App, normalizer: &mut fixture_support::Normalizer) {
    let (raw, view) = fixture_raw_and_view(app, normalizer);
    fixture_support::compare_or_update(&format!("local-commands/{name}.raw.txt"), &raw);
    for width in [100, 44] {
        fixture_support::compare_or_update(
            &format!("local-commands/{name}.w{width}.txt"),
            &fixture_screen(&view, width),
        );
    }
}

// Each command group owns its home, project, and host. Box the host/command
// futures at their call sites so their large runtime state never accumulates
// inside a test future on the default Rust test-thread stack.
struct FixtureLocal {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
    host: HostSession,
    skills: crate::skills::SkillContext,
}

impl FixtureLocal {
    async fn command(&self, name: &str, command: &str, app: App) {
        Box::pin(fixture_command(
            name,
            command,
            app,
            &self.host,
            self.project.path(),
            None,
            &self.skills,
        ))
        .await;
    }

    async fn commands(&self, cases: &[(&str, &str)]) {
        for (name, command) in cases {
            Box::pin(self.command(name, command, fixture_local_app(false, false))).await;
        }
    }

    async fn replay(&self, command: &str) {
        let mut app = fixture_local_app(false, false);
        let smith_client::commands::Command::Host(command) = smith_client::commands::parse(command)
            .expect("replayed command")
            .command
        else {
            panic!("replay requires a host command");
        };
        Box::pin(handle_local_command(
            &mut app,
            &self.host,
            self.project.path(),
            None,
            &self.skills,
            command,
        ))
        .await;
    }

    // Ephemeral cases intentionally call the handler directly.
    // They have no persisted session paths for fixture_command to inspect.
    async fn direct_commands(&self, cases: &[(&str, &str)]) {
        for (name, command) in cases {
            let mut app = fixture_local_app(false, false);
            match smith_client::commands::parse(command)
                .expect("direct command")
                .command
            {
                smith_client::commands::Command::Host(command) => {
                    Box::pin(handle_local_command(
                        &mut app,
                        &self.host,
                        self.project.path(),
                        None,
                        &self.skills,
                        command,
                    ))
                    .await;
                }
                other => panic!("fixture requires a host command: {other:?}"),
            }
            let mut normalizer =
                fixture_support::Normalizer::new(self.home.path(), self.project.path());
            normalizer.session(self.host.session().id().as_str());
            normalizer.profile(&self.host.runtime().policy().agent_profile_revision);
            fixture_record(name, &app, &mut normalizer);
        }
    }

    async fn shutdown(self) {
        Box::pin(self.host.shutdown()).await.expect("shutdown");
    }
}

async fn fixture_local_base() -> FixtureLocal {
    use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, usage_event};
    use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
    let (home, project) = mcp_project("");
    let (sources, skills) = crate::skills::SkillContext::compose(
        smith_runtime::skills::SmithSkillSources::new(),
        &home.path().join(".smith"),
        project.path(),
    )
    .expect("skills");
    let host = Box::pin(fixture_local_host(
        home.path(),
        project.path(),
        Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "done".into(),
                    },
                    usage_event(9, 3),
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "child done".into(),
                    },
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
            ],
        )),
        sources,
    ))
    .await;
    FixtureLocal {
        home,
        project,
        host,
        skills,
    }
}

async fn fixture_local_populated() -> FixtureLocal {
    use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
    use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
    let (home, project) = mcp_project("");
    // The existing roots and skill helpers keep config and trust isolated.
    write_skill_body(&home.path().join(".smith"), "rust-review", SKILL_BODY);
    write_skill_body(&home.path().join(".smith"), "broken", "no frontmatter\n");
    write_skill_body(&project.path().join(".smith"), "deploy", SKILL_BODY);
    let (sources, skills) = crate::skills::SkillContext::compose(
        smith_runtime::built_in_skills::built_in_sources(),
        &home.path().join(".smith"),
        project.path(),
    )
    .expect("populated skills");
    std::fs::write(project.path().join(".smith/config.toml"), format!(
        "{LOCAL_COMMAND_CONFIG}\n[mcp.servers.docs]\ncommand = \"docs-mcp\"\nargs = [\"--stdio\"]\nenv = {{ DOCS_TOKEN = \"keychain:smith/docs\" }}\n[mcp.servers.off]\ncommand = \"off-mcp\"\nenabled = false\n"
    )).expect("MCP config");
    let mut edit = tool_call_fragments(
        0,
        "call-edit",
        "edit",
        r#"{"path":"tracked.txt","old_string":"before","new_string":"after"}"#,
    );
    edit.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(edit),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "edited".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    );
    let host = Box::pin(fixture_local_host(
        home.path(),
        project.path(),
        Arc::new(provider),
        sources,
    ))
    .await;
    FixtureLocal {
        home,
        project,
        host,
        skills,
    }
}

const FIXTURE_GOAL_COMMANDS: &[(&str, &str)] = &[
    ("goal-create", "/goal finish the fixture"),
    ("goal-active", "/goal"),
    ("goal-create-conflict", "/goal a conflicting objective"),
    ("goal-edit", "/goal edit revised fixture objective"),
    ("goal-budget", "/goal budget 100"),
    ("goal-budget-none", "/goal budget none"),
    ("goal-pause", "/goal pause"),
    ("goal-paused", "/goal"),
    ("goal-resume", "/goal resume"),
];
const FIXTURE_GOAL_FINISH_COMMANDS: &[(&str, &str)] =
    &[("goal-usage", "/goal"), ("goal-clear", "/goal clear")];

async fn fixture_local_goal_turn(fixture: &FixtureLocal, record: bool) {
    for (name, command) in FIXTURE_GOAL_COMMANDS {
        if record {
            Box::pin(fixture.command(name, command, fixture_local_app(false, false))).await;
        } else {
            Box::pin(fixture.replay(command)).await;
        }
    }
    Box::pin(fixture.host.session().run(UserInput::text("a fixed turn")))
        .await
        .expect("turn");
    for (name, command) in FIXTURE_GOAL_FINISH_COMMANDS {
        if record {
            Box::pin(fixture.command(name, command, fixture_local_app(false, false))).await;
        } else {
            Box::pin(fixture.replay(command)).await;
        }
    }
}

fn fixture_local_git_setup(fixture: &FixtureLocal) {
    git(
        fixture.project.path(),
        &[
            "init",
            "--initial-branch=fixture",
            "--template=",
            "--object-format=sha1",
        ],
    );
    for arguments in [
        ["config", "user.email", "smith@example.invalid"],
        ["config", "user.name", "Smith Test"],
        ["config", "commit.gpgsign", "false"],
        ["config", "core.autocrlf", "false"],
        ["config", "core.attributesFile", "/dev/null"],
        ["config", "diff.algorithm", "myers"],
        ["config", "diff.context", "3"],
        ["config", "diff.noprefix", "false"],
        ["config", "diff.mnemonicPrefix", "false"],
        ["config", "diff.indentHeuristic", "false"],
        ["config", "color.ui", "false"],
    ] {
        git(fixture.project.path(), &arguments);
    }
    std::fs::write(fixture.project.path().join(".gitignore"), ".smith/\n").expect("ignore state");
    std::fs::write(fixture.project.path().join("tracked.txt"), "before\n").expect("tracked");
    git(
        fixture.project.path(),
        &["add", ".gitignore", "tracked.txt"],
    );
    git(
        fixture.project.path(),
        &["-c", "core.hooksPath=/dev/null", "commit", "-m", "initial"],
    );
}

async fn fixture_local_git_edit(fixture: &FixtureLocal) {
    Box::pin(
        fixture
            .host
            .session()
            .run(UserInput::text("edit tracked.txt")),
    )
    .await
    .expect("scripted edit");
    assert_eq!(
        std::fs::read_to_string(fixture.project.path().join("tracked.txt")).expect("edited"),
        "after\n"
    );
    std::fs::write(fixture.project.path().join("untracked.txt"), "new file\n").expect("untracked");
}

const FIXTURE_GIT_DIRTY_COMMANDS: &[(&str, &str)] = &[
    ("status-dirty", "/status"),
    ("diff-dirty", "/diff"),
    ("diff-unstaged", "/diff unstaged"),
    ("diff-staged-empty", "/diff staged"),
    ("diff-untracked", "/diff untracked"),
    ("diff-file", "/diff tracked.txt"),
    ("diff-hunk", "/diff tracked.txt#1"),
    ("diff-last-turn", "/diff last-turn"),
    ("review-dirty", "/review"),
    ("review-file", "/review tracked.txt"),
    ("undo-preview", "/undo"),
    ("revert-file", "/revert tracked.txt"),
    ("revert-hunk", "/revert tracked.txt#1"),
    ("revert-untracked", "/revert untracked.txt"),
    ("revert-missing", "/revert missing.txt"),
];

#[derive(Clone, Copy)]
enum FixtureGitGroup {
    DiffReview,
    Recovery,
    Timeline,
}

// Replay every preview in the original order, but let each test record only its
// own group's paths. In particular, timeline-recovery includes both undo
// previews and the three successful revert previews preceding it.
async fn fixture_local_git_dirty(fixture: &FixtureLocal, group: FixtureGitGroup) {
    for (name, command) in FIXTURE_GIT_DIRTY_COMMANDS {
        let recovery = command.starts_with("/undo") || command.starts_with("/revert");
        let record = match group {
            FixtureGitGroup::DiffReview => !recovery,
            FixtureGitGroup::Recovery => recovery,
            FixtureGitGroup::Timeline => false,
        };
        if record {
            Box::pin(fixture.command(name, command, fixture_local_app(false, false))).await;
        } else {
            Box::pin(fixture.replay(command)).await;
        }
    }
}

#[tokio::test]
async fn fixtures_local_help() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[("help", "/help"), ("details-shown", "/details")])).await;
    let mut details = fixture_local_app(false, false);
    details.work_details = true;
    Box::pin(fixture.command("details-hidden", "/details", details)).await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_pickers() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("model-picker-empty", "/model"),
        ("model-missing", "/model missing"),
        ("provider-picker-empty", "/provider"),
        ("provider-missing", "/provider missing"),
        ("profile-picker-empty", "/profile"),
        ("profile-missing", "/profile missing"),
        ("resume-picker-empty", "/resume"),
        ("resume-missing", "/resume missing"),
        ("think-picker-empty", "/think"),
        ("think-unavailable", "/think on"),
        ("effort-picker-empty", "/effort"),
        ("effort-unavailable", "/effort high"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_status_diagnostics() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("status-fresh", "/status"),
        ("diagnostics-fresh", "/diagnostics"),
    ]))
    .await;
    for (name, command, reported) in [
        ("status-planned", "/status", false),
        ("diagnostics-planned", "/diagnostics", false),
        ("status-usage", "/status", true),
        ("diagnostics-usage", "/diagnostics", true),
    ] {
        Box::pin(fixture.command(name, command, fixture_local_app(true, reported))).await;
    }
    let mut priced = fixture_local_app(true, true);
    priced.status.set_price(Some(PriceReference {
        provider: "local".into(),
        model: "example-model".into(),
        table: PriceTable {
            input: Some(2_000_000),
            output: Some(8_000_000),
            cache_read: Some(200_000),
            cache_write: None,
        },
    }));
    Box::pin(fixture.command("status-priced", "/status", priced)).await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_context() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("context-fresh", "/context"),
        ("context-argument", "/context 256k"),
    ]))
    .await;
    for (name, reported) in [("context-planned", false), ("context-usage", true)] {
        Box::pin(fixture.command(name, "/context", fixture_local_app(true, reported))).await;
    }
    for (name, compacted) in [("context-exact", false), ("context-compacted", true)] {
        let mut app = fixture_local_app(true, false);
        let plan = app.status.context_plan.as_mut().expect("context plan");
        plan.confidence = EstimationConfidence::Exact;
        if compacted {
            plan.totals.insert("history".into(), 1_000);
            plan.totals.insert("summary".into(), 300);
            plan.segment_count = 4;
        }
        Box::pin(fixture.command(name, "/context", app)).await;
    }
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_goal() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("goal-empty", "/goal"),
        ("goal-edit-empty", "/goal edit revised objective"),
        ("goal-budget-empty", "/goal budget 100"),
        ("goal-pause-empty", "/goal pause"),
        ("goal-resume-empty", "/goal resume"),
        ("goal-clear-empty", "/goal clear"),
    ]))
    .await;
    Box::pin(fixture_local_goal_turn(&fixture, true)).await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_timeline() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[("timeline-empty", "/timeline")])).await;
    // Keep the original goal event history and usage-bearing turn: it is part
    // of the timeline-turn recording, including any journal-read error.
    Box::pin(fixture_local_goal_turn(&fixture, false)).await;
    Box::pin(fixture.commands(&[("timeline-turn", "/timeline")])).await;
    Box::pin(fixture.shutdown()).await;

    let fixture = Box::pin(fixture_local_populated()).await;
    fixture_local_git_setup(&fixture);
    Box::pin(fixture_local_git_edit(&fixture)).await;
    Box::pin(fixture_local_git_dirty(&fixture, FixtureGitGroup::Timeline)).await;
    Box::pin(fixture.commands(&[("timeline-recovery", "/timeline")])).await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_agent() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("agent-empty", "/agent"),
        ("agent-next-empty", "/agent next"),
        ("agent-previous-empty", "/agent previous"),
        ("agent-parent", "/agent parent"),
        ("agent-missing", "/agent missing"),
        ("agent-resume-missing", "/agent resume missing"),
    ]))
    .await;
    // Consume the root stream before spawning, so the child still receives
    // the original "child done" stream with the same counters and turn IDs.
    Box::pin(fixture_local_goal_turn(&fixture, false)).await;
    let coordinator = fixture
        .host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .expect("delegation");
    let spawned = Box::pin(coordinator.spawn(ChildSpec {
        task: UserInput::text("inspect the fixture"),
        model: ChildModelSelection::Inherit,
        limits: ChildLimits::turns(1),
        tools: ToolViewScope::ReadOnly,
        workspace: WorkspacePolicy::ReadOnlyView,
    }))
    .await
    .expect("spawn child");
    let agent_runtime::delegation::SpawnOutcome::Spawned { child, .. } = spawned else {
        panic!("fixture child was not spawned");
    };
    let child_status =
        tokio::time::timeout(Duration::from_secs(60), Box::pin(coordinator.wait(&child)))
            .await
            .expect("child watchdog")
            .expect("completed child");
    for (name, command) in [
        ("agent-populated", "/agent".to_owned()),
        ("agent-inspector", format!("/agent {child}")),
        ("agent-next", "/agent next".to_owned()),
        ("agent-previous", "/agent previous".to_owned()),
    ] {
        let mut app = fixture_local_app(false, false);
        app.children.insert(
            child.as_str().to_owned(),
            smith_tui::app::ChildSummary {
                state: "completed".into(),
                detail: child_status.last_result.clone(),
                profile: None,
            },
        );
        Box::pin(fixture.command(name, &command, app)).await;
    }
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_mcp_skills() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("mcp-empty", "/mcp"),
        ("mcp-trust-empty", "/mcp trust missing"),
        ("skills-empty", "/skills"),
        ("skills-trust-missing", "/skills trust missing"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;

    let fixture = Box::pin(fixture_local_populated()).await;
    let mcp = mcp_context(fixture.home.path(), fixture.project.path());
    for (name, command) in [
        ("mcp-declared", "/mcp"),
        ("mcp-trust", "/mcp trust docs"),
        ("mcp-trust-missing", "/mcp trust missing"),
        ("skills-populated", "/skills"),
        ("skills-trust", "/skills trust deploy"),
    ] {
        Box::pin(fixture_command(
            name,
            command,
            fixture_local_app(false, false),
            &fixture.host,
            fixture.project.path(),
            Some(&mcp),
            &fixture.skills,
        ))
        .await;
    }
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_accounts_connections() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("account-picker-empty", "/account"),
        ("account-invalid", "/account nope"),
        ("account-missing", "/account 2"),
        ("connect-picker-empty", "/connect"),
        ("connect-missing", "/connect missing"),
        ("disconnect-picker-empty", "/disconnect"),
        ("disconnect-missing", "/disconnect missing"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;

    let fixture = Box::pin(fixture_local_populated()).await;
    let mut accounts = fixture_local_app(false, false);
    accounts.set_accounts(vec![
        smith_tui::picker::ResourceEntry::new("0", "1 · active", "env:FIRST · 25% used")
            .active(true),
        smith_tui::picker::ResourceEntry::new("1", "2 · available", "env:SECOND · 75% used"),
    ]);
    Box::pin(fixture.command("account-picker", "/account", accounts)).await;
    let mut active = fixture_local_app(false, false);
    active.set_accounts(vec![
        smith_tui::picker::ResourceEntry::new("0", "1", "env:FIRST").active(true),
    ]);
    Box::pin(fixture.command("account-active", "/account 1", active)).await;
    for (name, command, connected) in [
        ("connect-picker", "/connect", false),
        ("disconnect-picker", "/disconnect", true),
    ] {
        let mut app = fixture_local_app(false, false);
        let entry =
            smith_tui::picker::ResourceEntry::new("local", "Local provider", "configured fixture");
        if connected {
            app.resources.disconnections.push(entry);
        } else {
            app.resources.connections.push(entry);
        }
        Box::pin(fixture.command(name, command, app)).await;
    }
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_diff_review() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("diff-no-git", "/diff"),
        ("diff-last-turn-empty", "/diff last-turn"),
        ("review-no-git", "/review"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;

    let fixture = Box::pin(fixture_local_populated()).await;
    fixture_local_git_setup(&fixture);
    Box::pin(fixture.commands(&[("diff-clean", "/diff"), ("review-clean", "/review")])).await;
    Box::pin(fixture_local_git_edit(&fixture)).await;
    Box::pin(fixture_local_git_dirty(
        &fixture,
        FixtureGitGroup::DiffReview,
    ))
    .await;
    fixture
        .host
        .changes()
        .undo_latest()
        .expect("undo exact edit");
    Box::pin(fixture.replay("/undo")).await;
    Box::pin(fixture.replay("/redo")).await;
    fixture
        .host
        .changes()
        .redo_latest()
        .expect("redo exact edit");
    git(fixture.project.path(), &["add", "tracked.txt"]);
    Box::pin(fixture.commands(&[("diff-staged", "/diff staged")])).await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_undo_redo_revert() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("undo-empty", "/undo"),
        ("redo-empty", "/redo"),
        ("revert-no-scope", "/revert"),
        ("revert-no-git", "/revert tracked.txt"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;

    let fixture = Box::pin(fixture_local_populated()).await;
    fixture_local_git_setup(&fixture);
    Box::pin(fixture_local_git_edit(&fixture)).await;
    Box::pin(fixture_local_git_dirty(&fixture, FixtureGitGroup::Recovery)).await;
    fixture
        .host
        .changes()
        .undo_latest()
        .expect("undo exact edit");
    Box::pin(fixture.commands(&[("undo-already-undone", "/undo"), ("redo-preview", "/redo")]))
        .await;
    fixture
        .host
        .changes()
        .redo_latest()
        .expect("redo exact edit");
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_ephemeral() {
    use agent_runtime::provider::fake::FakeProvider;
    let (home, project) = mcp_project("");
    // The host rejects persistence policy from either project config layer.
    // ResolveRequest::with_home_dir reads this isolated user's .smith/config.toml.
    std::fs::write(
        home.path().join(".smith/config.toml"),
        "[persistence]\nenabled = false\n",
    )
    .expect("ephemeral user config");
    let ephemeral_config = LOCAL_COMMAND_CONFIG.replace(
        "model = \"example-model\"\n",
        "model = \"example-model\"\ndelegation = false\n",
    );
    std::fs::write(project.path().join(".smith/config.toml"), ephemeral_config)
        .expect("ephemeral project config");
    let (sources, skills) = crate::skills::SkillContext::compose(
        smith_runtime::skills::SmithSkillSources::new(),
        &home.path().join(".smith"),
        project.path(),
    )
    .expect("ephemeral skills");
    let host = Box::pin(fixture_local_host(
        home.path(),
        project.path(),
        Arc::new(FakeProvider::text_reply("unused")),
        sources,
    ))
    .await;
    assert!(host.paths().is_none(), "fixture host must be ephemeral");
    assert!(
        !host.runtime().policy().agent_delegation,
        "fixture delegation must be disabled"
    );
    let fixture = FixtureLocal {
        home,
        project,
        host,
        skills,
    };
    Box::pin(fixture.direct_commands(&[
        ("agent-unavailable", "/agent"),
        ("goal-ephemeral", "/goal"),
        ("goal-create-ephemeral", "/goal an unavailable goal"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;
}
