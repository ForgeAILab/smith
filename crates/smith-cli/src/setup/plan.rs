use super::{
    AgentPosture, BTreeMap, ConfigFile, ConfigSecret, ContextSection, GLM_5_3, GLM_ENDPOINT,
    GLM_PROFILE, GLM_PROVIDER, GOOGLE_PROFILE, GOOGLE_PROVIDER, KIND_ANTHROPIC_MESSAGES,
    KIND_GEMINI_INTERACTIONS, KIND_OPENAI_COMPATIBLE, KIND_OPENAI_RESPONSES, ModelSection,
    PlannedCredential, ProfileSection, ProfileUse, ProviderResponseSection, ProviderSection,
    ReasoningOnlyBehavior, Result, SetupCredential, SetupModelLimits, SetupPlan, SetupProviderKind,
    SetupSubmission, XAI_ENDPOINT, XAI_PROFILE, XAI_PROVIDER, setup_environment_reference,
    setup_keychain_reference,
};

pub(super) fn setup_plan(submission: SetupSubmission) -> Result<SetupPlan> {
    match submission {
        SetupSubmission::QuickGlm { credential } => {
            let PlannedCredential {
                reference,
                api_key,
                enrollment_secret,
            } = credential_plan(GLM_PROVIDER, credential)?;
            let mut patch = ConfigFile {
                providers: BTreeMap::from([(
                    GLM_PROVIDER.into(),
                    ProviderSection {
                        kind: Some(KIND_OPENAI_COMPATIBLE.into()),
                        base_url: Some(GLM_ENDPOINT.into()),
                        credential: reference.as_ref().map(ToString::to_string),
                        api_key,
                        response: Some(ProviderResponseSection {
                            reasoning_only: Some(ReasoningOnlyBehavior::Text),
                        }),
                        ..ProviderSection::default()
                    },
                )]),
                models: BTreeMap::from([(
                    format!("{GLM_PROVIDER}/{}", GLM_5_3.model),
                    ModelSection {
                        context_tokens: Some(GLM_5_3.context_tokens),
                        max_input_tokens: Some(GLM_5_3.max_input_tokens),
                        max_output_tokens: Some(GLM_5_3.max_output_tokens),
                        ..ModelSection::default()
                    },
                )]),
                ..ConfigFile::default()
            };
            select_default(
                &mut patch,
                GLM_PROFILE,
                GLM_PROVIDER,
                GLM_5_3.model,
                GLM_5_3.request_output_tokens,
            );
            if let Some(profile) = patch.profiles.get_mut(GLM_PROFILE) {
                // The trusted catalog contributes the request budget with
                // provenance; setup should not bake a duplicate into the
                // user's profile.
                profile.max_output_tokens = None;
                profile.context = None;
            }
            Ok(SetupPlan {
                patch,
                credential_reference: reference,
                secret: enrollment_secret,
            })
        }
        SetupSubmission::QuickXai { credential, model } => {
            if model.trim().is_empty() {
                anyhow::bail!("xAI setup requires a model selected from the frozen catalog");
            }
            let PlannedCredential {
                reference,
                api_key,
                enrollment_secret,
            } = credential_plan(XAI_PROVIDER, credential)?;
            let mut patch = ConfigFile {
                providers: BTreeMap::from([(
                    XAI_PROVIDER.into(),
                    ProviderSection {
                        kind: Some(KIND_OPENAI_RESPONSES.into()),
                        // The endpoint is what binds this provider to the
                        // catalog entry that supplies its limits, so it is
                        // written rather than left to a default.
                        base_url: Some(XAI_ENDPOINT.into()),
                        credential: reference.as_ref().map(ToString::to_string),
                        api_key,
                        ..ProviderSection::default()
                    },
                )]),
                ..ConfigFile::default()
            };
            select_default(&mut patch, XAI_PROFILE, XAI_PROVIDER, &model, 8_192);
            if let Some(profile) = patch.profiles.get_mut(XAI_PROFILE) {
                // The frozen Models.dev snapshot owns the limits; the user
                // config stores no duplicate model metadata to drift from it.
                profile.max_output_tokens = None;
                profile.context = None;
            }
            Ok(SetupPlan {
                patch,
                credential_reference: reference,
                secret: enrollment_secret,
            })
        }
        SetupSubmission::QuickGoogle { credential, model } => {
            if model.trim().is_empty() {
                anyhow::bail!("Google setup requires a model selected from the frozen catalog");
            }
            let PlannedCredential {
                reference,
                api_key,
                enrollment_secret,
            } = credential_plan(GOOGLE_PROVIDER, credential)?;
            let mut patch = ConfigFile {
                providers: BTreeMap::from([(
                    GOOGLE_PROVIDER.into(),
                    ProviderSection {
                        kind: Some(KIND_GEMINI_INTERACTIONS.into()),
                        credential: reference.as_ref().map(ToString::to_string),
                        api_key,
                        ..ProviderSection::default()
                    },
                )]),
                ..ConfigFile::default()
            };
            select_default(&mut patch, GOOGLE_PROFILE, GOOGLE_PROVIDER, &model, 8_192);
            if let Some(profile) = patch.profiles.get_mut(GOOGLE_PROFILE) {
                // The frozen Models.dev snapshot supplied to preflight/runtime
                // owns the model limits; the user config stores no guessed
                // endpoint or duplicate model metadata.
                profile.max_output_tokens = None;
                profile.context = None;
            }
            Ok(SetupPlan {
                patch,
                credential_reference: reference,
                secret: enrollment_secret,
            })
        }
        SetupSubmission::AddProvider {
            kind,
            provider,
            endpoint,
            credential,
            model,
            limits,
            reasoning_only_text,
            make_default,
        } => {
            let PlannedCredential {
                reference,
                api_key,
                enrollment_secret,
            } = credential_plan(&provider, credential)?;
            let mut patch = ConfigFile {
                providers: BTreeMap::from([(
                    provider.clone(),
                    ProviderSection {
                        kind: Some(
                            match kind {
                                SetupProviderKind::OpenAiCompatible => KIND_OPENAI_COMPATIBLE,
                                SetupProviderKind::AnthropicMessages => KIND_ANTHROPIC_MESSAGES,
                            }
                            .into(),
                        ),
                        base_url: Some(endpoint),
                        credential: reference.as_ref().map(ToString::to_string),
                        api_key,
                        response: (kind == SetupProviderKind::OpenAiCompatible
                            && reasoning_only_text)
                            .then_some(ProviderResponseSection {
                                reasoning_only: Some(ReasoningOnlyBehavior::Text),
                            }),
                        ..ProviderSection::default()
                    },
                )]),
                models: BTreeMap::from([(format!("{provider}/{model}"), model_section(limits))]),
                ..ConfigFile::default()
            };
            if make_default {
                let profile = safe_profile_name(&provider, &model);
                select_default(
                    &mut patch,
                    &profile,
                    &provider,
                    &model,
                    limits.max_output_tokens.min(8_192),
                );
            }
            Ok(SetupPlan {
                patch,
                credential_reference: reference,
                secret: enrollment_secret,
            })
        }
        SetupSubmission::AddModel {
            provider,
            model,
            limits,
            make_default,
        } => {
            let mut patch = ConfigFile {
                models: BTreeMap::from([(format!("{provider}/{model}"), model_section(limits))]),
                ..ConfigFile::default()
            };
            if make_default {
                let profile = safe_profile_name(&provider, &model);
                select_default(
                    &mut patch,
                    &profile,
                    &provider,
                    &model,
                    limits.max_output_tokens.min(8_192),
                );
            }
            Ok(SetupPlan {
                patch,
                credential_reference: None,
                secret: None,
            })
        }
        SetupSubmission::ChangeDefault { provider, model } => {
            let profile = safe_profile_name(&provider, &model);
            let mut patch = ConfigFile::default();
            select_default(&mut patch, &profile, &provider, &model, 0);
            if let Some(profile) = patch.profiles.get_mut(&profile) {
                profile.max_output_tokens = None;
                profile.context = None;
            }
            Ok(SetupPlan {
                patch,
                credential_reference: None,
                secret: None,
            })
        }
        SetupSubmission::ChangeCredential {
            provider,
            credential,
        } => {
            let PlannedCredential {
                reference,
                api_key,
                enrollment_secret,
            } = credential_plan(&provider, credential)?;
            let patch = ConfigFile {
                providers: BTreeMap::from([(
                    provider,
                    ProviderSection {
                        credential: reference.as_ref().map(ToString::to_string),
                        api_key,
                        ..ProviderSection::default()
                    },
                )]),
                ..ConfigFile::default()
            };
            Ok(SetupPlan {
                patch,
                credential_reference: reference,
                secret: enrollment_secret,
            })
        }
    }
}

