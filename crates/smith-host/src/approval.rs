//! The interactive approval gate.
//!
//! The runtime calls [`ApprovalPolicy::decide`] and awaits an answer before any
//! mutating or process-spawning tool runs. Smith answers by handing the request
//! to whichever surface owns the user's attention: the TUI draws a modal, and
//! the user's keystroke resolves the request.
//!
//! The channel is the seam. [`InteractiveApproval`] knows nothing about
//! rendering, so the same policy serves the TUI, a test, or a future headless
//! host that answers from a configured rule set.
//!
//! Three behaviors are deliberate:
//!
//! - **Fail closed on a dead surface.** If the receiver is gone — the TUI
//!   crashed, or shutdown raced a tool call — the request is unavailable,
//!   never allowed and never left hanging.
//! - **The prompt carries the prepared action.** Smith never reconstructs an
//!   approval target from model-authored arguments after preparation.
//! - **Session allowances remain resource-scoped.** Answering "allow for the
//!   session" covers only later calls of the same tool whose permissions and
//!   concrete resource are contained by the action the user reviewed.

use std::sync::Mutex;

use agent_runtime_core::approval::{ApprovalDecision, ApprovalPolicy, ApprovalRequest};
use agent_runtime_core::clock::Deadline;
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::security::{PermissionSet, SecurityResource};
use agent_runtime_core::tool::PreparedToolCall;
use agent_runtime_registry::Permission;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// Redaction-safe evidence that a headless run reached an approval boundary.
///
/// Argument values stay out of this record. Machine output can identify the
/// blocked tool and its input shape without copying a command, patch, token,
/// or other potentially sensitive value into an automation log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequired {
    /// The runtime's tool-call identity.
    pub call_id: String,
    /// The tool that was blocked.
    pub tool: String,
    /// Sorted top-level argument names.
    pub argument_keys: Vec<String>,
    /// Whether the declared effects include a write or spawned process.
    pub mutates: bool,
    /// Whether the declared effects require any explicit authorization.
    pub requires_authorization: bool,
    /// Typed permissions required by the exact prepared action.
    pub permissions: Vec<String>,
    /// Exact canonical resource the prepared action targets.
    pub resource: SecurityResource,
    /// Redaction-safe warnings for broad authority.
    pub authority_warnings: Vec<String>,
    /// Absolute approval deadline in Unix milliseconds, if finite.
    pub deadline_at_ms: Option<u64>,
    /// Fingerprint binding arguments, permissions, resource, effects, and
    /// display metadata without exposing raw argument values.
    pub preparation_fingerprint: String,
}

impl ApprovalRequired {
    fn from_request(request: &ApprovalRequest) -> Self {
        let prepared = request.prepared();
        let mut argument_keys: Vec<String> = request
            .prepared()
            .arguments()
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        argument_keys.sort();
        let permissions = prepared
            .required_permissions()
            .iter()
            .map(ToString::to_string)
            .collect();
        Self {
            call_id: prepared.call_id().as_str().to_owned(),
            tool: prepared.tool().to_owned(),
            argument_keys,
            mutates: prepared
                .required_permissions()
                .iter()
                .any(permission_mutates),
            requires_authorization: !prepared.required_permissions().is_empty(),
            permissions,
            resource: prepared.resource().clone(),
            authority_warnings: authority_warnings(prepared),
            deadline_at_ms: request.deadline().instant().map(|time| time.as_millis()),
            preparation_fingerprint: prepared.fingerprint().as_str().to_owned(),
        }
    }
}

fn permission_mutates(permission: &Permission) -> bool {
    matches!(
        permission,
        Permission::FsWrite
            | Permission::FsCreate
            | Permission::FsDelete
            | Permission::ProcessSpawn
            | Permission::StdioWrite
    )
}

