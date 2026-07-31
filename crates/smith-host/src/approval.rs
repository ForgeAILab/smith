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
//! Two behaviors are deliberate:
//!
//! - **Fail closed on a dead surface.** If the receiver is gone — the TUI
//!   crashed, or shutdown raced a tool call — the request is denied, never
//!   allowed and never left hanging.
//! - **Session allowances are per-tool, not per-call.** Answering "allow for
//!   the session" grants the tool, so an agent looping over twenty files does
//!   not ask twenty times. It does not grant *other* tools.

use std::collections::BTreeSet;
use std::sync::Mutex;

use agent_runtime_core::approval::{ApprovalDecision, ApprovalPolicy, ApprovalRequest};
use async_trait::async_trait;
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
}

impl ApprovalRequired {
    fn from_request(request: &ApprovalRequest) -> Self {
        let mut argument_keys: Vec<String> = request
            .arguments
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        argument_keys.sort();
        Self {
            call_id: request.call_id.as_str().to_owned(),
            tool: request.tool.clone(),
            argument_keys,
            mutates: request.effects.mutates(),
            requires_authorization: request.effects.requires_authorization(),
        }
    }
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
        ApprovalDecision::deny(
            "headless approval required; supply an explicit approval policy to authorize this tool",
        )
    }
}

/// How broadly an answer applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptScope {
    /// Applies to this call only.
    Once,
    /// Applies to every later call of the same tool in this session.
    Session,
}

/// One approval request handed to the user-facing surface, with the channel to
/// answer it on.
///
/// Dropping a prompt without answering denies the request. That makes the
/// closed path the default: forgetting to answer cannot grant a shell command.
#[derive(Debug)]
pub struct ApprovalPrompt {
    /// What the runtime is asking to run.
    pub request: ApprovalRequest,
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

    /// The tool being requested.
    pub fn tool(&self) -> &str {
        &self.request.tool
    }
}

impl Drop for ApprovalPrompt {
    fn drop(&mut self) {
        if let Some(responder) = self.responder.take() {
            let _ = responder.send((
                ApprovalDecision::deny("the approval prompt was dismissed"),
                PromptScope::Once,
            ));
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
    session_allowed: Mutex<BTreeSet<String>>,
}

impl InteractiveApproval {
    /// Creates the policy and the receiver its prompts arrive on.
    pub fn new(capacity: usize) -> (Self, ApprovalRequests) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (
            Self {
                tx,
                session_allowed: Mutex::new(BTreeSet::new()),
            },
            ApprovalRequests { rx },
        )
    }

    /// Whether a tool already carries a session-wide allowance.
    pub fn is_session_allowed(&self, tool: &str) -> bool {
        self.session_allowed
            .lock()
            .expect("approval allowlist poisoned")
            .contains(tool)
    }
}

#[async_trait]
impl ApprovalPolicy for InteractiveApproval {
    async fn decide(&self, request: &ApprovalRequest) -> ApprovalDecision {
        if self.is_session_allowed(&request.tool) {
            return ApprovalDecision::Allow;
        }

        let (responder, answer) = oneshot::channel();
        let prompt = ApprovalPrompt {
            request: request.clone(),
            responder: Some(responder),
        };
        if self.tx.send(prompt).await.is_err() {
            return ApprovalDecision::deny("no approval surface is available");
        }

        match answer.await {
            Ok((ApprovalDecision::Allow, scope)) => {
                if scope == PromptScope::Session {
                    self.session_allowed
                        .lock()
                        .expect("approval allowlist poisoned")
                        .insert(request.tool.clone());
                }
                ApprovalDecision::Allow
            }
            Ok((denial, _)) => denial,
            // The surface dropped the prompt without answering.
            Err(_) => ApprovalDecision::deny("the approval prompt was abandoned"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::ids::ToolCallId;
    use agent_runtime_core::tool::ToolEffects;

    fn request(tool: &str) -> ApprovalRequest {
        ApprovalRequest {
            call_id: ToolCallId::new("call-1"),
            tool: tool.to_owned(),
            arguments: serde_json::json!({"command": "rm -rf build"}),
            effects: ToolEffects::read_only().with_write("/repo"),
        }
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
    async fn a_session_allowance_covers_the_same_tool_but_not_others() {
        let (policy, mut requests) = InteractiveApproval::new(4);
        let surface = tokio::spawn(async move {
            requests
                .recv()
                .await
                .expect("first prompt")
                .allow(PromptScope::Session);
            // The second `shell` call must not reach the surface at all; the
            // next prompt we see is for a different tool.
            let next = requests.recv().await.expect("second prompt");
            assert_eq!(next.tool(), "patch");
            next.deny("still asking for other tools");
        });

        assert!(policy.decide(&request("shell")).await.is_allowed());
        assert!(policy.decide(&request("shell")).await.is_allowed());
        assert!(!policy.decide(&request("patch")).await.is_allowed());
        surface.await.unwrap();
    }

    #[tokio::test]
    async fn a_dropped_prompt_denies_rather_than_hangs() {
        let (policy, mut requests) = InteractiveApproval::new(4);
        tokio::spawn(async move {
            drop(requests.recv().await.expect("a prompt"));
        });
        assert!(!policy.decide(&request("shell")).await.is_allowed());
    }

    #[tokio::test]
    async fn a_missing_surface_fails_closed() {
        let (policy, requests) = InteractiveApproval::new(4);
        drop(requests);
        match policy.decide(&request("shell")).await {
            ApprovalDecision::Deny { reason } => assert!(reason.contains("no approval surface")),
            other => panic!("expected a denial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn headless_approval_denies_without_retaining_argument_values() {
        let policy = HeadlessApproval::new();
        let decision = policy.decide(&request("shell")).await;

        assert!(!decision.is_allowed());
        assert_eq!(
            policy.required(),
            Some(ApprovalRequired {
                call_id: "call-1".into(),
                tool: "shell".into(),
                argument_keys: vec!["command".into()],
                mutates: true,
                requires_authorization: true,
            })
        );
        assert!(
            !format!("{:?}", policy.required()).contains("rm -rf"),
            "headless outcome retained a protected argument value"
        );
    }
}
