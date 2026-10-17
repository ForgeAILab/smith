//! Tool-only model service seam; provider routing remains host-owned.
use agent_runtime_core::content::Role;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::usage::UsageDelta;
use std::{fmt, future::Future, pin::Pin};

/// Text-only input for one non-streaming, tool-free model request.
#[derive(Clone)]
pub struct ModuleModelRequest {
    /// Optional system text.
    pub system: Option<String>,
    /// At most 64 text messages with their roles.
    pub messages: Vec<(Role, String)>,
    /// Maximum output tokens requested by the module.
    pub max_output_tokens: u32,
}
impl ModuleModelRequest {
    /// Conservative input budget: one token per UTF-8 byte plus the ordinary
    /// request sizer's message framing, without a guest-supplied tokenizer.
    pub fn input_tokens_upper_bound(&self) -> u32 {
        use agent_runtime::context::{CharRatioSizer, RequestSizer};
        let framing = CharRatioSizer::new()
            .size_message(&agent_runtime_core::content::Message::user(String::new()));
        self.system
            .iter()
            .chain(self.messages.iter().map(|(_, text)| text))
            .fold(0u32, |total, text| {
                total
                    .saturating_add(u32::try_from(text.len()).unwrap_or(u32::MAX))
                    .saturating_add(framing)
            })
    }
}
impl fmt::Debug for ModuleModelRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleModelRequest")
            .field("max_output_tokens", &self.max_output_tokens)
            .finish_non_exhaustive()
    }
}

/// Complete response from the session's current model route.
#[derive(Clone)]
pub struct ModuleModelResponse {
    /// Response text.
    pub text: String,
    /// Provider finish reason, kept as bounded text for forward compatibility.
    pub finish_reason: String,
    /// Reported provider counters, before host attribution.
    pub usage: UsageDelta,
}
impl fmt::Debug for ModuleModelResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleModelResponse")
            .finish_non_exhaustive()
    }
}

/// Boxed model future with shared runtime errors.
pub type ModuleModelFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ModuleModelResponse, RuntimeError>> + Send + 'a>>;

/// Host-owned attribution and cancellation for a model call, never guest supplied.
#[derive(Debug, Clone)]
pub struct ModuleModelInvocation {
    /// Stable installed module id.
    pub module_id: String,
    /// Actual tool invocation owning this call.
    pub context: agent_runtime_core::tool::InvocationContext,
    /// One-based call number within this module's turn.
    pub sequence: u32,
}

/// Host-owned session model route, without guest-visible provider credentials.
pub trait ModuleModel: Send + Sync + fmt::Debug {
    /// Performs one non-streaming completion without tools.
    fn complete(
        &self,
        request: ModuleModelRequest,
        invocation: ModuleModelInvocation,
    ) -> ModuleModelFuture<'_>;
}
