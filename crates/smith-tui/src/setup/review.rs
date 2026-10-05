use super::*;

impl SetupApp {
    /// Labels reviewed facts so color is never needed to distinguish what is written.
    pub fn review_lines(&self) -> Vec<String> {
        let rows = self.review_rows();
        let label_width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
        rows.into_iter()
            .map(|(label, value)| format!("{label:label_width$}  {value}"))
            .collect()
    }

    fn review_rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = match self.action {
            Some(SetupAction::QuickGlm) => vec![
                (
                    "Provider",
                    format!(
                        "{} · {}",
                        self.quick_start.provider, self.quick_start.endpoint
                    ),
                ),
                (
                    "API key",
                    self.credential_reference(&self.quick_start.provider),
                ),
                (
                    "Model",
                    format!(
                        "{} · {} · trusted catalog v{}",
                        self.quick_start.model,
                        compact_limits(self.quick_start.limits),
                        self.quick_start.catalog_revision,
                    ),
                ),
                (
                    "Requests",
                    format!(
                        "{} output · {} reserved",
                        compact_tokens(u64::from(self.quick_start.request_output_tokens)),
                        compact_tokens(u64::from(self.quick_start.output_reserve)),
                    ),
                ),
                (
                    "GLM replies",
                    "an answer sent only as reasoning is shown as the reply; thinking stays on"
                        .into(),
                ),
                ("Default", format!("profile {}", self.quick_start.profile)),
            ],
            Some(SetupAction::QuickXai | SetupAction::QuickGoogle) => {
                let review = self
                    .key_review
                    .as_ref()
                    .expect("a quick key flow supplies review text");
                let mut rows = vec![
                    ("Action", review_value(&review.action).to_owned()),
                    ("Provider", format!("{} · {}", self.provider, self.endpoint)),
                    (
                        "Connection",
                        if self.action == Some(SetupAction::QuickGoogle) {
                            "Gemini API"
                        } else {
                            "xAI API"
                        }
                        .into(),
                    ),
                    ("API key", self.credential_reference(&self.provider)),
                    (
                        "Model",
                        format!(
                            "{}/{} · {} · Models.dev frozen catalog",
                            self.provider,
                            self.model,
                            compact_limits(SetupModelLimits {
                                context_tokens: self.context_tokens.unwrap_or_default(),
                                max_input_tokens: self.max_input_tokens.unwrap_or_default(),
                                max_output_tokens: self.max_output_tokens.unwrap_or_default(),
                            })
                        ),
                    ),
                    (
                        "Requests",
                        "output and reserve derived from the selected catalog model".into(),
                    ),
                ];
                if let Some(reasoning) = &review.reasoning {
                    rows.push(("Thinking", review_value(reasoning).to_owned()));
                }
                rows.push((
                    "Default",
                    format!("profile {}", review_value(&review.profile)),
                ));
                rows
            }
            Some(SetupAction::AddProvider) => vec![
                ("Action", review_value(&self.review_action).to_owned()),
                (
                    "Connection",
                    match self.adapter.as_str() {
                        "anthropic-messages" => "Anthropic Messages API".into(),
                        "openai-compatible" => "OpenAI-compatible API".into(),
                        _ => self.adapter.clone(),
                    },
                ),
                ("Provider", format!("{} · {}", self.provider, self.endpoint)),
                ("API key", self.credential_reference(&self.provider)),
                (
                    "Model",
                    format!(
                        "{}/{} · {}",
                        self.provider,
                        self.model,
                        self.limits_review()
                    ),
                ),
                (
                    "Replies",
                    if self.reasoning_only_text {
                        "an answer sent only as reasoning is shown as the reply".into()
                    } else {
                        "keep the provider's answer and thinking separate".into()
                    },
                ),
                (
                    "Default",
                    if self.make_default {
                        "use this model"
                    } else {
                        "keep current selection"
                    }
                    .into(),
                ),
            ],
            Some(SetupAction::AddModel) => vec![
                ("Action", "Add model".into()),
                ("Provider", self.provider.clone()),
                (
                    "Model",
                    format!(
                        "{}/{} · {}",
                        self.provider,
                        self.model,
                        self.limits_review()
                    ),
                ),
                (
                    "Default",
                    if self.make_default {
                        "use this model"
                    } else {
                        "keep current selection"
                    }
                    .into(),
                ),
            ],
            Some(SetupAction::ChangeDefault) => vec![
                ("Action", "Change default model".into()),
                ("Default", format!("{}/{}", self.provider, self.model)),
            ],
            Some(SetupAction::ChangeCredential) => vec![
                ("Action", "Change provider credential".into()),
                ("Provider", self.provider.clone()),
                ("API key", self.credential_reference(&self.provider)),
            ],
            None => vec![("Action", "Choose how to connect a model.".into())],
        };
        if self.credential_method == Some(CredentialMethod::Config) {
            rows.push((
                "Warning",
                "plaintext at rest; same-user processes can read this key".into(),
            ));
            rows.push(("Backups", "may retain this key after rotation".into()));
        }
        if let Some(preview) = &self.collision_preview {
            rows.push(("Changes", "existing values to replace:".into()));
            rows.extend(preview.lines().map(|line| ("", line.to_owned())));
        }
        let home = std::env::var_os("HOME");
        let destination = review_destination(&self.destination, home.as_deref().map(Path::new));
        rows.push((
            "Writes",
            format!("{destination}, then checks the configuration"),
        ));
        rows
    }

    pub(super) fn wrapped_review_lines(&self, width: u16) -> Vec<Line<'static>> {
        let rows = self.review_rows();
        let label_width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
        let indent = label_width + 4;
        let value_width = width
            .saturating_sub(u16::try_from(indent).unwrap_or(u16::MAX))
            .max(1);
        rows.into_iter()
            .flat_map(|(label, value)| {
                // Collision previews retain their existing full-row wrapping.
                if label.is_empty() {
                    let mut lines = crate::render::wrap::wrap_lines(
                        &[Line::from(format!("{label:label_width$}  {value}"))],
                        width.saturating_sub(2),
                    );
                    for line in &mut lines {
                        line.spans.insert(0, Span::raw("  "));
                    }
                    return lines;
                }
                let mut lines = crate::render::wrap::wrap_lines(&[Line::from(value)], value_width);
                for (index, line) in lines.iter_mut().enumerate() {
                    let prefix = if index == 0 {
                        format!("  {label:label_width$}  ")
                    } else {
                        " ".repeat(indent)
                    };
                    line.spans.insert(0, Span::raw(prefix));
                }
                lines
            })
            .collect()
    }

    fn credential_reference(&self, provider: &str) -> String {
        match self.credential_method {
            Some(CredentialMethod::Environment) => {
                format!("env:{}", self.environment_variable)
            }
            Some(CredentialMethod::Config) => "api_key = [redacted]".to_owned(),
            _ => format!("keychain:smith/{provider}"),
        }
    }

    fn limits_review(&self) -> String {
        format!(
            "{} ({})",
            compact_limits(SetupModelLimits {
                context_tokens: self.context_tokens.unwrap_or_default(),
                max_input_tokens: self.max_input_tokens.unwrap_or_default(),
                max_output_tokens: self.max_output_tokens.unwrap_or_default(),
            }),
            self.limits_source.as_deref().unwrap_or("explicit limits")
        )
    }
}