fn authority_warnings(prepared: &PreparedToolCall) -> Vec<String> {
    let permissions = prepared.required_permissions();
    let mut warnings = Vec::new();
    if permissions.contains(&Permission::ProcessSpawn) {
        warnings.push("process_execution".into());
    }
    if permissions.contains(&Permission::FsDelete) {
        warnings.push("file_deletion".into());
    }
    if matches!(
        prepared.resource(),
        SecurityResource::Filesystem { segments, .. } if segments.is_empty()
    ) && (permissions.contains(&Permission::FsWrite)
        || permissions.contains(&Permission::FsCreate)
        || permissions.contains(&Permission::FsDelete))
    {
        warnings.push("workspace_root_mutation".into());
    }
    if permissions.contains(&Permission::CredentialUse) {
        warnings.push("credential_use".into());
    }
    if permissions.contains(&Permission::DataEgress) {
        warnings.push("data_egress".into());
    }
    if permissions.contains(&Permission::NetHttp) {
        warnings.push("outbound_network_access".into());
    }
    if permissions
        .iter()
        .any(|permission| matches!(permission, Permission::Other(_)))
    {
        warnings.push("host_defined_authority".into());
    }
    warnings
}

/// A non-interactive approval surface that always denies and records why.
///
/// Supplying this policy lets an unattended turn use read-only tools while
/// turning the first authority-bearing request into a stable host outcome.
/// It never waits for input and never stores raw argument values.
#[derive(Debug, Default)]
pub struct HeadlessApproval {
    required: Mutex<Option<ApprovalRequired>>,
}

impl HeadlessApproval {
    /// Creates a fail-closed headless policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// The first request that required a policy the caller did not provide.
    pub fn required(&self) -> Option<ApprovalRequired> {
        self.required
            .lock()
            .expect("headless approval state poisoned")
            .clone()
    }
}

#[async_trait]
impl ApprovalPolicy for HeadlessApproval {
    async fn decide(&self, request: &ApprovalRequest) -> ApprovalDecision {
        let mut required = self
            .required
            .lock()
            .expect("headless approval state poisoned");
        if required.is_none() {
            *required = Some(ApprovalRequired::from_request(request));
        }
        ApprovalDecision::unavailable(
            "headless approval required; supply an explicit approval policy to authorize this tool",
        )
    }
}

/// How broadly an answer applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptScope {
    /// Applies to this call only.
    Once,
    /// Applies to later contained calls of the same tool in this session.
    Session,
}

/// One approval request handed to the user-facing surface, with the channel to
/// answer it on.
///
/// Dropping a prompt without answering cancels the request. That makes the
/// closed path the default: forgetting to answer cannot grant a shell command,
/// and the runtime can distinguish abandonment from an explicit user denial.
#[derive(Debug)]
pub struct ApprovalPrompt {
    /// What the runtime is asking to run.
    request: ApprovalRequest,
    responder: Option<oneshot::Sender<(ApprovalDecision, PromptScope)>>,
}

impl ApprovalPrompt {
    /// Allows the invocation.
    pub fn allow(mut self, scope: PromptScope) {
        if let Some(responder) = self.responder.take() {
            let _ = responder.send((ApprovalDecision::Allow, scope));
        }
    }

    /// Denies the invocation with a reason shown to the model.
    pub fn deny(mut self, reason: impl Into<String>) {
        if let Some(responder) = self.responder.take() {
            let _ = responder.send((ApprovalDecision::deny(reason), PromptScope::Once));
        }
    }

    /// Proposes replacement raw arguments.
    ///
    /// This does not approve the current action. Agent Runtime discards its
    /// eligibility and starts a new validate/prepare/authorize/approval cycle.
    pub fn edit(mut self, arguments: Value) {
        if let Some(responder) = self.responder.take() {
            let _ = responder.send((ApprovalDecision::Edit { arguments }, PromptScope::Once));
        }
    }

    /// Resolves the prompt as timed out.
    pub fn time_out(mut self) {
        if let Some(responder) = self.responder.take() {
            let _ = responder.send((ApprovalDecision::TimedOut, PromptScope::Once));
        }
    }

