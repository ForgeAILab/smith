//! Product-level inline evidence policy, shared by root and child runtimes.
//!
//! Storage/capture bounds and model working-set bounds are deliberately separate.
//! This is output admission, not semantic history compaction: many small results
//! can still fill a window. The canonical planner remains the final budget owner.

use std::sync::Arc;

use agent_runtime::harness::{
    ArtifactOffloader, ArtifactReadTool, ComponentDescriptor, ToolOutputPatch, ToolOutputProcessor,
    ToolOutputView,
};
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::artifact::ArtifactStore;
use agent_runtime_core::content::ContentPart;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolContent, ToolOutcome,
    ToolSpec,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use smith_config::resolve::ResolvedConfig;

/// Resolved byte bounds. These are not tokenizer-exact or whole-request limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolOutputContextPolicy {
    /// Serialized outcome threshold for recoverable text offloading.
    pub inline_bytes: u32,
    /// Maximum raw bytes requested from the artifact reader per invocation.
    pub artifact_page_bytes: u32,
}

impl ToolOutputContextPolicy {
    /// Derives the policy without increasing an existing stricter output bound.
    pub fn from_config(config: &ResolvedConfig) -> Self {
        let inline_bytes = u64::from(config.context.tool_output_inline_bytes.value)
            .min(config.limits.tool_output_limit_bytes.value) as u32;
        Self {
            inline_bytes,
            // Leave room for JSON escaping, metadata and non-UTF8 byte arrays.
            // The final planner still sizes the actual rendered request.
            artifact_page_bytes: (inline_bytes / 4).clamp(1, 4096),
        }
    }

    /// Reuses Runtime's durable, idempotent offloader with a small text preview.
    pub fn offloader(
        self,
        store: Arc<dyn ArtifactStore>,
    ) -> Result<ContextOffloader, RuntimeError> {
        let preview_chars = (self.inline_bytes as usize / 4).clamp(64, 1200);
        Ok(ContextOffloader {
            inner: ArtifactOffloader::new(store)
                .with_threshold_bytes(self.inline_bytes as usize)?
                .with_preview_chars(preview_chars)?,
            policy: self,
            preview_chars,
        })
    }

    /// Uses the same policy for bounded expansion, including child routes.
    pub fn reader(self, store: Arc<dyn ArtifactStore>) -> ContextArtifactReader {
        ContextArtifactReader {
            inner: ArtifactReadTool::new(store),
            page_bytes: self.artifact_page_bytes,
        }
    }
}

/// Text-only product preview over the shared archive-before-replace mechanism.
#[derive(Debug)]
pub struct ContextOffloader {
    inner: ArtifactOffloader,
    policy: ToolOutputContextPolicy,
    preview_chars: usize,
}

#[async_trait]
impl ToolOutputProcessor for ContextOffloader {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(
            "harness.artifact.offload",
            RegistryRevision::new(format!(
                "smith-inline-output-v1-{}",
                self.policy.inline_bytes
            )),
        )
    }

    async fn process(
        &self,
        view: &ToolOutputView,
        outcome: ToolOutcome,
    ) -> Result<ToolOutputPatch, RuntimeError> {
        if view.call.name == "artifact.read" {
            return Ok(ToolOutputPatch::outcome(outcome));
        }
        let Some(parts) = outcome.content.as_inline() else {
            return Ok(ToolOutputPatch::outcome(outcome));
        };
        // Do not replace images, audio, opaque reasoning or other typed content
        // with a text thumbnail merely because its serialization is large.
        if parts
            .iter()
            .any(|part| !matches!(part, ContentPart::Text { .. }))
        {
            return Ok(ToolOutputPatch::outcome(outcome));
        }
        let preview = text_preview(&outcome, self.preview_chars);
        // Runtime persists the *unchanged* outcome before returning a reference.
        // Storage errors remain errors; never fabricate a retrieval pointer.
        let mut patch = self.inner.process(view, outcome).await?;
        if let ToolContent::Artifact { preview: parts, .. } = &mut patch.outcome.content {
            *parts = vec![ContentPart::text(preview)];
        }
        Ok(patch)
    }
}

/// Artifact reader with bounded defaults and oversized-request clamping.
/// Session ownership, preparation and integrity checks remain Runtime-owned.
#[derive(Debug)]
pub struct ContextArtifactReader {
    inner: ArtifactReadTool,
    page_bytes: u32,
}

