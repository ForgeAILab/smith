//! Product-level inline evidence policy, shared by root and child runtimes.
//!
//! Storage/capture bounds and model working-set bounds are deliberately separate.
//! This is output admission, not semantic history compaction: many small results
//! can still fill a window. The canonical planner remains the final budget owner.

use std::sync::Arc;

use agent_runtime::harness::ARTIFACT_READ_PERMISSION;
use agent_runtime::harness::{
    ArtifactOffloader, ArtifactReadTool, ComponentDescriptor, ToolOutputPatch, ToolOutputProcessor,
    ToolOutputView,
};
use agent_runtime::registry::Permission;
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::artifact::{ArtifactRef, ArtifactStore};
use agent_runtime_core::content::ContentPart;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::{ChildId, SessionId};
use agent_runtime_core::security::{PermissionSet, SecurityResource};
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolCallDisplay, ToolContent,
    ToolEffects, ToolOutcome, ToolSpec,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use smith_config::resolve::ResolvedConfig;

use crate::artifact::ArtifactCatalog;
use crate::delegation::AGENT_TOOL_NAME;

/// Dedicated retrieval affordance: a broad `read` query can rank the file reader first.
pub const ARTIFACT_DISCOVERY_QUERY: &str = "artifact-read";

/// The session's artifact inventory, beside Runtime's `artifact.read`.
pub const ARTIFACT_LIST_TOOL_NAME: &str = "artifact.list";

/// Retrieval affordance for the lister, disjoint from the reader's.
pub const ARTIFACT_LIST_DISCOVERY_QUERY: &str = "artifact-list";

/// Runtime's exact message for an id no stored artifact has.
const ARTIFACT_NOT_FOUND: &str = "artifact not found";

const DEFAULT_LIST_ROWS: usize = 20;
const MAX_LIST_ROWS: usize = 100;
const MAX_PURPOSE_CHARS: usize = 128;

/// Resolved byte bounds. These are not tokenizer-exact or whole-request limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolOutputContextPolicy {
    /// Serialized outcome threshold for recoverable text offloading.
    pub inline_bytes: u32,
    /// Maximum raw bytes requested from the artifact reader per invocation.
    pub artifact_page_bytes: u32,
    /// Serialized bound up to which a completed child's reply stays inline,
    /// in an `agent` outcome or an automatic delivery: the per-call ceiling.
    pub reply_bytes: u32,
}

impl ToolOutputContextPolicy {
    /// Derives the policy without increasing an existing stricter output bound.
    pub fn from_config(config: &ResolvedConfig) -> Self {
        let limit = config.limits.tool_output_limit_bytes.value;
        let inline_bytes =
            u64::from(config.context.tool_output_inline_bytes.value).min(limit) as u32;
        Self {
            inline_bytes,
            // Leave room for JSON escaping, metadata and non-UTF8 byte arrays.
            // The final planner still sizes the actual rendered request.
            artifact_page_bytes: (inline_bytes / 4).clamp(1, 4096),
            reply_bytes: limit.min(u64::from(u32::MAX)) as u32,
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
            lister_registered: false,
        }
    }

    /// Lists the session's artifacts in pages that fit the inline allowance.
    pub fn lister(
        self,
        catalog: Arc<dyn ArtifactCatalog>,
        children: Option<Arc<dyn ChildDirectory>>,
    ) -> ContextArtifactLister {
        ContextArtifactLister {
            catalog,
            children,
            // Leave room for the page envelope around the rows.
            page_bytes: (self.inline_bytes as usize).saturating_sub(256).max(512),
        }
    }

