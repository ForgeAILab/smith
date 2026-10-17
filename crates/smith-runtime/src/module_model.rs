//! Tool-only completions on the current session's host-owned provider route.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::cli_agent::TurnExecution;
use agent_runtime::agent::one_shot::{
    OneShotError, OneShotExecutor, OneShotFinish, OneShotRequest, run_one_shot,
};
use agent_runtime::harness::{
    ComponentDescriptor, TurnCommitHook, TurnCommitPatch, TurnCommitView,
};
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::content::Message;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::{AttemptId, RequestId, SessionId};
use agent_runtime_core::provider::{ModelId, Provider};
use agent_runtime_core::store::{SessionSnapshot, SessionStore};
use agent_runtime_core::usage::{Provenance, UsageDelta, UsageRecord, UsageSource};
use smith_module::{
    ModuleModel, ModuleModelFuture, ModuleModelInvocation, ModuleModelRequest, ModuleModelResponse,
};

/// Prefix identifying module-attributed provider work in the canonical ledger.
pub const MODULE_USAGE_PREFIX: &str = "module:";

/// The already-constructed session provider; credentials never leave this route.
#[derive(Debug)]
pub(crate) struct ModuleModelService {
    execution: TurnExecution,
    provider: Arc<dyn Provider>,
    model: ModelId,
    max_input_tokens: u32,
    pending: Mutex<BTreeMap<SessionId, Vec<UsageRecord>>>,
}

impl ModuleModelService {
    pub(crate) fn new(
        execution: TurnExecution,
        provider: Arc<dyn Provider>,
        model: ModelId,
        max_input_tokens: u32,
    ) -> Self {
        Self {
            execution,
            provider,
            model,
            max_input_tokens,
            pending: Mutex::default(),
        }
    }

    pub(crate) fn account_snapshot(&self, snapshot: &mut SessionSnapshot) {
        if let Some(records) = self
            .pending
            .lock()
            .expect("module usage lock poisoned")
            .get(&snapshot.id)
        {
            for record in records {
                if !snapshot.usage.records().contains(record) {
                    snapshot.usage.record(record.clone());
                }
            }
        }
    }

    pub(crate) fn accounting_store(
        self: &Arc<Self>,
        inner: Arc<dyn SessionStore>,
    ) -> Arc<dyn SessionStore> {
        Arc::new(ModuleModelStore {
            inner,
            model: self.clone(),
        })
    }
}

// Like advisor accounting, retain reported partial usage on cancellation or drop.
struct ModelUsage<'a> {
    service: &'a ModuleModelService,
    session: SessionId,
    record: UsageRecord,
}
impl Drop for ModelUsage<'_> {
    fn drop(&mut self) {
        self.service
            .pending
            .lock()
            .expect("module usage lock poisoned")
            .entry(self.session.clone())
            .or_default()
            .push(self.record.clone());
    }
}

