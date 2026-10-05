use super::{
    ApplyOutcome, ConfigReadiness, CredentialEnroller, CredentialEnrollmentError, CredentialRef,
    EffectCancellation, EnrollmentReceipt, FactoryError, HostSurface, ResolveRequest, Result,
    SetupContext, SetupSubmission, canonical_start, factory, inspect, prepare_user_config_edit,
    setup_keychain_reference, setup_plan,
};

pub(super) async fn apply_submission(
    context: &SetupContext,
    submission: SetupSubmission,
    allow_collisions: bool,
    cancellation: &EffectCancellation,
) -> ApplyOutcome {
    const CANCELLED: &str = "Setup cancelled · nothing was written";
    if cancellation.requested() {
        return ApplyOutcome::Cancelled;
    }
    let enroller = CredentialEnroller::new();
    let outcome =
        apply_submission_with(context, submission, allow_collisions, &enroller, || async {
            let result = preflight(context).await;
            if cancellation.requested() {
                Err((CANCELLED.to_owned(), false))
            } else {
                result
            }
        })
        .await;
    match outcome {
        ApplyOutcome::Failed { message, .. } if message == CANCELLED => ApplyOutcome::Cancelled,
        outcome => outcome,
    }
}

pub(super) async fn apply_submission_with<F, Fut>(
    context: &SetupContext,
    submission: SetupSubmission,
    allow_collisions: bool,
    enroller: &CredentialEnroller,
    run_preflight: F,
) -> ApplyOutcome
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), (String, bool)>>,
{
    let plan = match setup_plan(submission) {
        Ok(plan) => plan,
        Err(error) => {
            return ApplyOutcome::Failed {
                message: error.to_string(),
                authentication: false,
            };
        }
    };
    let prepared = match prepare_user_config_edit(&context.user_dir, &plan.patch) {
        Ok(prepared) => prepared,
        Err(error) => {
            return ApplyOutcome::Failed {
                message: error.to_string(),
                authentication: false,
            };
        }
    };
    if !allow_collisions && !prepared.collisions().is_empty() {
        return ApplyOutcome::Collision(prepared.preview());
    }

    let receipt = match enroll_if_needed(enroller, plan.credential_reference.as_ref(), plan.secret)
        .await
    {
        Ok(receipt) => receipt,
        Err(error) => {
            return ApplyOutcome::Failed {
                authentication: error.can_use_environment_instead(),
                message: format!(
                    "{error}. Choose the environment-variable option if protected storage is unavailable."
                ),
            };
        }
    };

    let committed = match prepared.commit(allow_collisions) {
        Ok(committed) => committed,
        Err(error) => {
            let cleanup = restore_enrollment(enroller, receipt);
            return ApplyOutcome::Failed {
                message: append_cleanup(error.to_string(), cleanup.err()),
                authentication: false,
            };
        }
    };

    match run_preflight().await {
        Ok(()) => {
            committed.accept();
            ApplyOutcome::Completed
        }
        Err((message, authentication)) => {
            let rollback = committed.rollback();
            let cleanup = restore_enrollment(enroller, receipt);
            let message = append_cleanup(append_cleanup(message, rollback.err()), cleanup.err());
            ApplyOutcome::Failed {
                message,
                authentication,
            }
        }
    }
}

async fn enroll_if_needed(
    enroller: &CredentialEnroller,
    reference: Option<&CredentialRef>,
    secret: Option<agent_runtime_core::store::Secret>,
) -> Result<Option<EnrollmentReceipt>, CredentialEnrollmentError> {
    let (Some(reference), Some(secret)) = (reference, secret) else {
        return Ok(None);
    };
    let enroller = enroller.clone();
    let reference = reference.clone();
    tokio::task::spawn_blocking(move || enroller.enroll(&reference, &secret))
        .await
        .map_err(|_| CredentialEnrollmentError::Backend {
            reference: reference_for_task_failure(),
            operation: smith_config::credential::EnrollmentOperation::Store,
            cause: smith_config::credential::KeychainError::Unavailable(
                "the credential task did not complete".into(),
            ),
        })?
        .map(Some)
}

fn reference_for_task_failure() -> CredentialRef {
    setup_keychain_reference("unknown").expect("the fixed reference is valid")
}

fn restore_enrollment(
    enroller: &CredentialEnroller,
    receipt: Option<EnrollmentReceipt>,
) -> Result<(), CredentialEnrollmentError> {
    match receipt {
        Some(receipt) => enroller.restore(receipt),
        None => Ok(()),
    }
}

fn append_cleanup(mut message: String, cleanup: Option<impl std::fmt::Display>) -> String {
    if let Some(cleanup) = cleanup {
        message.push_str(&format!("; rollback also reported: {cleanup}"));
    }
    message
}

async fn preflight(context: &SetupContext) -> Result<(), (String, bool)> {
    let start = canonical_start(context.selection.project.as_deref())
        .map_err(|error| (error.to_string(), false))?;
    let request = ResolveRequest::new(&start)
        .with_env(std::env::vars())
        .with_cli(context.selection.overrides());
    let resolution = match inspect(&request) {
        ConfigReadiness::Ready(resolution) => *resolution,
        ConfigReadiness::Unconfigured(_) => {
            return Err((
                "the reviewed edit still leaves Smith unconfigured; make the new pair the default"
                    .into(),
                false,
            ));
        }
        ConfigReadiness::Invalid(error) => return Err((error.to_string(), false)),
    };
    if let Err(error) =
        smith_runtime::host::validate_host_policy(&resolution.config, &context.project)
    {
        return Err((error.to_string(), false));
    }
    let runtime = crate::runtime_host::preflight_request(
        &resolution,
        &context.project,
        HostSurface::Terminal,
        Some(context.catalog.clone()),
    )
    .map_err(|error| (error.to_string(), false))?;
    factory::preflight(&runtime)
        .await
        .map(|_| ())
        .map_err(|error| {
            let authentication = matches!(
                error,
                FactoryError::Credential(_)
                    | FactoryError::CredentialReference { .. }
                    | FactoryError::CredentialTask
                    | FactoryError::CredentialTimeout { .. }
            );
            (error.to_string(), authentication)
        })
}
