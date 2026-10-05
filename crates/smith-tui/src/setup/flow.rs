use super::*;

impl SetupApp {
    /// Reduces one setup key.
    pub fn on_key(&mut self, key: KeyEvent) -> SetupEffect {
        if key.kind == KeyEventKind::Release {
            return SetupEffect::None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return SetupEffect::Cancel;
        }
        if key.code == KeyCode::Esc || key.code == KeyCode::BackTab {
            if self.history.is_empty() {
                return if key.code == KeyCode::Esc {
                    SetupEffect::Cancel
                } else {
                    SetupEffect::None
                };
            }
            self.back();
            return SetupEffect::None;
        }
        if self.step == Step::Busy {
            return SetupEffect::None;
        }
        if self.step == Step::Review
            && matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
            )
        {
            let mut scroll = self.review_scroll.get();
            let page = scroll.page.max(1);
            scroll.offset = match key.code {
                KeyCode::Up => scroll.offset.saturating_sub(1),
                KeyCode::Down => scroll.offset.saturating_add(1).min(scroll.limit),
                KeyCode::PageUp => scroll.offset.saturating_sub(page),
                KeyCode::PageDown => scroll.offset.saturating_add(page).min(scroll.limit),
                _ => unreachable!("review scrolling keys were checked above"),
            };
            self.review_scroll.set(scroll);
            return SetupEffect::None;
        }
        self.error = None;

        if let Some(picker) = &mut self.picker {
            return match picker.on_key(key) {
                PickerOutcome::Pending => SetupEffect::None,
                PickerOutcome::Cancelled => SetupEffect::Cancel,
                PickerOutcome::Back => {
                    self.back();
                    SetupEffect::None
                }
                PickerOutcome::Selected(id) => self.select_picker(id),
            };
        }