impl ModuleModel for ModuleModelService {
    fn complete(
        &self,
        input: ModuleModelRequest,
        invocation: ModuleModelInvocation,
    ) -> ModuleModelFuture<'_> {
        Box::pin(async move {
            let ctx = invocation.context;
            let input_tokens = input.input_tokens_upper_bound();
            if input_tokens > self.max_input_tokens {
                return Err(RuntimeError::limit("module model input limit"));
            }
            if ctx.cancel.is_cancelled() {
                return Err(RuntimeError::cancelled("module model call cancelled"));
            }
            let executor = match &self.execution {
                TurnExecution::Provider => OneShotExecutor::Provider {
                    provider: self.provider.clone(),
                    model: self.model.clone(),
                },
                TurnExecution::InstalledAgent(plan) => OneShotExecutor::External(
                    plan.backend(std::path::PathBuf::from(ctx.workspace.root()), false),
                ),
                TurnExecution::MissingProgram { model, program, .. } => {
                    return Err(RuntimeError::tool(format!(
                        "module model `{model}` requires `{program}`, but `{program}` is not on PATH"
                    )));
                }
            };
            let messages = if matches!(self.execution, TurnExecution::InstalledAgent(_)) {
                // Installed-agent one-shot input is one user message. Preserve
                // roles and boundaries as data, without exposing provider state.
                vec![Message::user(
                    serde_json::to_string(&input.messages)
                        .map_err(|_| RuntimeError::tool("module model input encoding failed"))?,
                )]
            } else {
                input
                    .messages
                    .into_iter()
                    .map(|(role, text)| {
                        let mut message = Message::user(text);
                        message.role = role;
                        message
                    })
                    .collect()
            };
            let rendered_input = ModuleModelRequest {
                system: input.system.clone(),
                messages: messages
                    .iter()
                    .map(|message| (message.role, message.joined_text()))
                    .collect(),
                max_output_tokens: input.max_output_tokens,
            };
            if rendered_input.input_tokens_upper_bound() > self.max_input_tokens {
                return Err(RuntimeError::limit("module model input limit"));
            }
            // Opaque ids remain within the ABI ceiling even for long tool-call ids.
            let id = format!("module-{}-{}", uuid::Uuid::new_v4(), invocation.sequence);
            let request_id = RequestId::new(id.clone());
            let attempt_id = AttemptId::new(id);
            let mut usage = ModelUsage {
                service: self,
                session: ctx.session.clone(),
                record: UsageRecord {
                    // The pinned Runtime admits side-request hook usage through
                    // this channel, as it does for advisor; purpose owns attribution.
                    source: UsageSource::SemanticSummary,
                    provenance: Provenance {
                        request: Some(request_id.clone()),
                        attempt: Some(attempt_id.clone()),
                        tool_call: Some(ctx.call_id),
                        purpose: Some(format!("{MODULE_USAGE_PREFIX}{}", invocation.module_id)),
                        failed: true,
                        ..Provenance::default()
                    },
                    delta: UsageDelta::new(),
                },
            };
            let mut request = OneShotRequest::new(
                input.system.unwrap_or_default(),
                messages,
                ctx.session,
                request_id,
                attempt_id,
            );
            request.max_output_tokens = Some(input.max_output_tokens);
            // Collect one extra scalar so an oversized answer is refused rather
            // than silently accepted. The broker also enforces the ABI byte cap.
            request.output_char_limit = Some(1024 * 1024 + 1);
            request.cancel = ctx.cancel;
            request.deadline = invocation.deadline;
            request.clock = ctx.clock;
            let answer = run_one_shot(&executor, request, &mut usage.record.delta)
                .await
                .map_err(|error| match error {
                    OneShotError::Cancelled => {
                        RuntimeError::cancelled("module model call cancelled")
                    }
                    OneShotError::TimedOut => RuntimeError::tool("module model call timed out"),
                    OneShotError::ToolCall => {
                        RuntimeError::tool("module model attempted a tool call")
                    }
                    OneShotError::Filtered => {
                        RuntimeError::tool("module model response was filtered")
                    }
                    OneShotError::Empty => {
                        RuntimeError::tool("module model returned an empty answer")
                    }
                    OneShotError::Unfinished | OneShotError::UnsupportedInput(_) => {
                        RuntimeError::tool("module model did not finish normally")
                    }
                    OneShotError::Failed(error) => {
                        RuntimeError::new(error.kind, "module model request failed")
                    }
                })?;
            if answer.text.len() > 1024 * 1024 {
                return Err(RuntimeError::new(
                    agent_runtime_core::error::ErrorKind::Serialization,
                    "module model response limit",
                ));
            }
            let finish_reason = match answer.finish {
                OneShotFinish::Stopped => "stop",
                OneShotFinish::Truncated => "length",
            }
            .to_owned();
            usage.record.provenance.failed = false;
            Ok(ModuleModelResponse {
                text: answer.text,
                finish_reason,
                usage: usage.record.delta.clone(),
            })
        })
    }
}