    /// The reader and, when `catalog` is present, the lister it may name.
    /// Both come from here so a reader never names a lister that is absent.
    /// `children` resolves copied child artifacts to child ids; only a
    /// runtime that delegates has one.
    pub fn artifact_tools(
        self,
        store: Arc<dyn ArtifactStore>,
        catalog: Option<Arc<dyn ArtifactCatalog>>,
        children: Option<Arc<dyn ChildDirectory>>,
    ) -> Vec<Arc<dyn Tool>> {
        let mut reader = self.reader(store);
        let Some(catalog) = catalog else {
            return vec![Arc::new(reader)];
        };
        reader.lister_registered = true;
        vec![Arc::new(reader), Arc::new(self.lister(catalog, children))]
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
                "smith-inline-output-v2-{}-{}",
                self.policy.inline_bytes, self.policy.reply_bytes
            )),
        )
    }

    async fn process(
        &self,
        view: &ToolOutputView,
        outcome: ToolOutcome,
    ) -> Result<ToolOutputPatch, RuntimeError> {
        // Both already bound their own pages; offloading either would answer
        // a lookup with yet another reference.
        if view.call.name == "artifact.read" || view.call.name == ARTIFACT_LIST_TOOL_NAME {
            return Ok(ToolOutputPatch::outcome(outcome));
        }
        // A child's reply is the report the parent asked for. Offloaded, it
        // comes back one page per request, each resending the conversation.
        if view.call.name == AGENT_TOOL_NAME
            && carries_child_reply(&outcome.value)
            && serde_json::to_vec(&outcome)
                .is_ok_and(|encoded| encoded.len() <= self.policy.reply_bytes as usize)
        {
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

/// Whether an `agent` outcome hands over a completed child's reply: `result`
/// nests it under `text`, and `wait` on an idle child carries it bare.
fn carries_child_reply(value: &Value) -> bool {
    match value.get("result") {
        Some(Value::String(_)) => true,
        Some(Value::Object(result)) => result.get("text").is_some_and(Value::is_string),
        _ => false,
    }
}

/// Artifact reader with bounded defaults and oversized-request clamping.
/// Session ownership, preparation and integrity checks remain Runtime-owned.
#[derive(Debug)]
pub struct ContextArtifactReader {
    inner: ArtifactReadTool,
    page_bytes: u32,
    /// Whether the same tool set registers [`ContextArtifactLister`], which
    /// the not-found error may then name.
    lister_registered: bool,
}

impl ContextArtifactReader {
    /// Runtime's bare "artifact not found" gives a model that invented an id
    /// nothing to do but guess again; say where real ids come from instead.
    fn explain_missing(&self, error: RuntimeError) -> RuntimeError {
        if error.message != ARTIFACT_NOT_FOUND {
            return error;
        }
        let mut message = format!(
            "{ARTIFACT_NOT_FOUND}: no artifact with this id exists in this session. Artifact \
             ids are copied exactly from tool results or host messages, never constructed."
        );
        if self.lister_registered {
            message.push_str(&format!(
                " Call {ARTIFACT_LIST_TOOL_NAME} to see this session's artifacts (find it with \
                 registry.search, query: {ARTIFACT_LIST_DISCOVERY_QUERY}, if absent)."
            ));
        }
        RuntimeError { message, ..error }
    }
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
        self.inner
            .invoke(prepared, ctx)
            .await
            .map(text_page)
            .map_err(|error| self.explain_missing(error))
    }
}

/// Runtime pages artifacts by raw bytes and returns any page that is not
/// valid UTF-8 as an array of byte values, which is every text page whose edge
/// splits a multi-byte character. Trimming the partial characters keeps a text
/// artifact readable as text; a page that is invalid inside stays bytes.
fn text_page(mut outcome: ToolOutcome) -> ToolOutcome {
    if outcome.value.get("encoding").and_then(Value::as_str) != Some("bytes") {
        return outcome;
    }
    let (Some(offset), Some(values)) = (
        outcome.value.get("offset").and_then(Value::as_u64),
        outcome.value.get("content").and_then(Value::as_array),
    ) else {
        return outcome;
    };
    let Some(bytes) = values
        .iter()
        .map(|value| value.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect::<Option<Vec<u8>>>()
    else {
        return outcome;
    };
    // An offset inside a character starts with at most three continuation bytes.
    let lead = bytes
        .iter()
        .take(3)
        .take_while(|byte| (0x80..0xC0).contains(*byte))
        .count();
    let rest = &bytes[lead..];
    let text = match std::str::from_utf8(rest) {
        Ok(text) => text,
        // No error length: the bytes end partway through one character.
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&rest[..error.valid_up_to()]).expect("validated prefix")
        }
        Err(_) => return outcome,
    };
    // A page shorter than one character keeps its bytes so reading still advances.
    if text.is_empty() {
        return outcome;
    }
    let consumed = lead + text.len();
    let next_offset = if consumed < bytes.len() {
        json!(offset + consumed as u64)
    } else {
        outcome.value["next_offset"].clone()
    };
    outcome.value = json!({
        "artifact": outcome.value["artifact"].clone(),
        "offset": offset + lead as u64,
        "next_offset": next_offset,
        "encoding": "utf8",
        "content": text,
    });
    outcome.content = ToolContent::inline(vec![ContentPart::text(outcome.value.to_string())]);
    outcome
}

