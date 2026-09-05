//! Smith's revision-1 adapter for trusted local command model bridges.
//!
//! Agent Runtime owns process supervision. This module owns only the exact
//! external protocol: a narrow projection of one canonical request and a
//! strict decoder back into canonical provider events.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use agent_runtime::provider::command::{
    CommandAdapter, CommandAttempt, CommandOutputDecoder, CommandPreflight, CommandPreflightError,
    CommandProbe, CommandProvider,
};
use agent_runtime_core::content::{ContentPart, Message, Role};
use agent_runtime_core::provider::{
    AuthKind, Capabilities, FinishReason, ModelDescriptor, PromptCacheControl, Provider,
    ProviderAttemptPurpose, ProviderCallContext, ProviderError, ProviderErrorKind, ProviderRequest,
    ProviderStream, ProviderStreamEvent, ReasoningSupport, Sampling, ToolChoice,
};
use agent_runtime_core::usage::{CounterKind, UsageDelta};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable protocol name required on every probe, request, and output frame.
pub const COMMAND_PROTOCOL: &str = "smith-command-provider";
/// Exact schema revision supported by this adapter.
pub const COMMAND_SCHEMA_VERSION: u32 = 1;

const MAX_MODEL_BYTES: usize = 256;
const MAX_PROTOCOL_NAME_BYTES: usize = 256;
const MAX_TOOL_CALLS: usize = 1_024;
const MAX_TOOL_FIELD_BYTES: usize = 256;
const MAX_TOOL_ARGUMENT_BYTES: usize = 1024 * 1024;
const MAX_ERROR_MESSAGE_BYTES: usize = 512;
const MAX_IMPLEMENTATION_BYTES: usize = 64;

/// A Smith-owned adapter for one exact configured model.
#[derive(Clone)]
pub struct CommandJsonlAdapter {
    model: String,
}

impl CommandJsonlAdapter {
    /// Creates the adapter without touching the configured process.
    pub fn new(model: impl Into<String>) -> Result<Self, CommandAdapterConfigError> {
        let model = model.into();
        if model.is_empty() || model.len() > MAX_MODEL_BYTES || model.contains('\0') {
            return Err(CommandAdapterConfigError::InvalidModel);
        }
        Ok(Self { model })
    }

    /// Constructs the trait object consumed by [`agent_runtime::provider`].
    pub fn shared(self) -> Arc<dyn CommandAdapter> {
        Arc::new(self)
    }
}

impl fmt::Debug for CommandJsonlAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandJsonlAdapter")
            .field("model", &self.model)
            .field("protocol", &COMMAND_PROTOCOL)
            .field("schema_version", &COMMAND_SCHEMA_VERSION)
            .finish()
    }
}

/// Invalid static adapter configuration.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CommandAdapterConfigError {
    /// The selected model cannot safely participate in the fixed probe argv.
    #[error("command provider model id is empty or exceeds its protocol bound")]
    InvalidModel,
}

/// Boundary that removes Runtime's structural planning identity from ordinary
/// requests before they reach the revision-1 wire adapter.
///
/// Runtime creates this metadata for canonical context comparison even when a
/// provider declares cache unsupported. It is not provider cache behavior and
/// is never serialized here. Synthetic cache purposes remain untouched and
/// are rejected by the adapter before process I/O.
pub struct CommandProtocolProvider {
    inner: Arc<CommandProvider>,
}

impl CommandProtocolProvider {
    /// Wraps the already-probed process provider.
    pub fn new(inner: Arc<CommandProvider>) -> Self {
        Self { inner }
    }
}

impl fmt::Debug for CommandProtocolProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandProtocolProvider")
            .field("protocol", &COMMAND_PROTOCOL)
            .field("schema_version", &COMMAND_SCHEMA_VERSION)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl Provider for CommandProtocolProvider {
    fn describe(&self) -> Vec<ModelDescriptor> {
        self.inner.describe()
    }

    fn capabilities(&self, model: &agent_runtime_core::provider::ModelId) -> Option<Capabilities> {
        self.inner.capabilities(model)
    }

    async fn stream(
        &self,
        mut request: ProviderRequest,
        mut context: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        if context.purpose == ProviderAttemptPurpose::Ordinary {
            request.cache_identity = None;
            request.cache_boundary = None;
            context.cache_identity = None;
        }
        self.inner.stream(request, context).await
    }
}

impl CommandAdapter for CommandJsonlAdapter {
    fn describe(&self) -> Vec<ModelDescriptor> {
        vec![ModelDescriptor {
            id: agent_runtime_core::provider::ModelId::new(&self.model),
            display_name: self.model.clone(),
            vendor: "smith-command-provider".to_owned(),
            capabilities: Capabilities {
                streaming: true,
                tools: true,
                reasoning: ReasoningSupport::Unsupported,
                structured_output: false,
                usage: true,
                cache: false,
                prompt_cache: PromptCacheControl::None,
                cache_contract: None,
                auth: AuthKind::Custom("local_command".to_owned()),
                continuation: false,
                max_output_tokens: None,
            },
        }]
    }