fn credential_plan(provider: &str, credential: SetupCredential) -> Result<PlannedCredential> {
    match credential {
        SetupCredential::StoreInKeychain(secret) => Ok(PlannedCredential {
            reference: Some(setup_keychain_reference(provider)?),
            api_key: None,
            enrollment_secret: Some(secret),
        }),
        SetupCredential::StoreInConfig(secret) => {
            if secret.expose().is_empty() {
                anyhow::bail!("the API key cannot be empty");
            }
            Ok(PlannedCredential {
                reference: None,
                api_key: Some(ConfigSecret::new(secret.expose())),
                enrollment_secret: None,
            })
        }
        SetupCredential::ExistingKeychain => Ok(PlannedCredential {
            reference: Some(setup_keychain_reference(provider)?),
            api_key: None,
            enrollment_secret: None,
        }),
        SetupCredential::Environment(variable) => Ok(PlannedCredential {
            reference: Some(setup_environment_reference(&variable)?),
            api_key: None,
            enrollment_secret: None,
        }),
    }
}

fn model_section(limits: SetupModelLimits) -> ModelSection {
    ModelSection {
        context_tokens: Some(limits.context_tokens),
        max_input_tokens: Some(limits.max_input_tokens),
        max_output_tokens: Some(limits.max_output_tokens),
        ..ModelSection::default()
    }
}