/// Metadata-only inventory of the invoking session's artifacts.
///
/// Ownership comes from the runtime-owned invocation context, exactly as for
/// reads, and the call asks for the same `artifact.read` authority over a
/// `session-artifact` resource, so it stays as unattended as reading.
#[derive(Debug)]
pub struct ContextArtifactLister {
    catalog: Arc<dyn ArtifactCatalog>,
    children: Option<Arc<dyn ChildDirectory>>,
    /// Serialized page budget: rows stop before the page would exceed it, so
    /// a listing never needs offloading itself.
    page_bytes: usize,
}

/// The delegating session's view of its own children, by child session.
///
/// A stored child reply names its child in provenance, but a copied child
/// artifact names only the child session its lineage came from.
pub trait ChildDirectory: Send + Sync + std::fmt::Debug {
    /// The child whose session is `session`, if this runtime spawned it.
    fn child_for_session(&self, session: &SessionId) -> Option<ChildId>;
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArguments {
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    child_id: Option<String>,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_list_rows")]
    limit: usize,
}

const fn default_list_rows() -> usize {
    DEFAULT_LIST_ROWS
}

impl ListArguments {
    fn parse(arguments: &Value) -> Result<Self, RuntimeError> {
        let parsed: Self = serde_json::from_value(arguments.clone()).map_err(|error| {
            RuntimeError::tool(format!("invalid {ARTIFACT_LIST_TOOL_NAME} input: {error}"))
        })?;
        if !(1..=MAX_LIST_ROWS).contains(&parsed.limit) {
            return Err(RuntimeError::tool(format!(
                "{ARTIFACT_LIST_TOOL_NAME} limit must be in 1..={MAX_LIST_ROWS}"
            )));
        }
        for (field, value) in [("purpose", &parsed.purpose), ("child_id", &parsed.child_id)] {
            if value
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.chars().count() > MAX_PURPOSE_CHARS)
            {
                return Err(RuntimeError::tool(format!(
                    "{ARTIFACT_LIST_TOOL_NAME} {field} must contain 1..={MAX_PURPOSE_CHARS} characters"
                )));
            }
        }
        Ok(parsed)
    }

    fn canonical(&self) -> Value {
        json!({
            "purpose": self.purpose,
            "child_id": self.child_id,
            "offset": self.offset,
            "limit": self.limit,
        })
    }
}

impl ContextArtifactLister {
    /// The child behind an artifact: named outright on a stored reply,
    /// resolved through the lineage's child session on a copy.
    fn child_of(&self, reference: &ArtifactRef) -> Option<ChildId> {
        let provenance = &reference.provenance;
        provenance.child.clone().or_else(|| {
            let source = provenance.derived_from.as_ref()?;
            self.children.as_ref()?.child_for_session(&source.session)
        })
    }
}

fn artifact_row(reference: &ArtifactRef, child: Option<&ChildId>) -> Value {
    let provenance = &reference.provenance;
    json!({
        "id": reference.id.as_str(),
        "purpose": provenance.purpose,
        "media_type": reference.media_type,
        "bytes": reference.byte_length,
        "turn": provenance.turn.as_ref().map(|turn| turn.as_str()),
        "call": provenance.call.as_ref().map(|call| call.as_str()),
        "child_id": child.map(ChildId::as_str),
        "copied_from_session": provenance
            .derived_from
            .as_ref()
            .map(|source| source.session.as_str()),
    })
}