        if key.code == KeyCode::Enter {
            return self.submit_input();
        }
        self.active_input_mut().on_key(key);
        SetupEffect::None
    }

    fn active_input_mut(&mut self) -> &mut crate::line_input::LineInput {
        if self.step == Step::CredentialValue
            && self
                .credential_method
                .is_some_and(CredentialMethod::takes_secret)
        {
            &mut self.secret.0
        } else {
            &mut self.input
        }
    }

    /// Folds one bracketed paste into the active text field.
    ///
    /// Pasting is how credentials usually arrive; without this, enabling
    /// bracketed paste would silently swallow them. Newlines and controls are
    /// dropped so a trailing newline cannot auto-submit a half-read form.
    pub fn on_paste(&mut self, text: &str) {
        if self.step == Step::Busy {
            return;
        }
        if let Some(picker) = &mut self.picker {
            picker.paste(text);
            return;
        }
        self.error = None;
        self.active_input_mut().paste(text);
    }

    pub(super) fn enter(&mut self, step: Step, remember: bool) {
        if remember {
            self.history.push(self.step);
        }
        self.step = step;
        self.review_scroll.set(ReviewScroll::default());
        self.input.clear();
        self.error = None;
        if step != Step::Review {
            self.collision_preview = None;
            self.allow_collisions = false;
        }
        self.configure_picker();
    }

    fn back(&mut self) {
        if let Some(step) = self.history.pop() {
            // The variable name is non-secret, so even an unsubmitted correction
            // must survive leaving its field and choosing the method again.
            if self.step == Step::CredentialValue
                && self.credential_method == Some(CredentialMethod::Environment)
            {
                self.environment_variable = self.input.text().to_owned();
            }
            self.step = step;
            self.review_scroll.set(ReviewScroll::default());
            self.input = match step {
                Step::ProviderName => self.provider.clone(),
                Step::Endpoint => self.endpoint.clone(),
                Step::ModelName => self.model.clone(),
                Step::CredentialValue
                    if self.credential_method == Some(CredentialMethod::Environment) =>
                {
                    self.environment_variable.clone()
                }
                // A context window already chosen by resolution is what Back
                // is there to edit, so it is what the only numeric field
                // shows.
                Step::ContextTokens => self
                    .context_tokens
                    .map_or_else(String::new, |value| value.to_string()),
                _ => String::new(),
            }
            .into();
            // Secret input is deliberately never restored by navigation. If
            // Back reaches authentication again, require a fresh key rather
            // than retaining the previously entered credential.
            if matches!(step, Step::CredentialMethod | Step::CredentialValue) {
                self.secret.clear();
            }
            self.error = None;
            // Leaving the review invalidates a collision approval: anything
            // edited on the way back must be re-reviewed before it can
            // replace existing values.
            self.collision_preview = None;
            self.allow_collisions = false;
            self.configure_picker();
        }
    }

    /// Resumes setup after a nested login backs out without dropping its selected action.
    pub fn back_from_chatgpt(&mut self) {
        self.busy_note = None;
        self.back();
    }

    pub(super) fn configure_picker(&mut self) {
        self.step_generation = self.step_generation.wrapping_add(1);
        self.picker = match self.step {
            Step::Action => {
                let entries = self
                    .provider_actions
                    .iter()
                    .map(|entry| ResourceEntry::new(&entry.id, &entry.label, &entry.detail))
                    .collect();
                Some(ResourcePicker::choices(
                    "Smith setup",
                    entries,
                    "No setup actions are available.",
                ))
            }
            Step::ProviderChoice => Some(ResourcePicker::new(
                "Choose provider",
                self.provider_entries.clone(),
                "No configured provider · go back to add one",
            )),
            Step::CredentialMethod => Some(ResourcePicker::choices(
                "Authentication",
                vec![
                    ResourceEntry::new(
                        "keychain",
                        "Store API key securely",
                        "macOS Keychain / Linux Secret Service",
                    ),
                    ResourceEntry::new(
                        "existing-keychain",
                        "Use existing secure entry",
                        format!("keychain:smith/{}", self.provider),
                    ),
                    ResourceEntry::new(
                        "config",
                        "Store in config (no prompts)",
                        "plaintext at rest · readable by same-user processes and backups",
                    ),
                    ResourceEntry::new(
                        "environment",
                        "Use environment variable",
                        "record a reference; Smith does not read or copy it now",
                    ),
                ],
                "Choose a credential method.",
            )),
            Step::ModelChoice => Some(ResourcePicker::new(
                "Choose default model",
                self.model_entries.clone(),
                "No selectable model · go back to add one",
            )),
            Step::ResponseBehavior => Some(ResourcePicker::choices(
                "Response compatibility",
                vec![
                    ResourceEntry::new(
                        "normal",
                        "Preserve response fields",
                        "recommended for ordinary OpenAI-compatible endpoints",
                    ),
                    ResourceEntry::new(
                        "reasoning-text",
                        "Reasoning-only success is visible text",
                        "for endpoints that put final answers only in reasoning_content",
                    ),
                ],
                "Choose response behavior.",
            )),
            Step::DefaultChoice => Some(ResourcePicker::choices(
                "Default selection",
                vec![
                    ResourceEntry::new("yes", "Make this the default", "used by plain `smith`"),
                    ResourceEntry::new(
                        "no",
                        "Keep current default",
                        "new choice remains selectable",
                    ),
                ],
                "Choose whether to change the default.",
            )),
            _ => None,
        };
        if let Some(picker) = self.picker.take() {
            let mut picker = picker.with_back(!self.history.is_empty());
            if let Some(id) = self.picker_selections.get(&self.step)
                && let Some(index) = picker.entries.iter().position(|entry| &entry.id == id)
            {
                picker.selected = index;
            }
            self.picker = Some(picker);
        }
    }

    pub(super) fn start_flow(&mut self, flow: SetupFlow, remember: bool) -> SetupEffect {
        match flow {
            SetupFlow::QuickKey {
                kind,
                provider,
                endpoint,
                review,
                catalog_models,
            } => {
                self.action = Some(match kind {
                    SetupQuickKey::Glm => SetupAction::QuickGlm,
                    SetupQuickKey::Xai => SetupAction::QuickXai,
                    SetupQuickKey::Google => SetupAction::QuickGoogle,
                });
                self.provider = provider;
                self.endpoint = endpoint;
                self.key_review = Some(review);
                self.catalog_models = catalog_models;
                if kind == SetupQuickKey::Glm {
                    self.model = self.quick_start.model.clone();
                    self.context_tokens = Some(self.quick_start.limits.context_tokens);
                    self.max_input_tokens = Some(self.quick_start.limits.max_input_tokens);
                    self.max_output_tokens = Some(self.quick_start.limits.max_output_tokens);
                    self.reasoning_only_text = true;
                    self.make_default = true;
                }
                self.enter(Step::CredentialMethod, remember);
            }
            SetupFlow::CustomEndpoint {
                kind,
                provider,
                endpoint,
                review_action,
                adapter,
                catalog_models,
            } => {
                self.action = Some(SetupAction::AddProvider);
                self.provider_kind = kind;
                self.review_action = review_action;
                self.adapter = adapter;
                self.catalog_models = catalog_models;
                if let Some(provider) = provider {
                    self.provider = provider;
                    self.endpoint = endpoint.unwrap_or_default();
                    if kind == SetupProviderKind::AnthropicMessages {
                        self.reasoning_only_text = false;
                    }
                    self.enter(Step::CredentialMethod, remember);
                } else {
                    self.enter(Step::ProviderName, remember);
                }
            }
            SetupFlow::OAuth { busy_note } => {
                self.enter(Step::Busy, remember);
                self.busy_note = Some(busy_note);
                return SetupEffect::ConnectChatGpt;
            }
            SetupFlow::AddModel => {
                self.action = Some(SetupAction::AddModel);
                self.enter(Step::ProviderChoice, remember);
            }
            SetupFlow::ChangeDefault => {
                self.action = Some(SetupAction::ChangeDefault);
                self.enter(Step::ModelChoice, remember);
            }
        }
        SetupEffect::None
    }

    pub(super) fn select_picker(&mut self, id: String) -> SetupEffect {
        self.picker_selections.insert(self.step, id.clone());
        match self.step {
            Step::Action => {
                let flow = self
                    .provider_actions
                    .iter()
                    .find(|entry| entry.id == id)
                    .expect("the picker only confirms offered setup entries")
                    .flow
                    .clone();
                return self.start_flow(flow, true);
            }
            Step::ProviderChoice => {
                self.provider = id;
                self.enter(Step::ModelName, true);
            }
            Step::CredentialMethod => match id.as_str() {
                "keychain" => {
                    self.credential_method = Some(CredentialMethod::Keychain);
                    self.secret.clear();
                    self.enter(Step::CredentialValue, true);
                }
                "existing-keychain" => {
                    self.secret.clear();
                    self.credential_method = Some(CredentialMethod::ExistingKeychain);
                    self.enter(self.after_credential_step(), true);
                }
                "config" => {
                    self.secret.clear();
                    self.credential_method = Some(CredentialMethod::Config);
                    self.enter(Step::CredentialValue, true);
                }
                "environment" => {
                    self.secret.clear();
                    self.credential_method = Some(CredentialMethod::Environment);
                    self.enter(Step::CredentialValue, true);
                    self.input.replace(self.environment_variable.clone());
                }
                _ => {}
            },
            Step::ModelChoice => {
                if self.catalog_models {
                    let Some(limits) = self.catalog_model_limits.get(&id).copied() else {
                        self.error =
                            Some("the selected catalog model has no enforceable limits".to_owned());
                        return SetupEffect::None;
                    };
                    self.model = id;
                    self.context_tokens = Some(limits.context_tokens);
                    self.max_input_tokens = Some(limits.max_input_tokens);
                    self.max_output_tokens = Some(limits.max_output_tokens);
                    self.enter(Step::DefaultChoice, true);
                } else if let Some((provider, model)) = id.split_once('/') {
                    self.provider = provider.to_owned();
                    self.model = model.to_owned();
                    self.enter(Step::Review, true);
                }
            }
            Step::ResponseBehavior => {
                self.reasoning_only_text = id == "reasoning-text";
                if matches!(self.mode, SetupMode::FirstRun) {
                    self.make_default = true;
                    self.enter(Step::Review, true);
                } else {
                    self.enter(Step::DefaultChoice, true);
                }
            }
            Step::DefaultChoice => {
                self.make_default = id == "yes";
                self.enter(Step::Review, true);
            }
            _ => {}
        }
        SetupEffect::None
    }

    pub(super) fn needs_response_behavior(&self) -> bool {
        self.action == Some(SetupAction::AddProvider)
            && self.provider_kind == SetupProviderKind::OpenAiCompatible
    }

    fn after_credential_step(&self) -> Step {
        if self.catalog_models {
            return Step::ModelChoice;
        }
        if matches!(
            self.action,
            Some(SetupAction::QuickGlm | SetupAction::ChangeCredential)
        ) {
            Step::Review
        } else {
            Step::ModelName
        }
    }

    fn submit_input(&mut self) -> SetupEffect {
        let value = self.input.text().trim().to_owned();
        match self.step {
            Step::ProviderName => {
                if value.is_empty()
                    || value.contains(['/', '\\'])
                    || value.chars().any(char::is_whitespace)
                {
                    self.error = Some(
                        "Use a non-empty provider name without spaces or path separators.".into(),
                    );
                } else {
                    self.provider = value;
                    self.enter(Step::Endpoint, true);
                }
            }
            Step::Endpoint => {
                if !(value.starts_with("https://") || value.starts_with("http://")) {
                    self.error = Some("Enter a complete http:// or https:// API base URL.".into());
                } else {
                    self.endpoint = value;
                    self.enter(Step::CredentialMethod, true);
                }
            }
            Step::CredentialValue => match self.credential_method {
                Some(method) if method.takes_secret() && self.secret.is_empty() => {
                    self.error =
                        Some("Enter an API key or go Back to choose another method.".into());
                }
                Some(method) if method.takes_secret() => {
                    self.enter(self.after_credential_step(), true);
                }
                Some(CredentialMethod::Environment) if !valid_variable(&value) => {
                    self.error = Some(self.prompts.environment_variable_error.clone());
                }
                Some(CredentialMethod::Environment) => {
                    self.environment_variable = value;
                    self.enter(self.after_credential_step(), true);
                }
                _ => {}
            },
            Step::ModelName => {
                if value.is_empty() || value.chars().any(char::is_control) {
                    self.error = Some("Enter the provider's exact model ID.".into());
                } else {
                    self.model = value;
                    if matches!(
                        self.action,
                        Some(SetupAction::AddProvider | SetupAction::AddModel)
                    ) {
                        // Custom models are the only ones whose limits nobody
                        // has reviewed yet, so they are the only ones worth a
                        // bounded read of the endpoint's own advertisement.
                        self.busy_note = Some(
                            "Resolving model limits from the endpoint and trusted catalog…".into(),
                        );
                        self.enter(Step::Busy, true);
                        return SetupEffect::ResolveModelLimits {
                            request: ResolveModelLimits {
                                use_endpoint_listing: self.action != Some(SetupAction::AddProvider)
                                    || self.provider_kind == SetupProviderKind::OpenAiCompatible,
                                endpoint: if self.action == Some(SetupAction::AddProvider)
                                    && self.provider_kind == SetupProviderKind::OpenAiCompatible
                                    && !self.endpoint.is_empty()
                                {
                                    Some(self.endpoint.clone())
                                } else {
                                    None
                                },
                                bearer: match self.credential_method {
                                    Some(method)
                                        if method.takes_secret() && !self.secret.is_empty() =>
                                    {
                                        Some(self.secret.secret())
                                    }
                                    _ => None,
                                },
                                environment_variable: if self.credential_method
                                    == Some(CredentialMethod::Environment)
                                {
                                    Some(self.environment_variable.clone())
                                } else {
                                    None
                                },
                                provider: (!self.provider.is_empty())
                                    .then(|| self.provider.clone()),
                                model: self.model.clone(),
                            },
                        };
                    }
                    self.enter(Step::ContextTokens, true);
                }
            }
            Step::ContextTokens => match positive_u32(&value) {
                Ok(value) => {
                    let unchanged_resolution =
                        self.context_tokens == Some(value) && self.limits_source.is_some();
                    self.context_tokens = Some(value);
                    if !unchanged_resolution {
                        self.max_input_tokens = Some(value);
                        self.max_output_tokens =
                            Some(smith_runtime::probe::derived_output_ceiling(value));
                        self.limits_source =
                            Some("context entered manually · ceilings derived".into());
                    }
                    if self.needs_response_behavior() {
                        self.enter(Step::ResponseBehavior, true);
                    } else {
                        self.enter(Step::DefaultChoice, true);
                    }
                }
                Err(error) => self.error = Some(error),
            },
            Step::Review => {
                let Some(submission) = self.submission() else {
                    self.error =
                        Some("Setup choices are incomplete; go Back and review them.".into());
                    return SetupEffect::None;
                };
                self.step = Step::Busy;
                self.step_generation = self.step_generation.wrapping_add(1);
                return SetupEffect::Submit {
                    submission,
                    allow_collisions: self.allow_collisions,
                };
            }
            _ => {}
        }
        SetupEffect::None
    }

    pub(super) fn submission(&self) -> Option<SetupSubmission> {
        let credential = || match self.credential_method? {
            CredentialMethod::Keychain => {
                Some(SetupCredential::StoreInKeychain(self.secret.secret()))
            }
            CredentialMethod::Config => Some(SetupCredential::StoreInConfig(self.secret.secret())),
            CredentialMethod::ExistingKeychain => Some(SetupCredential::ExistingKeychain),
            CredentialMethod::Environment => Some(SetupCredential::Environment(
                self.environment_variable.clone(),
            )),
        };
        let limits = || {
            Some(SetupModelLimits {
                context_tokens: self.context_tokens?,
                max_input_tokens: self.max_input_tokens?,
                max_output_tokens: self.max_output_tokens?,
            })
        };
        match self.action? {
            SetupAction::QuickGlm => Some(SetupSubmission::QuickGlm {
                credential: credential()?,
            }),
            SetupAction::QuickXai => Some(SetupSubmission::QuickXai {
                credential: credential()?,
                model: self.model.clone(),
            }),
            SetupAction::QuickGoogle => Some(SetupSubmission::QuickGoogle {
                credential: credential()?,
                model: self.model.clone(),
            }),
            SetupAction::AddProvider => Some(SetupSubmission::AddProvider {
                kind: self.provider_kind,
                provider: self.provider.clone(),
                endpoint: self.endpoint.clone(),
                credential: credential()?,
                model: self.model.clone(),
                limits: limits()?,
                reasoning_only_text: self.reasoning_only_text,
                make_default: self.make_default,
            }),
            SetupAction::AddModel => Some(SetupSubmission::AddModel {
                provider: self.provider.clone(),
                model: self.model.clone(),
                limits: limits()?,
                make_default: self.make_default,
            }),
            SetupAction::ChangeDefault => Some(SetupSubmission::ChangeDefault {
                provider: self.provider.clone(),
                model: self.model.clone(),
            }),
            SetupAction::ChangeCredential => Some(SetupSubmission::ChangeCredential {
                provider: self.provider.clone(),
                credential: credential()?,
            }),
        }
    }

    pub(super) fn prompt(&self) -> (&'static str, String, bool) {
        match self.step {
            Step::ProviderName => (
                "Provider name",
                self.prompts.provider_name_help.clone(),
                false,
            ),
            Step::Endpoint => (
                "API base URL",
                self.prompts.endpoint_help.clone(),
                false,
            ),
            Step::CredentialValue
                if self
                    .credential_method
                    .is_some_and(CredentialMethod::takes_secret) =>
            {
                (
                    "API key",
                    if self.credential_method == Some(CredentialMethod::Config) {
                        "Plaintext in owner-only config; readable by same-user processes and backups"
                    } else {
                        "Stored only in the platform credential service"
                    }
                    .to_owned(),
                    true,
                )
            }
            Step::CredentialValue => (
                "Environment variable",
                "Smith records the name only and does not read its value during setup".to_owned(),
                false,
            ),
            Step::ModelName => (
                "Model ID",
                "Exact identifier; limits resolve automatically when the endpoint or catalog publishes them"
                    .to_owned(),
                false,
            ),
            Step::ContextTokens => (
                "Model context window",
                match self.limits_source.as_deref() {
                    Some(source) => format!(
                        "Resolved from {source} · edit only to override; input/output ceilings follow automatically"
                    ),
                    None => {
                        "Not published by the endpoint or catalog · input/output ceilings are derived"
                            .to_owned()
                    }
                },
                false,
            ),
            _ => ("", String::new(), false),
        }
    }
}
