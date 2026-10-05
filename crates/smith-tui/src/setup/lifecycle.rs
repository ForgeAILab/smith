use super::*;

impl SetupApp {
    /// Creates setup with locally configured provider/model choices.
    pub fn new(
        mode: SetupMode,
        provider_entries: Vec<ResourceEntry>,
        model_entries: Vec<ResourceEntry>,
        quick_start: SetupQuickStart,
        provider_actions: Vec<SetupEntry>,
        prompts: SetupPrompts,
    ) -> Self {
        let mut app = Self {
            mode: mode.clone(),
            step: Step::Action,
            step_generation: 0,
            history: Vec::new(),
            picker_selections: BTreeMap::new(),
            picker: None,
            provider_actions,
            key_review: None,
            review_action: String::new(),
            adapter: String::new(),
            prompts,
            catalog_models: false,
            provider_entries,
            model_entries,
            catalog_model_limits: BTreeMap::new(),
            quick_start,
            action: None,
            provider_kind: SetupProviderKind::OpenAiCompatible,
            provider: String::new(),
            endpoint: String::new(),
            credential_method: None,
            environment_variable: String::new(),
            secret: MaskedInput::default(),
            model: String::new(),
            context_tokens: None,
            max_input_tokens: None,
            max_output_tokens: None,
            reasoning_only_text: false,
            make_default: true,
            input: crate::line_input::LineInput::default(),
            error: None,
            busy_note: None,
            limits_source: None,
            collision_preview: None,
            review_scroll: Cell::new(ReviewScroll::default()),
            allow_collisions: false,
            destination: "~/.smith/config.toml".into(),
            title: None,
        };
        match mode {
            SetupMode::FirstRun | SetupMode::Menu => app.enter(Step::Action, false),
            SetupMode::AddProvider => {
                let flow = app
                    .provider_actions
                    .iter()
                    .find_map(|entry| {
                        matches!(entry.flow, SetupFlow::CustomEndpoint { provider: None, .. })
                            .then(|| entry.flow.clone())
                    })
                    .expect("the CLI supplies the custom endpoint flow for add-provider");
                app.start_flow(flow, false);
            }
            SetupMode::Provider { flow } => {
                app.start_flow(flow, false);
            }
            SetupMode::AddModel {
                provider: Some(provider),
            } => {
                app.action = Some(SetupAction::AddModel);
                app.provider = provider;
                app.enter(Step::ModelName, false);
            }
            SetupMode::AddModel { provider: None } => {
                app.action = Some(SetupAction::AddModel);
                app.enter(Step::ProviderChoice, false);
            }
            SetupMode::Credential { provider } => {
                app.action = Some(SetupAction::ChangeCredential);
                app.provider = provider;
                app.enter(Step::CredentialMethod, false);
            }
        }
        app
    }

    /// Sets the exact user-scoped config destination shown during review.
    #[must_use]
    pub fn with_destination(mut self, destination: impl Into<String>) -> Self {
        self.destination = destination.into();
        self
    }

    /// Replaces offered actions with entries carrying executable flows.
    #[must_use]
    pub fn with_provider_actions(mut self, actions: Vec<SetupEntry>) -> Self {
        self.provider_actions = actions;
        self.configure_picker();
        self
    }

    /// Supplies reviewed catalog limits keyed by the exact model IDs shown by
    /// a built-in provider picker.
    #[must_use]
    pub fn with_catalog_model_limits(mut self, limits: BTreeMap<String, SetupModelLimits>) -> Self {
        self.catalog_model_limits = limits;
        self.configure_picker();
        self
    }

    /// Whether setup is waiting for an external persistence/preflight effect.
    pub fn is_busy(&self) -> bool {
        self.step == Step::Busy
    }

    /// Whether the initial setup action picker is still active.
    pub fn is_choosing_action(&self) -> bool {
        self.step == Step::Action
    }

    /// Why the busy step is busy, when a specific reason was recorded.
    pub fn busy_note(&self) -> Option<&str> {
        self.busy_note.as_deref()
    }

    /// Continues after the driver's bounded automatic limit resolution.
    ///
    /// `None` means no source knew the context window, so the user enters that
    /// one value. Smith derives both ceilings without presenting more numeric
    /// fields. Resolved limits skip numeric entry; Back still exposes the
    /// prefilled context window for an intentional override.
    pub fn apply_resolved_limits(&mut self, resolved: Option<ResolvedModelLimits>) {
        self.busy_note = None;
        // Restore the model step as the back target so a completed effect
        // cannot become an editable step when the user moves backward.
        self.step = self.history.pop().unwrap_or(Step::ModelName);
        match resolved {
            None => self.enter(Step::ContextTokens, true),
            Some(resolved) => {
                self.context_tokens = Some(resolved.context_tokens);
                self.max_input_tokens = Some(resolved.max_input_tokens);
                self.max_output_tokens = Some(resolved.max_output_tokens);
                self.limits_source = Some(resolved.source);
                let next = if self.needs_response_behavior() {
                    Step::ResponseBehavior
                } else {
                    Step::DefaultChoice
                };
                self.enter(next, true);
                self.history.push(Step::ContextTokens);
            }
        }
    }

    /// Returns setup to an actionable step with a bounded external error.
    pub fn fail(&mut self, message: impl Into<String>, authentication: bool) {
        self.error = Some(bound(message.into(), 1_024));
        self.review_scroll.set(ReviewScroll::default());
        self.step = if authentication {
            Step::CredentialMethod
        } else {
            Step::Review
        };
        self.configure_picker();
    }

    /// Shows the exact secret-safe merge preview and requires a second
    /// confirmation before replacing differing existing leaves.
    pub fn review_collisions(&mut self, preview: impl Into<String>) {
        self.collision_preview = Some(bound(preview.into(), 8_192));
        self.review_scroll.set(ReviewScroll::default());
        self.allow_collisions = true;
        self.error = Some(
            "Existing values differ. Review the additional lines, then press Enter again to replace only those values."
                .into(),
        );
        self.step = Step::Review;
        self.configure_picker();
    }
}
impl SetupApp {
    /// Connection callers supply their identity without creating another setup screen.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self.step_generation = self.step_generation.wrapping_add(1);
        self
    }
}