#[async_trait]
impl Tool for ContextArtifactLister {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            ARTIFACT_LIST_TOOL_NAME,
            format!(
                "List this session's stored artifacts, newest first: metadata only (id, purpose, \
                 media type, bytes, turn, call, the child that produced it, and the child \
                 session a copied artifact came from), never content. Use it to find a real \
                 artifact id; ids are never constructed. Filter by purpose or child_id. Read a \
                 listed artifact with artifact.read. Purposes include \
                 tool-output (your offloaded tool results), delegation.child-reply (a child's \
                 full reply), delegation.child-result (artifacts a child produced, copied to \
                 you), and child-transcript. Pages hold at most {MAX_LIST_ROWS} rows; follow \
                 next_offset."
            ),
            json!({
                "type": "object",
                "properties": {
                    "purpose": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_PURPOSE_CHARS,
                        "description": "Only list artifacts with exactly this purpose."
                    },
                    "child_id": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_PURPOSE_CHARS,
                        "description": "Only list artifacts a child produced, such as child-1."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Rows to skip; use next_offset from the previous page."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_LIST_ROWS,
                        "default": DEFAULT_LIST_ROWS
                    }
                },
                "additionalProperties": false
            }),
            ToolEffects::default(),
        )
        .with_permission_upper_bound(PermissionSet::single(Permission::other(
            ARTIFACT_READ_PERMISSION,
        )))
    }

    async fn prepare(
        &self,
        arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let arguments = ListArguments::parse(&arguments)?;
        let filters = [arguments.purpose.as_deref(), arguments.child_id.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let detail = if filters.is_empty() {
            format!("from row {}", arguments.offset)
        } else {
            format!("{} from row {}", filters.join(", "), arguments.offset)
        };
        Ok(PreparedToolCall::new(
            ctx.call_id.clone(),
            ARTIFACT_LIST_TOOL_NAME,
            arguments.canonical(),
            PermissionSet::single(Permission::other(ARTIFACT_READ_PERMISSION)),
            SecurityResource::other("session-artifact", "index"),
            ToolEffects::default(),
            ToolCallDisplay::new("List session artifacts").with_detail(detail),
        ))
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let arguments = ListArguments::parse(prepared.arguments())?;
        let owned = self
            .catalog
            .list_owned(&ctx.session)
            .await
            .map_err(|error| RuntimeError::tool(error.to_string()))?;
        let matching = owned
            .iter()
            .map(|reference| (reference, self.child_of(reference)))
            .filter(|(reference, child)| {
                arguments
                    .purpose
                    .as_ref()
                    .is_none_or(|purpose| &reference.provenance.purpose == purpose)
                    && arguments.child_id.as_ref().is_none_or(|wanted| {
                        child.as_ref().is_some_and(|child| child.as_str() == wanted)
                    })
            })
            .collect::<Vec<_>>();
        let total = matching.len();
        let mut rows = Vec::new();
        let mut used = 0;
        for (reference, child) in matching.iter().skip(arguments.offset).take(arguments.limit) {
            let row = artifact_row(reference, child.as_ref());
            let size = row.to_string().len() + 1;
            // Always return at least one row, so a page can never stall.
            if !rows.is_empty() && used + size > self.page_bytes {
                break;
            }
            used += size;
            rows.push(row);
        }
        let end = arguments.offset.saturating_add(rows.len());
        Ok(ToolOutcome::json(json!({
            "artifacts": rows,
            "total": total,
            "offset": arguments.offset,
            "next_offset": (end < total).then_some(end),
        })))
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
    sections.push(format!("Read more with artifact.read; discover it with registry.search (query: {ARTIFACT_DISCOVERY_QUERY}) if absent."));
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
            reply_bytes: 65536,
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
    async fn a_childs_reply_stays_inline_up_to_the_output_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let offloader = policy().offloader(store(dir.path())).unwrap();
        let mut agent = view();
        agent.call.name = AGENT_TOOL_NAME.into();
        let report = "a finding\n".repeat(3000);
        for value in [
            json!({"child_id": "c1", "state": "idle", "result": {"text": report, "artifacts": []}}),
            json!({"child_id": "c1", "state": "idle", "result": report}),
        ] {
            let outcome = ToolOutcome::json(value);
            let patch = offloader.process(&agent, outcome.clone()).await.unwrap();
            assert_eq!(
                serde_json::to_value(patch.outcome).unwrap(),
                serde_json::to_value(outcome).unwrap()
            );
        }
        let offloaded = [
            // Over the ceiling, a reply follows the ordinary rule.
            (
                &agent,
                ToolOutcome::json(json!({"result": {"text": "x".repeat(70_000)}})),
            ),
            // So does an agent outcome that carries no reply.
            (&agent, ToolOutcome::json(json!({"children": report}))),
            (&view(), ToolOutcome::text(report.clone())),
        ];
        for (view, outcome) in offloaded {
            let patch = offloader.process(view, outcome).await.unwrap();
            assert!(patch.outcome.content.artifact_reference().is_some());
        }
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
        let stale = ArtifactReadTool::new(reopened.clone())
            .prepare(json!({"id": reference.id.as_str(), "limit": 65536}), &prep)
            .await
            .unwrap();
        assert!(
            reader.invoke(stale, &invocation(&prep)).await.is_err(),
            "an old prepared call must not bypass the current cap"
        );
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

    #[tokio::test]
    async fn pages_that_split_a_character_still_read_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let put = |bytes: Vec<u8>, key: &str| ArtifactWrite {
            bytes,
            media_type: "text/plain".into(),
            sensitivity: agent_runtime_core::artifact::ArtifactSensitivity::Sensitive,
            retention: agent_runtime_core::artifact::ArtifactRetention::Session,
            provenance: agent_runtime_core::artifact::ArtifactProvenance::new(
                SessionId::new("owner"),
                "test",
            ),
            idempotency_key: key.into(),
        };
        // Every character is two bytes, so any odd page edge splits one.
        let text = store
            .put(put("é".repeat(2000).into_bytes(), "text"))
            .await
            .unwrap();
        let binary = store
            .put(put(vec![0xFF, 0xFE, 0x00, 0x41], "binary"))
            .await
            .unwrap();
        let reader = policy().reader(store);
        let prep = preparation();
        let read = |id: &str, offset: u64, limit: u64| {
            let args = json!({"id": id, "offset": offset, "limit": limit});
            let (reader, prep) = (&reader, &prep);
            async move {
                let prepared = reader.prepare(args, prep).await.unwrap();
                reader.invoke(prepared, &invocation(prep)).await.unwrap()
            }
        };

        // The page ends on a character's first byte.
        let page = read(text.id.as_str(), 0, 2047).await;
        assert_eq!(page.value["encoding"], "utf8");
        assert_eq!(page.value["content"], "é".repeat(1023));
        assert_eq!(page.value["offset"], 0);
        assert_eq!(page.value["next_offset"], 2046);
        let model_facing = match &page.content.as_inline().unwrap()[0] {
            ContentPart::Text { text } => text.clone(),
            other => panic!("expected text, got {other:?}"),
        };
        assert_eq!(
            serde_json::from_str::<Value>(&model_facing).unwrap(),
            page.value
        );

        // A chosen offset inside a character skips the partial leading bytes.
        let page = read(text.id.as_str(), 1, 2048).await;
        assert_eq!(page.value["encoding"], "utf8");
        assert_eq!(page.value["offset"], 2);
        assert_eq!(page.value["content"], "é".repeat(1023));
        assert_eq!(page.value["next_offset"], 2048);

        // Shorter than one character: the bytes stay, and reading advances.
        let page = read(text.id.as_str(), 0, 1).await;
        assert_eq!(page.value["encoding"], "bytes");
        assert_eq!(page.value["next_offset"], 1);

        // Bytes that are not UTF-8 inside the page are left alone.
        let page = read(binary.id.as_str(), 0, 2048).await;
        assert_eq!(page.value["encoding"], "bytes");
        assert_eq!(page.value["content"], json!([0xFF, 0xFE, 0x00, 0x41]));
    }
    async fn put(
        store: &SmithArtifactStore,
        session: &str,
        purpose: &str,
        key: &str,
    ) -> ArtifactRef {
        store
            .put(ArtifactWrite {
                bytes: format!("{session} {purpose} {key}").into_bytes(),
                media_type: "text/plain".into(),
                sensitivity: agent_runtime_core::artifact::ArtifactSensitivity::Sensitive,
                retention: agent_runtime_core::artifact::ArtifactRetention::Session,
                provenance: agent_runtime_core::artifact::ArtifactProvenance::new(
                    SessionId::new(session),
                    purpose,
                ),
                idempotency_key: key.into(),
            })
            .await
            .unwrap()
    }
    async fn call(tool: &dyn Tool, arguments: Value) -> Result<ToolOutcome, RuntimeError> {
        let prep = preparation();
        let prepared = tool.prepare(arguments, &prep).await?;
        tool.invoke(prepared, &invocation(&prep)).await
    }
    fn named<'a>(tools: &'a [Arc<dyn Tool>], name: &str) -> &'a dyn Tool {
        tools
            .iter()
            .find(|tool| tool.spec().name == name)
            .unwrap_or_else(|| panic!("{name} is registered"))
            .as_ref()
    }
    #[tokio::test]
    async fn lister_pages_only_the_owners_metadata_and_filters_by_purpose() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        for key in ["a", "b", "c"] {
            put(&store, "owner", "delegation.child-result", key).await;
        }
        let reply = put(&store, "owner", "delegation.child-reply", "reply").await;
        put(&store, "other", "delegation.child-reply", "hidden").await;
        let tools = policy().artifact_tools(
            store.clone(),
            Some(store.clone() as Arc<dyn ArtifactCatalog>),
            None,
        );
        let lister = named(&tools, ARTIFACT_LIST_TOOL_NAME);

        let replies = call(lister, json!({"purpose": "delegation.child-reply"}))
            .await
            .unwrap()
            .value;
        assert_eq!(replies["total"], 1, "{replies}");
        assert_eq!(
            replies["artifacts"][0]["id"],
            reply.id.as_str(),
            "{replies}"
        );
        assert_eq!(replies["artifacts"][0]["bytes"], reply.byte_length);
        assert!(replies["artifacts"][0].get("content").is_none());
        assert!(replies["next_offset"].is_null());

        let first = call(lister, json!({"limit": 2})).await.unwrap().value;
        assert_eq!(first["total"], 4, "{first}");
        assert_eq!(first["artifacts"].as_array().unwrap().len(), 2);
        assert_eq!(first["next_offset"], 2);
        let rest = call(lister, json!({"offset": 2, "limit": 2}))
            .await
            .unwrap()
            .value;
        assert_eq!(rest["artifacts"].as_array().unwrap().len(), 2);
        assert!(rest["next_offset"].is_null(), "{rest}");
        let listed = [&first, &rest]
            .iter()
            .flat_map(|page| page["artifacts"].as_array().unwrap().clone())
            .map(|row| row["id"].as_str().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(listed.len(), 4);
        let reader = named(&tools, "artifact.read");
        for id in &listed {
            call(reader, json!({"id": id})).await.unwrap();
        }

        for invalid in [
            json!({"limit": 0}),
            json!({"limit": 101}),
            json!({"purpose": ""}),
        ] {
            assert!(call(lister, invalid).await.is_err());
        }
        // A listing is sized to fit, so it is never itself offloaded.
        let mut list_view = view();
        list_view.call.name = ARTIFACT_LIST_TOOL_NAME.into();
        let raw = ToolOutcome::text("x".repeat(9000));
        let result = policy()
            .offloader(store)
            .unwrap()
            .process(&list_view, raw.clone())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(result.outcome).unwrap(),
            serde_json::to_value(raw).unwrap()
        );
    }
    #[derive(Debug)]
    struct OneChild;
    impl ChildDirectory for OneChild {
        fn child_for_session(&self, session: &SessionId) -> Option<ChildId> {
            (session.as_str() == "child-session-1").then(|| ChildId::new("child-1"))
        }
    }
    #[tokio::test]
    async fn lister_attributes_replies_and_copies_to_their_child() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let attributed = |purpose: &str, key: &str, child: &str| ArtifactWrite {
            bytes: key.as_bytes().to_vec(),
            media_type: "text/plain".into(),
            sensitivity: agent_runtime_core::artifact::ArtifactSensitivity::Sensitive,
            retention: agent_runtime_core::artifact::ArtifactRetention::Session,
            provenance: agent_runtime_core::artifact::ArtifactProvenance::new(
                SessionId::new("owner"),
                purpose,
            )
            .with_child(ChildId::new(child)),
            idempotency_key: key.into(),
        };
        let reply = store
            .put(attributed("delegation.child-reply", "reply-1", "child-1"))
            .await
            .unwrap();
        store
            .put(attributed("delegation.child-reply", "reply-2", "child-2"))
            .await
            .unwrap();
        let mut copied = attributed("delegation.child-result", "copied-1", "unused");
        copied.provenance.child = None;
        copied.provenance =
            copied
                .provenance
                .with_derived_from(agent_runtime_core::artifact::ArtifactLineage {
                    session: SessionId::new("child-session-1"),
                    id: agent_runtime_core::artifact::ArtifactId::new("a-source").unwrap(),
                    digest: reply.digest.clone(),
                });
        let copy = store.put(copied).await.unwrap();
        put(&store, "owner", "tool-output", "own").await;

        let tools = policy().artifact_tools(
            store.clone(),
            Some(store.clone() as Arc<dyn ArtifactCatalog>),
            Some(Arc::new(OneChild)),
        );
        let lister = named(&tools, ARTIFACT_LIST_TOOL_NAME);
        let first = call(lister, json!({"child_id": "child-1"}))
            .await
            .unwrap()
            .value;
        assert_eq!(first["total"], 2, "{first}");
        let ids = first["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                assert_eq!(row["child_id"], "child-1", "{row}");
                row["id"].as_str().unwrap().to_owned()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            ids,
            [reply.id.as_str().to_owned(), copy.id.as_str().to_owned()].into()
        );
        let replies = call(
            lister,
            json!({"child_id": "child-2", "purpose": "delegation.child-reply"}),
        )
        .await
        .unwrap()
        .value;
        assert_eq!(replies["total"], 1, "{replies}");
        let everything = call(lister, json!({})).await.unwrap().value;
        let own = everything["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["purpose"] == "tool-output")
            .unwrap();
        assert!(own["child_id"].is_null(), "{own}");
        assert!(call(lister, json!({"child_id": ""})).await.is_err());
    }
    #[tokio::test]
    async fn a_missing_id_names_the_lister_only_when_it_is_registered() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let invented =
            json!({"id": "a-0000000000000000000000000000000000000000000000000000000000000000"});

        let with_lister = policy().artifact_tools(
            store.clone(),
            Some(store.clone() as Arc<dyn ArtifactCatalog>),
            None,
        );
        let error = call(named(&with_lister, "artifact.read"), invented.clone())
            .await
            .unwrap_err();
        assert!(
            error.message.starts_with(ARTIFACT_NOT_FOUND),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("never constructed"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains(ARTIFACT_LIST_TOOL_NAME),
            "{}",
            error.message
        );

        let alone = policy().artifact_tools(store.clone(), None, None);
        assert_eq!(alone.len(), 1);
        let error = call(named(&alone, "artifact.read"), invented)
            .await
            .unwrap_err();
        assert!(
            error.message.contains("never constructed"),
            "{}",
            error.message
        );
        assert!(
            !error.message.contains(ARTIFACT_LIST_TOOL_NAME),
            "{}",
            error.message
        );

        // Ownership failures keep Runtime's own error.
        let foreign = put(&store, "other", "tool-output", "foreign").await;
        let error = call(
            named(&with_lister, "artifact.read"),
            json!({"id": foreign.id.as_str()}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.message, "artifact access denied");
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