    fn prepare(
        &self,
        request: &ProviderRequest,
        context: &ProviderCallContext,
    ) -> Result<CommandAttempt, ProviderError> {
        let envelope = AttemptEnvelope::project(request, context, &self.model)?;
        let mut stdin = serde_json::to_vec(&envelope).map_err(|_| {
            bad_request("command provider request could not be encoded within protocol revision 1")
        })?;
        stdin.push(b'\n');
        Ok(CommandAttempt::new(
            vec!["--smith-provider-attempt".to_owned()],
            stdin,
            Box::new(CommandJsonlDecoder::default()),
        ))
    }

    fn probe(&self) -> Option<CommandProbe> {
        Some(
            CommandProbe::new(vec![
                "--smith-provider-probe".to_owned(),
                self.model.clone(),
            ])
            .expect("the adapter constructor validated its fixed probe arguments"),
        )
    }

    fn parse_probe(&self, stdout: &[u8]) -> Result<CommandPreflight, CommandPreflightError> {
        let response: ProbeResponse =
            serde_json::from_slice(stdout).map_err(|_| CommandPreflightError::MalformedOutput)?;
        if response.protocol != COMMAND_PROTOCOL
            || response.schema_version != COMMAND_SCHEMA_VERSION
            || response.model != self.model
        {
            return CommandPreflight::incompatible(
                None,
                "bridge does not implement the selected Smith command-provider protocol",
            );
        }
        validate_implementation_metadata(&response.implementation)?;
        validate_implementation_metadata(&response.implementation_version)?;
        CommandPreflight::compatible(Some(format!(
            "{}/{}",
            response.implementation, response.implementation_version
        )))
    }
}