#[async_trait::async_trait]
impl TurnCommitHook for ModuleModelService {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new("smith.module-model.usage", RegistryRevision::new("1"))
    }
    async fn after_commit(&self, view: &TurnCommitView) -> Result<TurnCommitPatch, RuntimeError> {
        Ok(TurnCommitPatch {
            usage: self
                .pending
                .lock()
                .expect("module usage lock poisoned")
                .remove(&view.session)
                .unwrap_or_default(),
            ..TurnCommitPatch::default()
        })
    }
}

#[derive(Debug)]
struct ModuleModelStore {
    inner: Arc<dyn SessionStore>,
    model: Arc<ModuleModelService>,
}
#[async_trait::async_trait]
impl SessionStore for ModuleModelStore {
    async fn load(&self, session: &SessionId) -> Result<Option<SessionSnapshot>, RuntimeError> {
        self.inner.load(session).await
    }
    async fn save(&self, snapshot: &SessionSnapshot) -> Result<(), RuntimeError> {
        let mut snapshot = snapshot.clone();
        self.model.account_snapshot(&mut snapshot);
        self.inner.save(&snapshot).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::provider::fake::{FakeProvider, ScriptedStream};
    use agent_runtime_core::cancel::Cancellation;
    use agent_runtime_core::clock::{Deadline, SystemClock};
    use agent_runtime_core::content::Role;
    use agent_runtime_core::ids::{ToolCallId, TurnId};
    use agent_runtime_core::provider::{
        Capabilities, ProviderError, ProviderErrorKind, ProviderStreamEvent,
    };
    use agent_runtime_core::tool::InvocationContext;
    use agent_runtime_core::usage::CounterKind;
    use agent_runtime_testkit::workspace::MemoryWorkspace;

    fn invocation(sequence: u32) -> ModuleModelInvocation {
        ModuleModelInvocation {
            module_id: "review/module".into(),
            sequence,
            deadline: Deadline::after(&SystemClock, 120_000),
            context: InvocationContext {
                session: SessionId::new("session"),
                turn: Some(TurnId::new("turn")),
                request: RequestId::new("request"),
                call_id: ToolCallId::new("tool-call"),
                workspace: Arc::new(MemoryWorkspace::new("/workspace")),
                clock: Arc::new(SystemClock),
                cancel: Cancellation::new(),
                // The guest deadline must never become the provider deadline.
                deadline: Deadline::after(&SystemClock, 1),
                output_limit: 10000,
            },
        }
    }
    fn request() -> ModuleModelRequest {
        ModuleModelRequest {
            system: None,
            messages: vec![(Role::User, "prompt".into())],
            max_output_tokens: 32,
        }
    }

    #[tokio::test]
    async fn partial_model_usage_is_retained_on_error_and_drop_without_exposing_secrets() {
        let delta = UsageDelta::new()
            .with(CounterKind::InputUncached, 11)
            .with(CounterKind::Output, 7);
        let provider = Arc::new(FakeProvider::new(
            "worker",
            Capabilities::basic_streaming(),
            vec![
                ScriptedStream::new(vec![
                    ProviderStreamEvent::Usage {
                        delta: delta.clone(),
                    },
                    ProviderStreamEvent::Error {
                        error: ProviderError::new(
                            ProviderErrorKind::Network,
                            "secret-provider-credential",
                        ),
                    },
                ]),
                ScriptedStream::blocking(vec![ProviderStreamEvent::Usage {
                    delta: delta.clone(),
                }]),
            ],
        ));
        let service = ModuleModelService::new(
            TurnExecution::Provider,
            provider.clone(),
            ModelId::new("worker"),
            32000,
        );
        let error = service
            .complete(request(), invocation(1))
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("secret-provider-credential"));
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(20),
            service.complete(request(), invocation(2)),
        )
        .await;
        assert!(result.is_err());
        let records = service.pending.lock().unwrap();
        let records = &records[&SessionId::new("session")];
        assert_eq!(records.len(), 2);
        for record in records {
            assert_eq!(record.delta, delta);
            assert_eq!(
                record.provenance.purpose.as_deref(),
                Some("module:review/module")
            );
            assert!(record.provenance.failed);
        }
        assert_ne!(records[0].provenance.attempt, records[1].provenance.attempt);
        assert_eq!(provider.requests().len(), 2);
    }

    #[tokio::test]
    async fn input_limit_refusal_makes_no_provider_attempt() {
        let provider = Arc::new(FakeProvider::new(
            "worker",
            Capabilities::basic_streaming(),
            vec![],
        ));
        let service = ModuleModelService::new(
            TurnExecution::Provider,
            provider.clone(),
            ModelId::new("worker"),
            1,
        );
        assert_eq!(
            service
                .complete(request(), invocation(1))
                .await
                .unwrap_err()
                .kind,
            agent_runtime_core::error::ErrorKind::Limit
        );
        assert!(provider.requests().is_empty());
        assert!(service.pending.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn one_shot_finish_errors_limits_and_usage_follow_the_shared_contract() {
        use agent_runtime_core::provider::{FinishReason, ToolChoice};
        for (finish, text, expected) in [
            (FinishReason::Stop, "answer", Ok("stop")),
            (FinishReason::Length, "partial answer", Ok("length")),
            (FinishReason::ContentFilter, "hidden", Err("filtered")),
            (FinishReason::ToolCalls, "answer", Err("tool call")),
            (FinishReason::Stop, "  ", Err("empty")),
            (FinishReason::Error, "answer", Err("finish normally")),
        ] {
            let delta = UsageDelta::new().with(CounterKind::Output, 3);
            let provider = Arc::new(FakeProvider::new(
                "worker",
                Capabilities::basic_streaming(),
                vec![ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta { text: text.into() },
                    ProviderStreamEvent::Usage {
                        delta: delta.clone(),
                    },
                    ProviderStreamEvent::Finish { reason: finish },
                ])],
            ));
            let service = ModuleModelService::new(
                TurnExecution::Provider,
                provider.clone(),
                ModelId::new("worker"),
                32000,
            );
            let invocation = invocation(1);
            let deadline = invocation.deadline;
            let result = service.complete(request(), invocation).await;
            match expected {
                Ok(finish) => assert_eq!(result.unwrap().finish_reason, finish),
                Err(reason) => assert!(result.unwrap_err().to_string().contains(reason)),
            }
            let requests = provider.requests();
            assert_eq!(requests[0].max_output_tokens, Some(32));
            assert_eq!(requests[0].tool_choice, ToolChoice::None);
            assert!(requests[0].tools.is_empty());
            assert_eq!(provider.calls()[0].deadline, deadline);
            assert_eq!(
                service.pending.lock().unwrap()[&SessionId::new("session")][0].delta,
                delta
            );
        }
    }

    #[tokio::test]
    async fn one_shot_uses_the_model_deadline_and_session_cancellation() {
        let provider = Arc::new(FakeProvider::new(
            "worker",
            Capabilities::basic_streaming(),
            vec![ScriptedStream::blocking(vec![])],
        ));
        let service = ModuleModelService::new(
            TurnExecution::Provider,
            provider,
            ModelId::new("worker"),
            32000,
        );
        let mut ctx = invocation(1);
        ctx.deadline = Deadline::after(&SystemClock, 20);
        assert!(
            service
                .complete(request(), ctx.clone())
                .await
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        assert!(!ctx.context.cancel.is_cancelled());
        ctx.context
            .cancel
            .cancel(agent_runtime_core::cancel::CancelReason::UserRequested);
        assert!(
            service
                .complete(request(), ctx)
                .await
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
    }

    #[tokio::test]
    async fn missing_installed_program_is_clear_and_does_not_fall_back_to_provider() {
        let provider = Arc::new(FakeProvider::text_reply("should not be called"));
        let service = ModuleModelService::new(
            TurnExecution::MissingProgram {
                model: "cli/codex/test".into(),
                kind: "codex".into(),
                program: "missing-codex".into(),
            },
            provider.clone(),
            ModelId::new("cli/codex/test"),
            32000,
        );
        let error = service
            .complete(request(), invocation(1))
            .await
            .unwrap_err();
        assert!(error.message.contains("missing-codex") && error.message.contains("not on PATH"));
        assert!(provider.requests().is_empty());
    }
}
