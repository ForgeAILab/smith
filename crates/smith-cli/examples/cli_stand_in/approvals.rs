//! In-process terminal host controlled by the contract test over stdio.
//!
//! The embedding host explicitly supplies the real Smith relay/supervisor.

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

use agent_runtime_core::approval::{ApprovalDecision, ApprovalPolicy, ApprovalRequest};
use agent_runtime_core::checkpoint::TurnState;
use agent_runtime_core::content::UserInput;
use agent_runtime_core::interaction::QuestionAnswer;
use anyhow::{Result, ensure};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{Value, json};
use smith_host::{
    InteractionNotice, InteractiveApproval, InteractiveInteraction, ProjectWorkspace, PromptScope,
};
use smith_runtime::checkpoint::{CheckpointKey, CheckpointKeyProvider, CheckpointProtectionError};
use smith_runtime::client::SmithEventKind;
use smith_runtime::factory::{HostSurface, RuntimeRequest};
use smith_runtime::host::{HostSessionRequest, start};
use smith_runtime::mcp::{McpOptions, McpSupervisor};
use tokio::sync::mpsc;

#[derive(Debug)]
struct Keys;
impl CheckpointKeyProvider for Keys {
    fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
        Ok(CheckpointKey::new([0x51; 32]))
    }
}

#[derive(Debug)]
struct ObservedApproval {
    inner: InteractiveApproval,
    withdrawn: mpsc::UnboundedSender<String>,
}

struct PendingDecision {
    tool: String,
    withdrawn: mpsc::UnboundedSender<String>,
    answered: bool,
}
impl Drop for PendingDecision {
    fn drop(&mut self) {
        if !self.answered {
            // InteractiveApproval keeps the decision receiver in this future.
            // Dropping it closes the prompt's responder: a late allow cannot
            // reach Runtime. This observes withdrawal through the approver.
            let _ = self.withdrawn.send(self.tool.clone());
        }
    }
}

#[async_trait]
impl ApprovalPolicy for ObservedApproval {
    async fn decide(&self, request: &ApprovalRequest) -> ApprovalDecision {
        let mut pending = PendingDecision {
            tool: request.prepared().tool().to_owned(),
            withdrawn: self.withdrawn.clone(),
            answered: false,
        };
        let decision = self.inner.decide(request).await;
        pending.answered = true;
        decision
    }
}

fn emit(value: Value) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &value)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

pub(crate) fn serve(args: &[String]) -> Result<()> {
    ensure!(args.len() == 3, "approval host needs project and home");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(25), run(&args[1], &args[2]))
            .await
            .map_err(|_| anyhow::anyhow!("interactive bridge host deadline exceeded"))?
    })
}