fn validate_implementation_metadata(value: &str) -> Result<(), CommandPreflightError> {
    if value.is_empty()
        || value.len() > MAX_IMPLEMENTATION_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._+".contains(&byte))
    {
        return Err(CommandPreflightError::MalformedOutput);
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeResponse {
    protocol: String,
    schema_version: u32,
    model: String,
    implementation: String,
    implementation_version: String,
}

#[derive(Serialize)]
struct AttemptEnvelope {
    protocol: &'static str,
    schema_version: u32,
    attempt_id: String,
    purpose: CommandPurpose,
    request: CommandRequest,
}

impl AttemptEnvelope {
    fn project(
        request: &ProviderRequest,
        context: &ProviderCallContext,
        selected_model: &str,
    ) -> Result<Self, ProviderError> {
        if request.model.as_str() != selected_model {
            return Err(bad_request(
                "command provider request model does not match the selected adapter model",
            ));
        }
        if request.cache_identity.is_some()
            || request.cache_boundary.is_some()
            || context.cache_identity.is_some()
        {
            return Err(unsupported(
                "command provider protocol revision 1 does not support provider cache state",
            ));
        }
        if request.reasoning.is_some() {
            return Err(unsupported(
                "command provider protocol revision 1 does not support reasoning",
            ));
        }
        if request.structured_output.is_some() {
            return Err(unsupported(
                "command provider protocol revision 1 does not support structured output",
            ));
        }
        if !request.vendor_extensions.is_null() {
            return Err(unsupported(
                "command provider protocol revision 1 does not support vendor extensions",
            ));
        }
        if context.purpose != ProviderAttemptPurpose::Ordinary {
            return Err(unsupported(
                "command provider protocol revision 1 does not support synthetic cache attempts",
            ));
        }
        if context.attempt_id.as_str().is_empty()
            || context.attempt_id.as_str().len() > MAX_PROTOCOL_NAME_BYTES
            || context.attempt_id.as_str().contains('\0')
        {
            return Err(bad_request(
                "command provider attempt id is outside the protocol bound",
            ));
        }

        let messages = request
            .messages
            .iter()
            .map(CommandMessage::project)
            .collect::<Result<Vec<_>, _>>()?;
        let tools = project_tools(request)?;
        let tool_choice = CommandToolChoice::project(&request.tool_choice, &tools)?;
        validate_sampling(&request.sampling)?;

        Ok(Self {
            protocol: COMMAND_PROTOCOL,
            schema_version: COMMAND_SCHEMA_VERSION,
            attempt_id: context.attempt_id.as_str().to_owned(),
            purpose: CommandPurpose::Ordinary,
            request: CommandRequest {
                model: request.model.as_str().to_owned(),
                messages,
                tools,
                tool_choice,
                sampling: CommandSampling {
                    temperature: request.sampling.temperature,
                    top_p: request.sampling.top_p,
                },
                max_output_tokens: request.max_output_tokens,
                stop: request.stop.clone(),
            },
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum CommandPurpose {
    Ordinary,
}

#[derive(Serialize)]
struct CommandRequest {
    model: String,
    messages: Vec<CommandMessage>,
    tools: Vec<CommandTool>,
    tool_choice: CommandToolChoice,
    sampling: CommandSampling,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<String>,
}

#[derive(Serialize)]
struct CommandMessage {
    role: CommandRole,
    content: Vec<CommandContentPart>,
}

impl CommandMessage {
    fn project(message: &Message) -> Result<Self, ProviderError> {
        if message.content.is_empty() {
            return Err(bad_request(
                "command provider protocol does not accept empty messages",
            ));
        }
        let role = CommandRole::from(message.role);
        let mut content = Vec::with_capacity(message.content.len());
        for part in &message.content {
            let projected = match (message.role, part) {
                (Role::System | Role::User | Role::Assistant, ContentPart::Text { text }) => {
                    CommandContentPart::Text { text: text.clone() }
                }
                (Role::Assistant, ContentPart::ToolCall(call)) => {
                    validate_tool_field(call.id.as_str(), "tool-call id")?;
                    validate_tool_field(&call.name, "tool name")?;
                    CommandContentPart::ToolCall {
                        id: call.id.as_str().to_owned(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    }
                }
                (Role::Tool, ContentPart::ToolResult(result)) => {
                    validate_tool_field(result.call_id.as_str(), "tool-result call id")?;
                    validate_tool_field(&result.name, "tool-result name")?;
                    let result_content = result
                        .content
                        .iter()
                        .map(|part| match part {
                            ContentPart::Text { text } => {
                                Ok(CommandResultContent::Text { text: text.clone() })
                            }
                            _ => Err(unsupported(
                                "command provider protocol revision 1 accepts only text tool results",
                            )),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    CommandContentPart::ToolResult {
                        call_id: result.call_id.as_str().to_owned(),
                        name: result.name.clone(),
                        content: result_content,
                        is_error: result.is_error,
                    }
                }
                (_, ContentPart::Reasoning { .. }) => {
                    return Err(unsupported(
                        "command provider protocol revision 1 does not accept reasoning content",
                    ));
                }
                (_, ContentPart::Image { .. }) => {
                    return Err(unsupported(
                        "command provider protocol revision 1 accepts text input only",
                    ));
                }
                _ => {
                    return Err(bad_request(
                        "command provider message role and content are inconsistent",
                    ));
                }
            };
            content.push(projected);
        }
        Ok(Self { role, content })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum CommandRole {
    System,
    User,
    Assistant,
    Tool,
}

impl From<Role> for CommandRole {
    fn from(value: Role) -> Self {
        match value {
            Role::System => Self::System,
            Role::User => Self::User,
            Role::Assistant => Self::Assistant,
            Role::Tool => Self::Tool,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CommandContentPart {
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        call_id: String,
        name: String,
        content: Vec<CommandResultContent>,
        is_error: bool,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CommandResultContent {
    Text { text: String },
}

#[derive(Serialize)]
struct CommandTool {
    name: String,
    description: String,
    input_schema: Value,
}

fn project_tools(request: &ProviderRequest) -> Result<Vec<CommandTool>, ProviderError> {
    let mut names = BTreeSet::new();
    request
        .tools
        .iter()
        .map(|tool| {
            validate_tool_field(&tool.name, "tool name")?;
            if !names.insert(tool.name.as_str()) {
                return Err(bad_request(
                    "command provider request contains duplicate tool names",
                ));
            }
            if !tool.input_schema.is_object() {
                return Err(bad_request(
                    "command provider tool input schema must be a JSON object",
                ));
            }
            Ok(CommandTool {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            })
        })
        .collect()
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CommandToolChoice {
    Auto,
    None,
    Required,
    Named { name: String },
}

impl CommandToolChoice {
    fn project(value: &ToolChoice, tools: &[CommandTool]) -> Result<Self, ProviderError> {
        match value {
            ToolChoice::Auto => Ok(Self::Auto),
            ToolChoice::None => Ok(Self::None),
            ToolChoice::Required if tools.is_empty() => Err(bad_request(
                "command provider cannot require a tool when none are advertised",
            )),
            ToolChoice::Required => Ok(Self::Required),
            ToolChoice::Named(name) => {
                validate_tool_field(name, "named tool choice")?;
                if !tools.iter().any(|tool| tool.name == *name) {
                    return Err(bad_request(
                        "command provider named tool choice is not advertised",
                    ));
                }
                Ok(Self::Named { name: name.clone() })
            }
        }
    }
}

#[derive(Serialize)]
struct CommandSampling {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
}

fn validate_sampling(sampling: &Sampling) -> Result<(), ProviderError> {
    if sampling.temperature.is_some_and(|value| !value.is_finite())
        || sampling.top_p.is_some_and(|value| !value.is_finite())
    {
        return Err(bad_request(
            "command provider sampling values must be finite",
        ));
    }
    Ok(())
}

fn validate_tool_field(value: &str, field: &str) -> Result<(), ProviderError> {
    if value.is_empty() || value.len() > MAX_TOOL_FIELD_BYTES || value.contains('\0') {
        return Err(bad_request(format!(
            "command provider {field} is outside the protocol bound"
        )));
    }
    Ok(())
}

/// Attempt-local strict JSONL decoder.
#[derive(Default)]
pub struct CommandJsonlDecoder {
    terminal: bool,
    usage_seen: bool,
    text_seen: bool,
    tool_calls: BTreeMap<u32, ToolFrameState>,
}

impl fmt::Debug for CommandJsonlDecoder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandJsonlDecoder")
            .field("terminal", &self.terminal)
            .field("usage_seen", &self.usage_seen)
            .field("text_seen", &self.text_seen)
            .field("tool_call_slots", &self.tool_calls.len())
            .finish()
    }
}

#[derive(Default)]
struct ToolFrameState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl CommandOutputDecoder for CommandJsonlDecoder {
    fn decode_frame(&mut self, frame: &[u8]) -> Result<Vec<ProviderStreamEvent>, ProviderError> {
        if self.terminal {
            return Err(malformed(
                "command bridge emitted data after its terminal frame",
            ));
        }
        let frame: StdoutFrame = serde_json::from_slice(frame)
            .map_err(|_| malformed("command bridge emitted a malformed revision-1 frame"))?;
        frame.validate_revision()?;

        let event = match frame {
            StdoutFrame::TextDelta { text, .. } => {
                if text.is_empty() {
                    return Err(malformed("command bridge emitted an empty text delta"));
                }
                self.text_seen = true;
                ProviderStreamEvent::TextDelta { text }
            }
            StdoutFrame::ToolCallDelta {
                index,
                id,
                name,
                arguments_fragment,
                ..
            } => {
                if usize::try_from(index).map_or(true, |index| index >= MAX_TOOL_CALLS) {
                    return Err(malformed(
                        "command bridge tool-call index exceeds the protocol bound",
                    ));
                }
                if let Some(id) = &id {
                    validate_tool_frame_field(id, "tool-call id")?;
                }
                if let Some(name) = &name {
                    validate_tool_frame_field(name, "tool name")?;
                }
                let state = self.tool_calls.entry(index).or_default();
                merge_tool_field(&mut state.id, id.as_deref(), "tool-call id")?;
                merge_tool_field(&mut state.name, name.as_deref(), "tool name")?;
                if state
                    .arguments
                    .len()
                    .saturating_add(arguments_fragment.len())
                    > MAX_TOOL_ARGUMENT_BYTES
                {
                    return Err(malformed(
                        "command bridge tool arguments exceed the protocol bound",
                    ));
                }
                state.arguments.push_str(&arguments_fragment);
                ProviderStreamEvent::ToolCallDelta {
                    index,
                    id,
                    name,
                    arguments_fragment,
                }
            }
            StdoutFrame::Usage {
                input_tokens,
                output_tokens,
                ..
            } => {
                if self.usage_seen {
                    return Err(malformed(
                        "command bridge emitted more than one usage frame",
                    ));
                }
                self.usage_seen = true;
                ProviderStreamEvent::Usage {
                    delta: UsageDelta::new()
                        .with(CounterKind::InputUncached, input_tokens)
                        .with(CounterKind::Output, output_tokens),
                }
            }
            StdoutFrame::Finish { reason, .. } => {
                if !self.usage_seen {
                    return Err(malformed("command bridge finished without required usage"));
                }
                let reason = FinishReason::from(reason);
                validate_finish(reason, self.text_seen, &self.tool_calls)?;
                self.terminal = true;
                ProviderStreamEvent::Finish { reason }
            }
            StdoutFrame::Error {
                kind,
                message,
                retryable,
                ..
            } => {
                validate_safe_error_message(&message)?;
                self.terminal = true;
                let kind = ProviderErrorKind::from(kind);
                let mut error = ProviderError::new(
                    kind,
                    format!(
                        "command bridge reported a {} error",
                        provider_error_kind_label(kind)
                    ),
                );
                if retryable {
                    error = error.retryable();
                }
                ProviderStreamEvent::Error { error }
            }
        };
        Ok(vec![event])
    }

    fn finish(&mut self) -> Result<Vec<ProviderStreamEvent>, ProviderError> {
        if !self.terminal {
            return Err(malformed(
                "command bridge stdout ended before a terminal frame",
            ));
        }
        Ok(Vec::new())
    }
}

fn validate_tool_frame_field(value: &str, field: &str) -> Result<(), ProviderError> {
    if value.is_empty() || value.len() > MAX_TOOL_FIELD_BYTES || value.chars().any(char::is_control)
    {
        return Err(malformed(format!(
            "command bridge {field} is outside the protocol bound"
        )));
    }
    Ok(())
}

fn merge_tool_field(
    slot: &mut Option<String>,
    incoming: Option<&str>,
    field: &str,
) -> Result<(), ProviderError> {
    let Some(incoming) = incoming else {
        return Ok(());
    };
    match slot {
        Some(existing) if existing != incoming => Err(malformed(format!(
            "command bridge emitted conflicting {field} fragments"
        ))),
        Some(_) => Ok(()),
        None => {
            *slot = Some(incoming.to_owned());
            Ok(())
        }
    }
}

fn validate_finish(
    reason: FinishReason,
    text_seen: bool,
    tool_calls: &BTreeMap<u32, ToolFrameState>,
) -> Result<(), ProviderError> {
    if reason == FinishReason::ToolCalls && tool_calls.is_empty() {
        return Err(malformed(
            "command bridge reported tool calls without emitting one",
        ));
    }
    if reason != FinishReason::ToolCalls && !tool_calls.is_empty() {
        return Err(malformed(
            "command bridge emitted tool calls with a non-tool finish reason",
        ));
    }
    if !text_seen && tool_calls.is_empty() && reason == FinishReason::Stop {
        return Err(malformed(
            "command bridge stopped without visible text or tool calls",
        ));
    }
    for state in tool_calls.values() {
        if state.id.is_none() || state.name.is_none() {
            return Err(malformed("command bridge emitted an incomplete tool call"));
        }
        let arguments: Value = serde_json::from_str(&state.arguments)
            .map_err(|_| malformed("command bridge emitted malformed tool arguments"))?;
        if !arguments.is_object() {
            return Err(malformed(
                "command bridge tool arguments must form a JSON object",
            ));
        }
    }
    Ok(())
}

fn validate_safe_error_message(message: &str) -> Result<(), ProviderError> {
    if message.is_empty()
        || message.len() > MAX_ERROR_MESSAGE_BYTES
        || message
            .chars()
            .any(|character| character.is_control() && character != '\t')
    {
        return Err(malformed(
            "command bridge error detail is outside the protocol bound",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum StdoutFrame {
    TextDelta {
        protocol: String,
        schema_version: u32,
        text: String,
    },
    ToolCallDelta {
        protocol: String,
        schema_version: u32,
        index: u32,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        name: Option<String>,
        arguments_fragment: String,
    },
    Usage {
        protocol: String,
        schema_version: u32,
        input_tokens: u64,
        output_tokens: u64,
    },
    Finish {
        protocol: String,
        schema_version: u32,
        reason: CommandFinishReason,
    },
    Error {
        protocol: String,
        schema_version: u32,
        kind: CommandErrorKind,
        message: String,
        retryable: bool,
    },
}

impl StdoutFrame {
    fn validate_revision(&self) -> Result<(), ProviderError> {
        let (protocol, schema_version) = match self {
            Self::TextDelta {
                protocol,
                schema_version,
                ..
            }
            | Self::ToolCallDelta {
                protocol,
                schema_version,
                ..
            }
            | Self::Usage {
                protocol,
                schema_version,
                ..
            }
            | Self::Finish {
                protocol,
                schema_version,
                ..
            }
            | Self::Error {
                protocol,
                schema_version,
                ..
            } => (protocol, schema_version),
        };
        if protocol != COMMAND_PROTOCOL || *schema_version != COMMAND_SCHEMA_VERSION {
            return Err(malformed(
                "command bridge emitted an incompatible protocol revision",
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum CommandFinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
}

impl From<CommandFinishReason> for FinishReason {
    fn from(value: CommandFinishReason) -> Self {
        match value {
            CommandFinishReason::Stop => Self::Stop,
            CommandFinishReason::ToolCalls => Self::ToolCalls,
            CommandFinishReason::Length => Self::Length,
            CommandFinishReason::ContentFilter => Self::ContentFilter,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum CommandErrorKind {
    Network,
    Timeout,
    RateLimited,
    Auth,
    BadRequest,
    MalformedStream,
    Server,
    Cancelled,
    Unsupported,
    LimitExhausted,
}

impl From<CommandErrorKind> for ProviderErrorKind {
    fn from(value: CommandErrorKind) -> Self {
        match value {
            CommandErrorKind::Network => Self::Network,
            CommandErrorKind::Timeout => Self::Timeout,
            CommandErrorKind::RateLimited => Self::RateLimited,
            CommandErrorKind::Auth => Self::Auth,
            CommandErrorKind::BadRequest => Self::BadRequest,
            CommandErrorKind::MalformedStream => Self::MalformedStream,
            CommandErrorKind::Server => Self::Server,
            CommandErrorKind::Cancelled => Self::Cancelled,
            CommandErrorKind::Unsupported => Self::Unsupported,
            CommandErrorKind::LimitExhausted => Self::LimitExhausted,
        }
    }
}

fn provider_error_kind_label(kind: ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::Network => "network",
        ProviderErrorKind::Timeout => "timeout",
        ProviderErrorKind::RateLimited => "rate-limited",
        ProviderErrorKind::Auth => "authentication",
        ProviderErrorKind::BadRequest => "bad-request",
        ProviderErrorKind::MalformedStream => "malformed-stream",
        ProviderErrorKind::Server => "server",
        ProviderErrorKind::Cancelled => "cancelled",
        ProviderErrorKind::Unsupported => "unsupported-capability",
        ProviderErrorKind::CacheExpired => "cache-expired",
        ProviderErrorKind::LimitExhausted => "limit-exhausted",
    }
}

fn bad_request(message: impl Into<String>) -> ProviderError {
    ProviderError::new(ProviderErrorKind::BadRequest, message)
}

fn unsupported(message: impl Into<String>) -> ProviderError {
    ProviderError::new(ProviderErrorKind::Unsupported, message)
}

fn malformed(message: impl Into<String>) -> ProviderError {
    ProviderError::new(ProviderErrorKind::MalformedStream, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::cancel::Cancellation;
    use agent_runtime_core::clock::Deadline;
    use agent_runtime_core::content::{ToolCall, ToolResultBlock};
    use agent_runtime_core::ids::{AttemptId, RequestId, SessionId, ToolCallId};
    use agent_runtime_core::provider::{
        ModelId, ReasoningConfig, StructuredOutputConfig, ToolSchema,
    };

    const MODEL: &str = "local-model";

    fn adapter() -> CommandJsonlAdapter {
        CommandJsonlAdapter::new(MODEL).expect("a bounded model")
    }

    fn context() -> ProviderCallContext {
        ProviderCallContext {
            session: SessionId::new("session-command"),
            request_id: RequestId::new("request-command"),
            attempt_id: AttemptId::new("attempt-command"),
            cache_identity: None,
            purpose: ProviderAttemptPurpose::Ordinary,
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
        }
    }

    fn request() -> ProviderRequest {
        ProviderRequest::new(ModelId::new(MODEL), vec![Message::user("hello")])
    }

    fn decode(
        decoder: &mut CommandJsonlDecoder,
        line: &str,
    ) -> Result<ProviderStreamEvent, ProviderError> {
        let events = decoder.decode_frame(line.as_bytes())?;
        assert_eq!(events.len(), 1);
        Ok(events.into_iter().next().expect("one decoded event"))
    }

    #[test]
    fn revision_one_capabilities_are_fixed_and_model_local() {
        let descriptors = adapter().describe();
        assert_eq!(descriptors.len(), 1);
        let descriptor = &descriptors[0];
        assert_eq!(descriptor.id.as_str(), MODEL);
        assert!(descriptor.capabilities.streaming);
        assert!(descriptor.capabilities.tools);
        assert!(descriptor.capabilities.usage);
        assert_eq!(
            descriptor.capabilities.reasoning,
            ReasoningSupport::Unsupported
        );
        assert!(!descriptor.capabilities.structured_output);
        assert!(!descriptor.capabilities.cache);
        assert_eq!(
            descriptor.capabilities.prompt_cache,
            PromptCacheControl::None
        );
        assert!(!descriptor.capabilities.continuation);
        assert_eq!(
            descriptor.capabilities.auth,
            AuthKind::Custom("local_command".to_owned())
        );
    }

    #[test]
    fn probe_contract_is_exact_bounded_and_redaction_safe() {
        let adapter = adapter();
        assert_eq!(
            adapter.probe().expect("a required probe").args(),
            ["--smith-provider-probe", MODEL]
        );
        let preflight = adapter
            .parse_probe(include_bytes!(
                "../tests/fixtures/command_provider/probe_success.json"
            ))
            .expect("a compatible probe");
        assert!(preflight.is_compatible());
        assert_eq!(preflight.version(), Some("fixture-bridge/1.2.3"));

        for invalid in [
            r#"{"protocol":"other","schema_version":1,"model":"local-model","implementation":"fixture","implementation_version":"1"}"#,
            r#"{"protocol":"smith-command-provider","schema_version":2,"model":"local-model","implementation":"fixture","implementation_version":"1"}"#,
            r#"{"protocol":"smith-command-provider","schema_version":1,"model":"other","implementation":"fixture","implementation_version":"1"}"#,
        ] {
            let result = adapter
                .parse_probe(invalid.as_bytes())
                .expect("a parsed incompatibility");
            assert!(!result.is_compatible());
            assert!(!result.detail().unwrap().contains("other"));
        }

        let unknown = r#"{"protocol":"smith-command-provider","schema_version":1,"model":"local-model","implementation":"fixture","implementation_version":"1","extra":true}"#;
        assert_eq!(
            adapter.parse_probe(unknown.as_bytes()),
            Err(CommandPreflightError::MalformedOutput)
        );
        let unsafe_metadata = r#"{"protocol":"smith-command-provider","schema_version":1,"model":"local-model","implementation":"bad metadata","implementation_version":"1"}"#;
        assert_eq!(
            adapter.parse_probe(unsafe_metadata.as_bytes()),
            Err(CommandPreflightError::MalformedOutput)
        );
    }

    #[test]
    fn canonical_text_tools_and_results_project_losslessly() {
        let call = ToolCall {
            id: ToolCallId::new("call-1"),
            name: "mcp__docs__lookup".to_owned(),
            arguments: serde_json::json!({"query": "smith"}),
        };
        let result = ToolResultBlock {
            call_id: call.id.clone(),
            name: call.name.clone(),
            content: vec![ContentPart::text("found")],
            is_error: false,
        };
        let mut request = ProviderRequest::new(
            ModelId::new(MODEL),
            vec![
                Message::system("system"),
                Message::user("question"),
                Message::assistant(vec![
                    ContentPart::text("checking"),
                    ContentPart::ToolCall(call),
                ]),
                Message::tool_result(result),
            ],
        );
        request.tools.push(ToolSchema {
            name: "mcp__docs__lookup".to_owned(),
            description: "Look up docs".to_owned(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            }),
        });
        request.tool_choice = ToolChoice::Named("mcp__docs__lookup".to_owned());
        request.sampling.temperature = Some(0.2);
        request.max_output_tokens = Some(512);
        request.stop = vec!["DONE".to_owned()];

        let envelope = AttemptEnvelope::project(&request, &context(), MODEL)
            .expect("a supported canonical request");
        let value = serde_json::to_value(envelope).expect("a JSON projection");
        assert_eq!(value["protocol"], COMMAND_PROTOCOL);
        assert_eq!(value["schema_version"], COMMAND_SCHEMA_VERSION);
        assert_eq!(value["attempt_id"], "attempt-command");
        assert_eq!(value["purpose"], "ordinary");
        assert_eq!(value["request"]["model"], MODEL);
        assert_eq!(
            value["request"]["messages"][2]["content"][1]["arguments"],
            serde_json::json!({"query": "smith"})
        );
        assert_eq!(
            value["request"]["messages"][3]["content"][0]["content"][0]["text"],
            "found"
        );
        assert_eq!(value["request"]["tools"][0]["name"], "mcp__docs__lookup");
        assert_eq!(
            value["request"]["tool_choice"],
            serde_json::json!({"type":"named", "name":"mcp__docs__lookup"})
        );
    }

    #[test]
    fn attempt_debug_and_argv_never_carry_request_content() {
        const PROMPT: &str = "prompt-must-stay-on-stdin";
        let request = ProviderRequest::new(ModelId::new(MODEL), vec![Message::user(PROMPT)]);
        let attempt = adapter()
            .prepare(&request, &context())
            .expect("a prepared attempt");
        let debug = format!("{attempt:?}");
        assert!(debug.contains("--smith-provider-attempt"));
        assert!(debug.contains("stdin_bytes"));
        assert!(!debug.contains(PROMPT));
    }

    #[test]
    fn unsupported_revision_one_request_features_fail_before_attempt_creation() {
        let unsupported_requests = [
            {
                let mut request = request();
                request.reasoning = Some(ReasoningConfig::default());
                request
            },
            {
                let mut request = request();
                request.structured_output = Some(StructuredOutputConfig {
                    schema: serde_json::json!({"type":"object"}),
                    name: None,
                });
                request
            },
            {
                let mut request = request();
                request.vendor_extensions = serde_json::json!({"vendor": true});
                request
            },
            ProviderRequest::new(
                ModelId::new(MODEL),
                vec![Message {
                    role: Role::User,
                    content: vec![ContentPart::Image {
                        url: "data:image/png;base64,AAAA".to_owned(),
                        detail: None,
                    }],
                }],
            ),
        ];

        for request in unsupported_requests {
            let error = adapter()
                .prepare(&request, &context())
                .expect_err("an unsupported revision-1 request");
            assert_eq!(error.kind, ProviderErrorKind::Unsupported);
        }

        let mut synthetic = context();
        synthetic.purpose = ProviderAttemptPurpose::CacheKeepalive;
        let error = adapter()
            .prepare(&request(), &synthetic)
            .expect_err("a synthetic cache attempt");
        assert_eq!(error.kind, ProviderErrorKind::Unsupported);
    }

    #[test]
    fn golden_text_stream_decodes_to_disjoint_usage_and_finish() {
        let mut decoder = CommandJsonlDecoder::default();
        let fixture = include_str!("../tests/fixtures/command_provider/text_success.jsonl");
        let events = fixture
            .lines()
            .map(|line| decode(&mut decoder, line).expect("a valid golden frame"))
            .collect::<Vec<_>>();
        assert!(matches!(
            &events[0],
            ProviderStreamEvent::TextDelta { text } if text == "hello"
        ));
        assert!(matches!(
            &events[1],
            ProviderStreamEvent::Usage { delta }
                if delta.get(CounterKind::InputUncached) == 7
                    && delta.get(CounterKind::Output) == 2
                    && delta.total() == 9
        ));
        assert!(matches!(
            events[2],
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop
            }
        ));
        assert!(decoder.finish().expect("a complete stream").is_empty());
    }

    #[test]
    fn tool_fragments_are_validated_and_preserved_for_runtime_assembly() {
        let mut decoder = CommandJsonlDecoder::default();
        let first = r#"{"protocol":"smith-command-provider","schema_version":1,"type":"tool_call_delta","index":0,"id":"call-1","name":"mcp__docs__lookup","arguments_fragment":"{\"query\":"}"#;
        let second = r#"{"protocol":"smith-command-provider","schema_version":1,"type":"tool_call_delta","index":0,"arguments_fragment":"\"smith\"}"}"#;
        let usage = r#"{"protocol":"smith-command-provider","schema_version":1,"type":"usage","input_tokens":8,"output_tokens":3}"#;
        let finish = r#"{"protocol":"smith-command-provider","schema_version":1,"type":"finish","reason":"tool_calls"}"#;
        assert!(matches!(
            decode(&mut decoder, first).unwrap(),
            ProviderStreamEvent::ToolCallDelta { index: 0, .. }
        ));
        assert!(matches!(
            decode(&mut decoder, second).unwrap(),
            ProviderStreamEvent::ToolCallDelta { index: 0, .. }
        ));
        decode(&mut decoder, usage).unwrap();
        assert!(matches!(
            decode(&mut decoder, finish).unwrap(),
            ProviderStreamEvent::Finish {
                reason: FinishReason::ToolCalls
            }
        ));
    }

    #[test]
    fn malformed_unknown_and_out_of_order_frames_fail_closed() {
        let bad_frames = [
            "not-json",
            r#"{"protocol":"smith-command-provider","schema_version":2,"type":"text_delta","text":"hello"}"#,
            r#"{"protocol":"smith-command-provider","schema_version":1,"type":"reasoning_delta","text":"hidden"}"#,
            r#"{"protocol":"smith-command-provider","schema_version":1,"type":"text_delta","text":"hello","extra":true}"#,
        ];
        for frame in bad_frames {
            let error = decode(&mut CommandJsonlDecoder::default(), frame)
                .expect_err("an unsupported or malformed frame");
            assert_eq!(error.kind, ProviderErrorKind::MalformedStream);
            assert!(!error.message.contains("hidden"));
        }

        let mut missing_usage = CommandJsonlDecoder::default();
        decode(
            &mut missing_usage,
            r#"{"protocol":"smith-command-provider","schema_version":1,"type":"text_delta","text":"hello"}"#,
        )
        .unwrap();
        let error = decode(
            &mut missing_usage,
            r#"{"protocol":"smith-command-provider","schema_version":1,"type":"finish","reason":"stop"}"#,
        )
        .expect_err("finish requires usage");
        assert_eq!(error.kind, ProviderErrorKind::MalformedStream);

        let mut duplicate_usage = CommandJsonlDecoder::default();
        let usage = r#"{"protocol":"smith-command-provider","schema_version":1,"type":"usage","input_tokens":1,"output_tokens":1}"#;
        decode(&mut duplicate_usage, usage).unwrap();
        assert!(decode(&mut duplicate_usage, usage).is_err());

        let mut terminal = CommandJsonlDecoder::default();
        decode(
            &mut terminal,
            r#"{"protocol":"smith-command-provider","schema_version":1,"type":"text_delta","text":"done"}"#,
        )
        .unwrap();
        decode(&mut terminal, usage).unwrap();
        decode(
            &mut terminal,
            r#"{"protocol":"smith-command-provider","schema_version":1,"type":"finish","reason":"stop"}"#,
        )
        .unwrap();
        let error = decode(
            &mut terminal,
            r#"{"protocol":"smith-command-provider","schema_version":1,"type":"text_delta","text":"late"}"#,
        )
        .expect_err("post-terminal data must fail");
        assert_eq!(error.kind, ProviderErrorKind::MalformedStream);

        let oversized = serde_json::json!({
            "protocol": COMMAND_PROTOCOL,
            "schema_version": COMMAND_SCHEMA_VERSION,
            "type": "tool_call_delta",
            "index": 0,
            "id": "call-1",
            "name": "lookup",
            "arguments_fragment": "x".repeat(MAX_TOOL_ARGUMENT_BYTES + 1),
        });
        let error = decode(
            &mut CommandJsonlDecoder::default(),
            &serde_json::to_string(&oversized).unwrap(),
        )
        .expect_err("oversized tool arguments must fail");
        assert_eq!(error.kind, ProviderErrorKind::MalformedStream);
    }

    #[test]
    fn every_revision_one_finish_and_error_classification_is_typed() {
        for (wire, expected) in [
            ("length", FinishReason::Length),
            ("content_filter", FinishReason::ContentFilter),
        ] {
            let mut decoder = CommandJsonlDecoder::default();
            decode(
                &mut decoder,
                r#"{"protocol":"smith-command-provider","schema_version":1,"type":"usage","input_tokens":1,"output_tokens":0}"#,
            )
            .unwrap();
            let frame = format!(
                r#"{{"protocol":"smith-command-provider","schema_version":1,"type":"finish","reason":"{wire}"}}"#
            );
            assert!(matches!(
                decode(&mut decoder, &frame).unwrap(),
                ProviderStreamEvent::Finish { reason } if reason == expected
            ));
        }

        for (wire, expected) in [
            ("network", ProviderErrorKind::Network),
            ("timeout", ProviderErrorKind::Timeout),
            ("rate_limited", ProviderErrorKind::RateLimited),
            ("auth", ProviderErrorKind::Auth),
            ("bad_request", ProviderErrorKind::BadRequest),
            ("malformed_stream", ProviderErrorKind::MalformedStream),
            ("server", ProviderErrorKind::Server),
            ("cancelled", ProviderErrorKind::Cancelled),
            ("unsupported", ProviderErrorKind::Unsupported),
            ("limit_exhausted", ProviderErrorKind::LimitExhausted),
        ] {
            let frame = format!(
                r#"{{"protocol":"smith-command-provider","schema_version":1,"type":"error","kind":"{wire}","message":"bounded","retryable":false}}"#
            );
            let ProviderStreamEvent::Error { error } =
                decode(&mut CommandJsonlDecoder::default(), &frame).unwrap()
            else {
                panic!("an error event")
            };
            assert_eq!(error.kind, expected);
        }
    }

    #[test]
    fn bridge_error_detail_is_validated_but_not_relayed() {
        const SECRET: &str = "bridge-secret-must-not-reach-diagnostics";
        let frame = format!(
            r#"{{"protocol":"smith-command-provider","schema_version":1,"type":"error","kind":"server","message":"{SECRET}","retryable":true}}"#
        );
        let event = decode(&mut CommandJsonlDecoder::default(), &frame).unwrap();
        let ProviderStreamEvent::Error { error } = event else {
            panic!("an error event");
        };
        assert_eq!(error.kind, ProviderErrorKind::Server);
        assert!(error.retryable);
        assert!(!error.message.contains(SECRET));
        assert!(!format!("{error:?}").contains(SECRET));
    }
}