#[async_trait]
impl Tool for ContextArtifactReader {
    fn spec(&self) -> ToolSpec {
        let mut spec = self.inner.spec();
        // Keep Runtime's accepted range: schema validation precedes prepare,
        // so advertising the effective cap as `maximum` would reject requests
        // we intend to clamp before preparation can normalize them.
        spec.input_schema["properties"]["limit"]["default"] = json!(self.page_bytes);
        spec.description.push_str(&format!(
            " Returns at most {} raw bytes per page; larger valid limits are clamped during preparation. Follow next_offset for more; retrieve only the evidence needed.",
            self.page_bytes,
        ));
        spec
    }

    async fn prepare(
        &self,
        mut arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        if let Some(object) = arguments.as_object_mut() {
            match object.get("limit") {
                None => {
                    object.insert("limit".into(), json!(self.page_bytes));
                }
                Some(value)
                    if value
                        .as_u64()
                        .is_some_and(|limit| limit > u64::from(self.page_bytes)) =>
                {
                    object.insert("limit".into(), json!(self.page_bytes));
                }
                // Invalid types and zero still receive Runtime's precise error.
                _ => {}
            }
        }
        self.inner.prepare(arguments, ctx).await
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        // A stale or manually constructed prepared call cannot bypass the cap.
        if !prepared
            .arguments()
            .get("limit")
            .and_then(Value::as_u64)
            .is_some_and(|limit| (1..=u64::from(self.page_bytes)).contains(&limit))
        {
            return Err(RuntimeError::tool(
                "artifact.read must be prepared with the current page bound",
            ));
        }
        self.inner.invoke(prepared, ctx).await
    }
}