/// Adds the default coding, planning, and review profiles for a reviewed pair.
pub(crate) fn select_default(
    patch: &mut ConfigFile,
    profile: &str,
    provider: &str,
    model: &str,
    request_output_tokens: u32,
) {
    let plan_profile = profile_variant_name(profile, "plan");
    let review_profile = profile_variant_name(profile, "review");
    patch.default_profile = Some(profile.to_owned());
    patch.profile_order = Some(vec![
        profile.to_owned(),
        plan_profile.clone(),
        review_profile.clone(),
    ]);
    patch.profiles.insert(
        profile.to_owned(),
        ProfileSection {
            description: Some("default Smith coding agent".to_owned()),
            posture: Some(AgentPosture::Build),
            uses: Some(vec![ProfileUse::Main, ProfileUse::Child]),
            instructions: Some(
                "Implement the requested change, verify it, and report concrete evidence."
                    .to_owned(),
            ),
            provider: Some(provider.to_owned()),
            model: Some(model.to_owned()),
            max_output_tokens: Some(request_output_tokens),
            context: Some(ContextSection {
                output_reserve: Some(request_output_tokens),
                ..ContextSection::default()
            }),
            ..ProfileSection::default()
        },
    );
    patch.profiles.insert(
        plan_profile,
        ProfileSection {
            extends: Some(profile.to_owned()),
            description: Some("read-only implementation planning".to_owned()),
            posture: Some(AgentPosture::Plan),
            uses: Some(vec![ProfileUse::Main, ProfileUse::Child]),
            instructions: Some(
                "Inspect the repository and produce an implementation-ready plan without modifying the workspace."
                    .to_owned(),
            ),
            ..ProfileSection::default()
        },
    );
    patch.profiles.insert(
        review_profile,
        ProfileSection {
            extends: Some(profile.to_owned()),
            description: Some("read-only independent review".to_owned()),
            posture: Some(AgentPosture::Review),
            uses: Some(vec![ProfileUse::Main, ProfileUse::Child]),
            instructions: Some(
                "Report prioritized, evidence-backed findings without modifying the workspace."
                    .to_owned(),
            ),
            ..ProfileSection::default()
        },
    );
}

fn profile_variant_name(profile: &str, posture: &str) -> String {
    let suffix = format!("-{posture}");
    let keep = 32usize.saturating_sub(suffix.len());
    let mut base = profile.chars().take(keep).collect::<String>();
    base.push_str(&suffix);
    base
}

pub(super) fn safe_profile_name(provider: &str, model: &str) -> String {
    let mut name = format!("{provider}-{model}")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    name.truncate(32);
    let name = name.trim_matches('-');
    if name.is_empty() {
        "smith-model".to_owned()
    } else {
        name.to_owned()
    }
}