    /// Resolves the prompt as cancelled with its turn or session.
    pub fn cancel(mut self) {
        if let Some(responder) = self.responder.take() {
            let _ = responder.send((ApprovalDecision::Cancelled, PromptScope::Once));
        }
    }

    /// The tool being requested.
    pub fn tool(&self) -> &str {
        self.request.prepared().tool()
    }

    /// The exact immutable action the runtime prepared.
    pub fn prepared(&self) -> &PreparedToolCall {
        self.request.prepared()
    }

    /// The absolute deadline the runtime is enforcing around this prompt.
    pub fn deadline(&self) -> Deadline {
        self.request.deadline()
    }
}

impl Drop for ApprovalPrompt {
    fn drop(&mut self) {
        if let Some(responder) = self.responder.take() {
            let _ = responder.send((ApprovalDecision::Cancelled, PromptScope::Once));
        }
    }
}

/// The receiving half: the surface that shows prompts to the user.
#[derive(Debug)]
pub struct ApprovalRequests {
    rx: mpsc::Receiver<ApprovalPrompt>,
}

impl ApprovalRequests {
    /// Waits for the next approval prompt, or `None` once the runtime is gone.
    pub async fn recv(&mut self) -> Option<ApprovalPrompt> {
        self.rx.recv().await
    }

    /// Takes a pending prompt without waiting.
    pub fn try_recv(&mut self) -> Option<ApprovalPrompt> {
        self.rx.try_recv().ok()
    }
}

/// An approval policy that asks the user.
#[derive(Debug)]
pub struct InteractiveApproval {
    tx: mpsc::Sender<ApprovalPrompt>,
    session_allowed: Mutex<Vec<SessionAllowance>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionAllowance {
    session: SessionId,
    tool: String,
    permissions: PermissionSet,
    resource: SecurityResource,
}

impl SessionAllowance {
    fn from_request(request: &ApprovalRequest) -> Self {
        let prepared = request.prepared();
        Self {
            session: request.origin().session().clone(),
            tool: prepared.tool().to_owned(),
            permissions: prepared.required_permissions().clone(),
            resource: prepared.resource().clone(),
        }
    }

    fn covers(&self, request: &ApprovalRequest) -> bool {
        let prepared = request.prepared();
        self.session == *request.origin().session()
            && self.tool == prepared.tool()
            && prepared.required_permissions().is_subset(&self.permissions)
            && self.resource.contains(prepared.resource())
    }
}

impl InteractiveApproval {
    /// Creates the policy and the receiver its prompts arrive on.
    pub fn new(capacity: usize) -> (Self, ApprovalRequests) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (
            Self {
                tx,
                session_allowed: Mutex::new(Vec::new()),
            },
            ApprovalRequests { rx },
        )
    }

    /// Whether a tool already carries an allowance in this exact session.
    pub fn is_session_allowed(&self, session: &SessionId, tool: &str) -> bool {
        self.session_allowed
            .lock()
            .expect("approval allowlist poisoned")
            .iter()
            .any(|allowance| allowance.session == *session && allowance.tool == tool)
    }

    fn is_prepared_allowed(&self, request: &ApprovalRequest) -> bool {
        self.session_allowed
            .lock()
            .expect("approval allowlist poisoned")
            .iter()
            .any(|allowance| allowance.covers(request))
    }
}