fn prefix(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn head_tail(text: &str, limit: usize) -> String {
    const OMIT: &str = "\n[... excerpt omitted; read artifact for exact output ...]\n";
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let available = limit.saturating_sub(OMIT.chars().count());
    let tail: String = text
        .chars()
        .rev()
        .take(available / 2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{}{OMIT}{tail}", prefix(text, available.div_ceil(2)))
}

fn text_preview(outcome: &ToolOutcome, limit: usize) -> String {
    let mut sections = vec![format!(
        "Captured tool output (excerpt, not a semantic summary); is_error={}",
        outcome.is_error
    )];
    // Exact factual metadata, not conclusions inferred from matching a log line.
    let fields = [
        "exit_code",
        "success",
        "timed_out",
        "truncated",
        "running",
        "task_id",
        "path",
        "lines",
        "shown",
        "matches",
    ];
    let metadata = fields
        .iter()
        .filter_map(|key| {
            let value = outcome.value.get(*key)?;
            Some(format!("{key}={}", prefix(&value.to_string(), 96)))
        })
        .collect::<Vec<_>>()
        .join(" · ");
    sections.push(prefix(&metadata, limit / 4));
    let text = outcome
        .content
        .as_inline()
        .unwrap_or_default()
        .iter()
        .filter_map(ContentPart::as_text)
        .collect::<Vec<_>>()
        .join("\n");
    let text = if text.is_empty() {
        outcome.value.to_string()
    } else {
        text
    };
    // These are explicitly heuristic excerpts. A hit is never evidence of the
    // command's success/failure, nor a claim that every failure was selected.
    let diagnostics = text
        .lines()
        .filter(|line| {
            let sample = prefix(line, 512).to_ascii_lowercase();
            [
                "error",
                "failed",
                "panic",
                "exception",
                "assertion",
                "traceback",
            ]
            .iter()
            .any(|needle| sample.contains(needle))
        })
        .take(3)
        .map(|line| prefix(line, 120))
        .collect::<Vec<_>>();
    if !diagnostics.is_empty() {
        sections.push(format!(
            "Selected diagnostic lines (heuristic):\n{}",
            diagnostics.join("\n")
        ));
    }
    let used = sections
        .iter()
        .map(|section| section.chars().count() + 1)
        .sum::<usize>();
    sections.push(head_tail(&text, limit.saturating_sub(used)));
    prefix(&sections.join("\n"), limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::SmithArtifactStore;
    use crate::session::{ProjectId, SessionPaths};
    use agent_runtime_core::artifact::{
        ArtifactChunk, ArtifactError, ArtifactRead, ArtifactRef, ArtifactWrite,
    };
    use agent_runtime_core::cancel::Cancellation;
    use agent_runtime_core::clock::{Deadline, SystemClock, Timestamp};
    use agent_runtime_core::content::ToolCall;
    use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId, TurnId};
    use agent_runtime_testkit::MemoryWorkspace;

    fn policy() -> ToolOutputContextPolicy {
        ToolOutputContextPolicy {
            inline_bytes: 8192,
            artifact_page_bytes: 2048,
        }
    }
    fn view() -> ToolOutputView {
        ToolOutputView {
            session: SessionId::new("owner"),
            turn: TurnId::new("turn"),
            request: RequestId::new("request"),
            call: ToolCall {
                id: ToolCallId::new("call"),
                name: "shell".into(),
                arguments: json!({}),
            },
            state: None,
            usage: Arc::from([]),
            now: Timestamp::ZERO,
        }
    }
    fn store(root: &std::path::Path) -> Arc<SmithArtifactStore> {
        Arc::new(SmithArtifactStore::new(SessionPaths::new(
            root,
            &ProjectId::new("test").unwrap(),
        )))
    }
    fn preparation() -> PreparationContext {
        PreparationContext {
            session: SessionId::new("owner"),
            turn: None,
            call_id: ToolCallId::new("read"),
            request: RequestId::new("request"),
            workspace: Arc::new(MemoryWorkspace::new("/repo")),
            clock: Arc::new(SystemClock),
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
        }
    }
    fn invocation(prep: &PreparationContext) -> InvocationContext {
        InvocationContext {
            session: prep.session.clone(),
            turn: prep.turn.clone(),
            call_id: prep.call_id.clone(),
            request: prep.request.clone(),
            workspace: prep.workspace.clone(),
            clock: prep.clock.clone(),
            cancel: prep.cancel.clone(),
            deadline: prep.deadline,
            output_limit: 65536,
        }
    }
    async fn bytes(store: &dyn ArtifactStore, reference: &ArtifactRef) -> Vec<u8> {
        let mut result = Vec::new();
        let mut offset = 0;
        loop {
            let page = store
                .read(ArtifactRead {
                    session: reference.provenance.session.clone(),
                    id: reference.id.clone(),
                    offset,
                    limit: 2048,
                })
                .await
                .unwrap();
            result.extend(page.bytes);
            match page.next_offset {
                Some(next) => {
                    assert!(next > offset);
                    offset = next;
                }
                None => return result,
            }
        }
    }
    #[tokio::test]
    async fn small_text_is_unchanged_and_does_not_create_storage() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let original = ToolOutcome::text("small result");
        let patch = policy()
            .offloader(store.clone())
            .unwrap()
            .process(&view(), original.clone())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(patch.outcome).unwrap(),
            serde_json::to_value(original).unwrap()
        );
        assert!(!store.directory().exists());
    }
    #[tokio::test]
    async fn medium_failure_has_bounded_diagnostics_and_exact_idempotent_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let original = ToolOutcome {
            value: json!({"exit_code": 1, "truncated": false}),
            content: ToolContent::inline(vec![ContentPart::text(format!(
                "BEGIN\n{}\nerror: MIDDLE_FAILURE\n{}\nEND",
                "passing test\n".repeat(600),
                "passing test\n".repeat(600)
            ))]),
            is_error: true,
        };
        let encoded = serde_json::to_vec(&original).unwrap();
        assert!((8193..65536).contains(&encoded.len()));
        let offloader = policy().offloader(store.clone()).unwrap();
        let first = offloader.process(&view(), original.clone()).await.unwrap();
        let second = offloader.process(&view(), original).await.unwrap();
        assert!(first.outcome.is_error);
        let reference = first.outcome.content.artifact_reference().unwrap();
        assert_eq!(Some(reference), second.outcome.content.artifact_reference());
        assert_eq!(bytes(store.as_ref(), reference).await, encoded);
        let ToolContent::Artifact { preview, .. } = &first.outcome.content else {
            panic!("artifact")
        };
        let text = preview[0].as_text().unwrap();
        assert!(text.chars().count() <= 1200);
        for expected in [
            "is_error=true",
            "exit_code=1",
            "MIDDLE_FAILURE",
            "heuristic",
            "BEGIN",
            "END",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        assert!(!text.contains("\"type\":\"text\""));
        assert_eq!(bytes(store.as_ref(), reference).await, encoded);
    }
    #[tokio::test]
    async fn unicode_preview_is_bounded_and_multimodal_content_is_never_text_offloaded() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let offloader = policy().offloader(store.clone()).unwrap();
        let unicode = ToolOutcome::text("测试🦀".repeat(8000));
        let patch = offloader.process(&view(), unicode).await.unwrap();
        let ToolContent::Artifact { preview, .. } = patch.outcome.content else {
            panic!("artifact")
        };
        assert!(preview[0].as_text().unwrap().chars().count() <= 1200);
        let image = ToolOutcome {
            value: json!({}),
            content: ToolContent::inline(vec![
                ContentPart::Image {
                    url: format!("data:image/png;base64,{}", "A".repeat(9000)),
                    detail: None,
                },
                ContentPart::text("keep image and caption together"),
            ]),
            is_error: false,
        };
        let result = offloader.process(&view(), image.clone()).await.unwrap();
        assert_eq!(
            serde_json::to_value(result.outcome).unwrap(),
            serde_json::to_value(image).unwrap()
        );
    }
    #[tokio::test]
    async fn reader_clamps_pages_and_reopened_store_still_enforces_session_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let saved = policy()
            .offloader(store(dir.path()))
            .unwrap()
            .process(&view(), ToolOutcome::text("evidence\n".repeat(3000)))
            .await
            .unwrap();
        let reference = saved.outcome.content.artifact_reference().unwrap().clone();
        let reopened = store(dir.path());
        let reader = policy().reader(reopened.clone());
        assert_eq!(
            reader.spec().input_schema["properties"]["limit"]["maximum"],
            agent_runtime_core::artifact::MAX_ARTIFACT_READ_BYTES
        );
        assert_eq!(
            reader.spec().input_schema["properties"]["limit"]["default"],
            2048
        );
        assert!(reader.spec().description.contains("at most 2048 raw bytes"));
        let prep = preparation();
        for limit in [None, Some(65536)] {
            let mut args = json!({"id": reference.id.as_str()});
            if let Some(limit) = limit {
                args["limit"] = json!(limit);
            }
            let prepared = reader.prepare(args, &prep).await.unwrap();
            assert_eq!(prepared.arguments()["limit"], 2048);
            let page = reader
                .invoke(prepared.clone(), &invocation(&prep))
                .await
                .unwrap();
            assert_eq!(page.value["next_offset"], 2048);
            assert!(page.value["content"].as_str().unwrap().len() <= 2048);
            let mut wrong = invocation(&prep);
            wrong.session = SessionId::new("other");
            assert!(reader.invoke(prepared, &wrong).await.is_err());
        }
        for invalid in [json!(0), json!(-1), json!("2048"), json!(null), json!(1.5)] {
            assert!(
                reader
                    .prepare(
                        json!({"id": reference.id.as_str(), "limit": invalid}),
                        &prep
                    )
                    .await
                    .is_err()
            );
        }
        // Pages are not recursively wrapped into an endless chain of artifacts.
        let mut read_view = view();
        read_view.call.name = "artifact.read".into();
        let raw = ToolOutcome::text("x".repeat(9000));
        let result = policy()
            .offloader(reopened)
            .unwrap()
            .process(&read_view, raw.clone())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(result.outcome).unwrap(),
            serde_json::to_value(raw).unwrap()
        );
    }
    #[derive(Debug)]
    struct FailedStore;
    #[async_trait]
    impl ArtifactStore for FailedStore {
        async fn put(&self, _: ArtifactWrite) -> Result<ArtifactRef, ArtifactError> {
            Err(ArtifactError::Unavailable {
                detail: "fixture".into(),
            })
        }
        async fn read(&self, _: ArtifactRead) -> Result<ArtifactChunk, ArtifactError> {
            Err(ArtifactError::NotFound)
        }
    }
    #[tokio::test]
    async fn failed_archive_never_returns_a_fabricated_reference() {
        let offloader = policy().offloader(Arc::new(FailedStore)).unwrap();
        assert!(
            offloader
                .process(&view(), ToolOutcome::text("large".repeat(3000)))
                .await
                .is_err()
        );
    }
}
