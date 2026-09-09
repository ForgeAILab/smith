//! Product-owned request-output and context-reserve policy.
//!
//! Model catalogs describe hard provider ceilings. They do not decide how much
//! output an ordinary Smith turn should request or reserve by default.

/// Largest automatic output request Smith makes without an explicit setting.
pub const AUTOMATIC_REQUEST_OUTPUT_TOKEN_CAP: u32 = 32_768;

/// Where an effective request or reserve value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputBudgetOrigin {
    /// Smith derived the value from immutable model limits.
    Automatic,
    /// A resolved configuration or trusted product default supplied the value.
    Configured,
}

impl OutputBudgetOrigin {
    /// Stable short label for selection surfaces and diagnostics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Configured => "configured",
        }
    }
}

/// Effective output request and context reserve for one frozen model profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputBudget {
    /// Maximum output tokens sent on ordinary provider requests.
    pub request_tokens: u32,
    /// Whether the request maximum was derived or configured.
    pub request_origin: OutputBudgetOrigin,
    /// Tokens the context planner holds back from admitted input.
    pub output_reserve: u32,
    /// Whether the reserve was derived from the request or configured directly.
    pub reserve_origin: OutputBudgetOrigin,
}

/// Why an output budget cannot be used with a model profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OutputBudgetError {
    /// Provider requests must ask for at least one output token.
    #[error("request output budget must be greater than zero")]
    ZeroRequest,
    /// A configured request cannot exceed the immutable model ceiling.
    #[error(
        "request output budget {request_tokens} exceeds model output ceiling {model_max_output_tokens}"
    )]
    RequestExceedsModel {
        /// Configured request maximum.
        request_tokens: u32,
        /// Frozen model output ceiling.
        model_max_output_tokens: u32,
    },
    /// The reasoning reserve leaves no room for automatic output and input.
    #[error(
        "reasoning reserve {reasoning_reserve} leaves no space for automatic output and input in the {context_tokens}-token context window"
    )]
    NoAutomaticBudget {
        /// Frozen model context window.
        context_tokens: u32,
        /// Resolved reasoning reserve.
        reasoning_reserve: u32,
    },
    /// The effective reserves consume the complete context window.
    #[error(
        "output reserve {output_reserve} + reasoning reserve {reasoning_reserve} leaves no input budget in the {context_tokens}-token context window"
    )]
    NoInputBudget {
        /// Frozen model context window.
        context_tokens: u32,
        /// Effective output reserve.
        output_reserve: u32,
        /// Resolved reasoning reserve.
        reasoning_reserve: u32,
    },
}