#[async_trait]
impl ApprovalPolicy for InteractiveApproval {
    async fn decide(&self, request: &ApprovalRequest) -> ApprovalDecision {
        if self.is_prepared_allowed(request) {
            return ApprovalDecision::Allow;
        }

        let (responder, answer) = oneshot::channel();
        let prompt = ApprovalPrompt {
            request: request.clone(),
            responder: Some(responder),
        };
        if self.tx.send(prompt).await.is_err() {
            return ApprovalDecision::unavailable("no approval surface is available");
        }

        match answer.await {
            Ok((ApprovalDecision::Allow, scope)) => {
                if scope == PromptScope::Session {
                    self.session_allowed
                        .lock()
                        .expect("approval allowlist poisoned")
                        .push(SessionAllowance::from_request(request));
                }
                ApprovalDecision::Allow
            }
            Ok((denial, _)) => denial,
            // The surface dropped the prompt without answering.
            Err(_) => ApprovalDecision::Cancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::approval::ApprovalOrigin;
    use agent_runtime_core::clock::Deadline;
    use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId};
    use agent_runtime_core::security::SecurityResource;
    use agent_runtime_core::tool::{PreparedToolCall, ToolCallDisplay, ToolEffects};

    fn request(tool: &str) -> ApprovalRequest {
        request_for(tool, Vec::new())
    }

    fn request_for(tool: &str, segments: Vec<String>) -> ApprovalRequest {
        request_for_session(tool, segments, "session-1")
    }

    fn request_for_session(tool: &str, segments: Vec<String>, session: &str) -> ApprovalRequest {
        let effects = ToolEffects::read_only().with_write("/repo");
        let (permissions, _) = effects.authorization_request(tool, "/repo");
        ApprovalRequest::new(
            PreparedToolCall::new(
                ToolCallId::new("call-1"),
                tool,
                serde_json::json!({"command": "rm -rf build"}),
                permissions,
                SecurityResource::filesystem("/repo", segments),
                effects,
                ToolCallDisplay::new(format!("Run {tool}")),
            ),
            Deadline::never(),
            ApprovalOrigin::new(
                SessionId::new(session),
                RequestId::new(format!("request-{session}")),
            ),
        )
    }

    #[tokio::test]
    async fn an_allowed_prompt_permits_the_call() {
        let (policy, mut requests) = InteractiveApproval::new(4);
        let surface = tokio::spawn(async move {
            let prompt = requests.recv().await.expect("a prompt");
            assert_eq!(prompt.tool(), "shell");
            prompt.allow(PromptScope::Once);
        });

        assert!(policy.decide(&request("shell")).await.is_allowed());
        surface.await.unwrap();
    }

    #[tokio::test]
    async fn a_denial_carries_its_reason_to_the_model() {
        let (policy, mut requests) = InteractiveApproval::new(4);
        tokio::spawn(async move {
            requests
                .recv()
                .await
                .expect("a prompt")
                .deny("the user declined");
        });

        match policy.decide(&request("shell")).await {
            ApprovalDecision::Deny { reason } => assert_eq!(reason, "the user declined"),
            other => panic!("expected a denial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_session_allowance_covers_only_the_same_tool_and_resource() {
        let (policy, mut requests) = InteractiveApproval::new(4);
        let surface = tokio::spawn(async move {
            requests
                .recv()
                .await
                .expect("first prompt")
                .allow(PromptScope::Session);
            // The contained `edit` call must not reach the surface. A
            // different target and a different tool must each prompt.
            let next = requests.recv().await.expect("second prompt");
            assert_eq!(next.tool(), "edit");
            next.deny("a different target still asks");
            let next = requests.recv().await.expect("third prompt");
            assert_eq!(next.tool(), "shell");
            next.deny("a different tool still asks");
        });

        assert!(
            policy
                .decide(&request_for("edit", vec!["src".into()]))
                .await
                .is_allowed()
        );
        assert!(
            policy
                .decide(&request_for("edit", vec!["src".into(), "lib.rs".into()]))
                .await
                .is_allowed()
        );
        assert!(
            !policy
                .decide(&request_for("edit", vec!["tests".into()]))
                .await
                .is_allowed()
        );
        assert!(!policy.decide(&request("shell")).await.is_allowed());
        surface.await.unwrap();
    }

    #[tokio::test]
    async fn a_session_allowance_never_leaks_to_another_runtime_session() {
        let (policy, mut requests) = InteractiveApproval::new(4);
        let surface = tokio::spawn(async move {
            let first = requests.recv().await.expect("first-session prompt");
            first.allow(PromptScope::Session);

            let second = requests.recv().await.expect("second-session prompt");
            assert_eq!(second.tool(), "edit");
            second.deny("another session must ask independently");
        });

        assert!(
            policy
                .decide(&request_for_session(
                    "edit",
                    vec!["src".into(), "lib.rs".into()],
                    "session-1",
                ))
                .await
                .is_allowed()
        );
        assert!(
            !policy
                .decide(&request_for_session(
                    "edit",
                    vec!["src".into(), "lib.rs".into()],
                    "session-2",
                ))
                .await
                .is_allowed()
        );
        assert!(policy.is_session_allowed(&SessionId::new("session-1"), "edit"));
        assert!(!policy.is_session_allowed(&SessionId::new("session-2"), "edit"));
        surface.await.unwrap();
    }

    #[tokio::test]
    async fn a_dropped_prompt_denies_rather_than_hangs() {
        let (policy, mut requests) = InteractiveApproval::new(4);
        tokio::spawn(async move {
            drop(requests.recv().await.expect("a prompt"));
        });
        assert_eq!(
            policy.decide(&request("shell")).await,
            ApprovalDecision::Cancelled
        );
    }

    #[tokio::test]
    async fn a_missing_surface_fails_closed() {
        let (policy, requests) = InteractiveApproval::new(4);
        drop(requests);
        match policy.decide(&request("shell")).await {
            ApprovalDecision::Unavailable { reason } => {
                assert!(reason.contains("no approval surface"));
            }
            other => panic!("expected unavailable approval, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn headless_approval_denies_without_retaining_argument_values() {
        let policy = HeadlessApproval::new();
        let decision = policy.decide(&request("shell")).await;

        assert!(!decision.is_allowed());
        let required = policy.required().expect("prepared approval evidence");
        assert_eq!(required.call_id, "call-1");
        assert_eq!(required.tool, "shell");
        assert_eq!(required.argument_keys, ["command"]);
        assert!(required.mutates);
        assert!(required.requires_authorization);
        assert_eq!(required.permissions, ["fs.read", "fs.write"]);
        assert_eq!(
            required.resource,
            SecurityResource::filesystem("/repo", Vec::new())
        );
        assert_eq!(required.authority_warnings, ["workspace_root_mutation"]);
        assert_eq!(required.deadline_at_ms, None);
        assert_eq!(required.preparation_fingerprint.len(), 32);
        assert!(
            !format!("{:?}", policy.required()).contains("rm -rf"),
            "headless outcome retained a protected argument value"
        );
    }

    #[tokio::test]
    async fn headless_authority_evidence_uses_typed_permissions_not_scheduler_effects() {
        let prepared = PreparedToolCall::new(
            ToolCallId::new("credential-call"),
            "credential",
            serde_json::json!({"reference": "provider"}),
            PermissionSet::single(Permission::CredentialUse),
            SecurityResource::credential("provider"),
            ToolEffects::new(Vec::new()),
            ToolCallDisplay::new("Use provider credential"),
        );
        let policy = HeadlessApproval::new();

        let decision = policy
            .decide(&ApprovalRequest::new(
                prepared,
                Deadline::never(),
                ApprovalOrigin::new(SessionId::new("session-1"), RequestId::new("request-1")),
            ))
            .await;

        assert!(matches!(decision, ApprovalDecision::Unavailable { .. }));
        let required = policy.required().expect("redacted approval evidence");
        assert!(required.requires_authorization);
        assert!(!required.mutates);

        let delete = PreparedToolCall::new(
            ToolCallId::new("delete-call"),
            "delete",
            serde_json::json!({"path": "obsolete.txt"}),
            PermissionSet::single(Permission::FsDelete),
            SecurityResource::filesystem("/repo", vec!["obsolete.txt".into()]),
            ToolEffects::new(Vec::new()),
            ToolCallDisplay::new("Delete obsolete.txt"),
        );
        let required = ApprovalRequired::from_request(&ApprovalRequest::new(
            delete,
            Deadline::never(),
            ApprovalOrigin::new(SessionId::new("session-1"), RequestId::new("request-2")),
        ));
        assert!(required.requires_authorization);
        assert!(required.mutates);
    }
}
