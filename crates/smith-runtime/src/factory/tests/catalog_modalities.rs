use super::*;

#[tokio::test]
async fn chatgpt_catalog_modalities_leave_trusted_limits_and_capabilities_unchanged() {
    let snapshot: smith_config::catalog::CatalogSnapshot =
        serde_json::from_str(crate::model_catalog::EMBEDDED_MODELS_DEV_SEED).unwrap();
    for model in ["gpt-5.6-luna", "gpt-6.1-sol"] {
        let mut config = resolved_config();
        config.provider = provider(
            KIND_CHATGPT_RESPONSES,
            Some(smith_config::setup::CHATGPT_ENDPOINT),
        );
        config.provider.name = sourced("chatgpt".to_owned());
        config.provider.api_key = Some(sourced(agent_runtime_core::store::Secret::new(
            r#"{"schema":1,"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_at_ms":1,"account_id":"fixture-account"}"#,
        )));
        config.model = sourced(model.to_owned());
        let mut request = RuntimeRequest::new(config, HostSurface::Headless);
        let baseline = prepare_factory_inputs(&request)
            .await
            .expect("trusted baseline");
        request.model_catalog = Some(Arc::new(snapshot.clone()));
        let prepared = prepare_factory_inputs(&request)
            .await
            .expect("modality metadata");
        assert!(
            prepared
                .profile
                .profile
                .input_modalities
                .contains(&Modality::Image)
        );
        assert_eq!(
            prepared.profile.profile.limits,
            baseline.profile.profile.limits
        );
        assert_eq!(
            prepared.profile.contributions,
            baseline.profile.contributions
        );
        assert_eq!(
            prepared.profile.profile.capabilities,
            baseline.profile.profile.capabilities
        );
        let trusted = smith_config::setup::trusted_model("chatgpt", model).unwrap();
        assert_eq!(
            prepared.profile.profile.limits,
            ModelLimits::new(
                trusted.context_tokens,
                trusted.max_input_tokens,
                trusted.max_output_tokens,
            )
        );
        request.config.model_input_modalities = Some(Sourced::new(
            vec![smith_config::catalog::CatalogModality::Text],
            Source::file(
                Layer::ProjectFile,
                "/repo/.smith/config.toml",
                "input_modalities",
            ),
        ));
        let explicit = prepare_factory_inputs(&request)
            .await
            .expect("explicit text override");
        assert_eq!(explicit.profile.profile.input_modalities, [Modality::Text]);
    }
}

#[tokio::test]
async fn openai_responses_catalog_modalities_do_not_import_api_limits() {
    let mut config = resolved_config();
    config.provider = provider(
        KIND_OPENAI_RESPONSES,
        Some(smith_config::catalog::OPENAI_ENDPOINT),
    );
    config.model = sourced("gpt-5.6-luna".to_owned());
    config.model_limits.context_tokens = Some(sourced(32_768));
    config.model_limits.max_input_tokens = Some(sourced(28_672));
    config.model_limits.max_output_tokens = Some(sourced(4_096));
    let mut request = RuntimeRequest::new(config, HostSurface::Headless);
    let baseline = prepare_factory_inputs(&request)
        .await
        .expect("explicit baseline");
    request.model_catalog = Some(Arc::new(
        serde_json::from_str(crate::model_catalog::EMBEDDED_MODELS_DEV_SEED).unwrap(),
    ));
    let prepared = prepare_factory_inputs(&request)
        .await
        .expect("modality metadata");
    assert!(
        prepared
            .profile
            .profile
            .input_modalities
            .contains(&Modality::Image)
    );
    assert_eq!(
        prepared.profile.profile.limits,
        baseline.profile.profile.limits
    );
    assert_eq!(
        prepared.profile.contributions,
        baseline.profile.contributions
    );
    assert_eq!(
        prepared.profile.profile.capabilities,
        baseline.profile.profile.capabilities
    );
}