/// Resolves the output request and default context reserve for one model.
///
/// Explicit values are authoritative and never clamped. Without an explicit
/// request, Smith derives a conservative product default bounded by the model
/// ceiling, a product-wide cap, one quarter of context, and the space remaining
/// after reasoning while retaining at least one input token.
pub fn resolve_output_budget(
    context_tokens: u32,
    model_max_output_tokens: u32,
    configured_request_tokens: Option<u32>,
    configured_output_reserve: Option<u32>,
    reasoning_reserve: u32,
) -> Result<OutputBudget, OutputBudgetError> {
    let (request_tokens, request_origin) = match configured_request_tokens {
        Some(0) => return Err(OutputBudgetError::ZeroRequest),
        Some(request_tokens) if request_tokens > model_max_output_tokens => {
            return Err(OutputBudgetError::RequestExceedsModel {
                request_tokens,
                model_max_output_tokens,
            });
        }
        Some(request_tokens) => (request_tokens, OutputBudgetOrigin::Configured),
        None => {
            let remaining = context_tokens
                .checked_sub(reasoning_reserve)
                .and_then(|tokens| tokens.checked_sub(1))
                .filter(|tokens| *tokens > 0)
                .ok_or(OutputBudgetError::NoAutomaticBudget {
                    context_tokens,
                    reasoning_reserve,
                })?;
            let quarter_context = (context_tokens / 4).max(1);
            let request_tokens = model_max_output_tokens
                .min(AUTOMATIC_REQUEST_OUTPUT_TOKEN_CAP)
                .min(quarter_context)
                .min(remaining);
            if request_tokens == 0 {
                return Err(OutputBudgetError::NoAutomaticBudget {
                    context_tokens,
                    reasoning_reserve,
                });
            }
            (request_tokens, OutputBudgetOrigin::Automatic)
        }
    };

    let (output_reserve, reserve_origin) = configured_output_reserve
        .map_or((request_tokens, request_origin), |reserve| {
            (reserve, OutputBudgetOrigin::Configured)
        });
    let input_boundary = context_tokens.saturating_sub(reasoning_reserve);
    if reasoning_reserve >= context_tokens || output_reserve >= input_boundary {
        return Err(OutputBudgetError::NoInputBudget {
            context_tokens,
            output_reserve,
            reasoning_reserve,
        });
    }

    Ok(OutputBudget {
        request_tokens,
        request_origin,
        output_reserve,
        reserve_origin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_equal_ceiling_uses_the_product_cap() {
        let budget =
            resolve_output_budget(500_000, 500_000, None, None, 0).expect("an automatic budget");

        assert_eq!(budget.request_tokens, 32_768);
        assert_eq!(budget.output_reserve, 32_768);
        assert_eq!(budget.request_origin, OutputBudgetOrigin::Automatic);
        assert_eq!(budget.reserve_origin, OutputBudgetOrigin::Automatic);
    }

    #[test]
    fn small_context_uses_the_quarter_window_bound() {
        let budget =
            resolve_output_budget(16_000, 16_000, None, None, 0).expect("an automatic budget");

        assert_eq!(budget.request_tokens, 4_000);
        assert_eq!(budget.output_reserve, 4_000);
    }

    #[test]
    fn smaller_model_ceiling_wins() {
        let budget =
            resolve_output_budget(128_000, 8_192, None, None, 0).expect("an automatic budget");

        assert_eq!(budget.request_tokens, 8_192);
        assert_eq!(budget.output_reserve, 8_192);
    }

    #[test]
    fn remaining_space_after_reasoning_is_a_checked_bound() {
        let budget = resolve_output_budget(1_000, 1_000, None, None, 998)
            .expect("one output and one input token remain");

        assert_eq!(budget.request_tokens, 1);
        assert_eq!(budget.output_reserve, 1);
        assert_eq!(
            resolve_output_budget(1_000, 1_000, None, None, 999),
            Err(OutputBudgetError::NoAutomaticBudget {
                context_tokens: 1_000,
                reasoning_reserve: 999,
            })
        );
        assert_eq!(
            resolve_output_budget(1, u32::MAX, None, None, u32::MAX),
            Err(OutputBudgetError::NoAutomaticBudget {
                context_tokens: 1,
                reasoning_reserve: u32::MAX,
            })
        );
    }

    #[test]
    fn explicit_request_and_reserve_keep_precedence() {
        let budget = resolve_output_budget(128_000, 64_000, Some(12_000), Some(16_000), 2_000)
            .expect("configured values fit");

        assert_eq!(budget.request_tokens, 12_000);
        assert_eq!(budget.output_reserve, 16_000);
        assert_eq!(budget.request_origin, OutputBudgetOrigin::Configured);
        assert_eq!(budget.reserve_origin, OutputBudgetOrigin::Configured);
    }

    #[test]
    fn explicit_request_becomes_the_default_reserve() {
        let budget = resolve_output_budget(128_000, 64_000, Some(12_000), None, 2_000)
            .expect("configured request fits");

        assert_eq!(budget.request_tokens, 12_000);
        assert_eq!(budget.output_reserve, 12_000);
        assert_eq!(budget.request_origin, OutputBudgetOrigin::Configured);
        assert_eq!(budget.reserve_origin, OutputBudgetOrigin::Configured);
    }

    #[test]
    fn invalid_explicit_values_are_not_clamped() {
        assert_eq!(
            resolve_output_budget(128_000, 64_000, Some(0), None, 0),
            Err(OutputBudgetError::ZeroRequest)
        );
        assert_eq!(
            resolve_output_budget(128_000, 64_000, Some(64_001), None, 0),
            Err(OutputBudgetError::RequestExceedsModel {
                request_tokens: 64_001,
                model_max_output_tokens: 64_000,
            })
        );
        assert_eq!(
            resolve_output_budget(128_000, 64_000, Some(4_096), Some(127_000), 1_000),
            Err(OutputBudgetError::NoInputBudget {
                context_tokens: 128_000,
                output_reserve: 127_000,
                reasoning_reserve: 1_000,
            })
        );
    }
}