async fn run(project: &str, home: &str) -> Result<()> {
    let resolution = smith_config::resolve::resolve(
        &smith_config::resolve::ResolveRequest::new(project).with_home_dir(home),
    )?;
    let trust = smith_config::trust::TrustStore::open(std::path::Path::new(home).join(".smith"))?;
    let mcp = McpSupervisor::plan(&resolution.config.mcp, &trust, McpOptions::new(project))?;
    mcp.connect();
    mcp.settle(Duration::from_secs(5)).await;
    ensure!(mcp.settled(), "counting server startup deadline");
    let (inner, mut approvals) = InteractiveApproval::new(8);
    let (interaction, mut questions) = InteractiveInteraction::new();
    let (withdrawn, mut withdrawals) = mpsc::unbounded_channel();
    let request = RuntimeRequest {
        smith_executable: Some(
            std::env::var_os("CLI_STAND_IN_SMITH_BINARY")
                .ok_or_else(|| anyhow::anyhow!("approval host needs the Smith binary path"))?
                .into(),
        ),
        workspace: Some(Arc::new(ProjectWorkspace::new(project)?)),
        approval: Some(Arc::new(ObservedApproval { inner, withdrawn })),
        interaction: Some(Arc::new(interaction)),
        mcp: Some(mcp.clone()),
        ..RuntimeRequest::new(resolution.config, HostSurface::Terminal)
    };
    let host =
        start(HostSessionRequest::new(request, project).checkpoint_keys(Arc::new(Keys))).await?;
    let mut events = host.client().events();
    let (commands, mut input) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let value = line
                .map_err(anyhow::Error::from)
                .and_then(|line| serde_json::from_str::<Value>(&line).map_err(Into::into));
            if commands.send(value).is_err() {
                break;
            }
        }
    });
    let mut prompt = None;
    let mut question = None;
    let mut question_count = 0;
    let turn = host
        .session()
        .run(UserInput::text("interactive bridge contract"));
    tokio::pin!(turn);
    let settle = tokio::time::sleep(Duration::from_secs(25));
    tokio::pin!(settle);
    let mut settling = false;
    let mut terminal_seen = false;
    loop {
        tokio::select! {
            result = &mut turn => {
                result?;
                break;
            }
            event = events.next() => {
                let event = event.ok_or_else(|| anyhow::anyhow!("host event stream closed"))?;
                terminal_seen |= matches!(event.payload, SmithEventKind::TurnCompleted { .. });
                emit(json!({"type":"runtime_event", "event":event}))?;
            }
            next = approvals.recv() => {
                let next = next.ok_or_else(|| anyhow::anyhow!("approval broker closed"))?;
                emit(json!({"type":"approval", "tool":next.tool(), "resource":next.prepared().resource()}))?;
                ensure!(prompt.is_none(), "only one pending prompt expected");
                prompt = Some(next);
            }
            tool = withdrawals.recv() => {
                emit(json!({"type":"withdrawn", "tool":tool.expect("withdrawal channel")}))?;
            }
            notice = questions.recv() => {
                match notice.ok_or_else(|| anyhow::anyhow!("interaction broker closed"))? {
                    InteractionNotice::Present(next) => {
                        ensure!(question.is_none(), "only one pending questionnaire expected");
                        question_count += 1;
                        emit(json!({"type":"question", "request_id":next.request().id(),
                            "call":next.request().origin().call(),
                            "questions":next.request().questionnaire_payload().questions()}))?;
                        question = Some(next);
                    }
                    InteractionNotice::Close { request_id, outcome } => {
                        // Retain the handle so the test can deliberately try
                        // a late answer after Runtime withdrew this request.
                        emit(json!({"type":"question_withdrawn", "request_id":request_id,
                            "outcome":outcome}))?;
                    }
                }
            }
            command = input.recv() => {
                let command = command.ok_or_else(|| anyhow::anyhow!("approver input closed"))??;
                if command["allow"] == true {
                    prompt.take().ok_or_else(|| anyhow::anyhow!("no prompt to approve"))?.allow(PromptScope::Once);
                    emit(json!({"type":"answered"}))?;
                } else if let Some(answers) = command.get("answers") {
                    let answers: Vec<QuestionAnswer> = serde_json::from_value(answers.clone())?;
                    question.take().ok_or_else(|| anyhow::anyhow!("no questionnaire to answer"))?.answer(answers)?;
                    emit(json!({"type":"question_answered"}))?;
                } else if let Some(ms) = command["settle_ms"].as_u64() {
                    // A timer branch keeps driving the real session while
                    // waiting to observe any incorrectly delayed invocation.
                    settle.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(ms));
                    settling = true;
                } else {
                    anyhow::bail!("unknown approver command: {command}");
                }
            }
            _ = &mut settle, if settling => {
                settling = false;
                emit(json!({"type":"settled"}))?;
            }
        }
    }
    while !terminal_seen {
        let event = events
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing terminal event"))?;
        terminal_seen = matches!(event.payload, SmithEventKind::TurnCompleted { .. });
        emit(json!({"type":"runtime_event", "event":event}))?;
    }
    // Inspect the live session before shutdown can clear unfinished work.
    let checkpoint = host
        .runtime()
        .checkpoint_store()
        .ok_or_else(|| anyhow::anyhow!("missing protected checkpoint store"))?
        .load_latest(host.session().id())
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing terminal checkpoint"))?;
    let pending_interaction = matches!(checkpoint.state, TurnState::AwaitingInteraction { .. });
    host.shutdown().await?;
    mcp.shutdown().await;
    emit(
        json!({"type":"finished", "history":host.session().history(),
        "question_count":question_count, "pending_interaction":pending_interaction}),
    )
}
