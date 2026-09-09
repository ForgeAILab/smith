// resources behavior tests.

    #[test]
    fn a_home_relative_path_is_abbreviated() {
        let home = "/Users/example";
        assert_eq!(abbreviate("/Users/example/work/api", home), "~/work/api");
        assert_eq!(abbreviate("/Users/example", home), "~");
        assert_eq!(abbreviate("/opt/other", home), "/opt/other");
    }

    #[test]
    fn catalog_inventory_becomes_searchable_resource_metadata_with_disabled_reasons() {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
        std::fs::write(
            project.path().join(".smith/config.toml"),
            r#"
default_profile = "router"
[profiles.router]
provider = "openrouter"
model = "~openai/gpt-latest"
[providers.openrouter]
kind = "openai-compatible"
base_url = "https://openrouter.ai/api/v1"
credential = "env:OPENROUTER_API_KEY"
[context]
output_reserve = 4096
"#,
        )
        .expect("config");
        let resolution = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolution");
        let snapshot: smith_config::catalog::CatalogSnapshot =
            serde_json::from_str(smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED)
                .expect("embedded catalog");
        let inventory =
            local_inventory_with_catalog(&resolution, AVAILABLE_ADAPTER_KINDS, Some(&snapshot))
                .expect("catalog inventory");
        let selectable_count = inventory.providers[0].model_count;

        let resources = runtime_resources(
            inventory,
            Vec::new(),
            "session",
            project.path(),
            &resolution.config.agent,
            &smith_runtime::reasoning::ReasoningRuntimePolicy::default(),
            None,
            None,
        );
        // Derived from the embedded catalog rather than pinned to a literal:
        // the seed is regenerated whenever Models.dev is refreshed, and a
        // hard-coded total turns every refresh into a spurious failure.
        let catalogued = snapshot
            .providers
            .get("openrouter")
            .map(|provider| provider.models.len())
            .expect("the fixture's provider is in the embedded catalog");
        // Installed coding agents are offered alongside the catalog under
        // their own `cli/<agent>/<model>` namespace, so the list is the
        // catalog plus every agent model.
        let cli_models = smith_config::cli_agents::cli_model_ids();
        assert_eq!(resources.models.len(), catalogued + cli_models.len());
        assert!(catalogued > 0, "the embedded catalog lost its models");
        for id in &cli_models {
            assert!(
                resources.models.iter().any(|entry| entry.id == *id),
                "`{id}` is selectable without any configuration"
            );
        }
        assert_eq!(
            resources
                .main_profiles
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["router"]
        );
        assert!(resources.profiles.iter().any(|entry| {
            entry.label == "review" && entry.id == format!("{LEGACY_AGENT_PROFILE_PREFIX}review")
        }));
        assert!(
            resources.providers[0]
                .detail
                .contains(&format!("{selectable_count} models"))
        );
        let current = resources
            .models
            .iter()
            .find(|entry| entry.id == "openrouter/~openai/gpt-latest")
            .expect("nested catalog model");
        assert_eq!(current.label, "OpenAI GPT Latest");
        assert!(current.active);
        assert!(current.detail.contains("tools"), "{}", current.detail);
        assert!(current.detail.contains("advertised"), "{}", current.detail);
        assert!(
            current.detail.contains("output ceiling"),
            "{}",
            current.detail
        );
        assert!(
            current.detail.contains("request 32768 [automatic]"),
            "{}",
            current.detail
        );
        let incompatible = resources
            .models
            .iter()
            .find(|entry| entry.id == "openrouter/mancer/weaver")
            .expect("advertised incompatible model");
        assert!(
            incompatible
                .disabled_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("tool"))
        );
        assert!(resources.connections.iter().any(|entry| {
            entry.id == "chatgpt"
                && entry.detail.contains("Smith OAuth")
                && entry.detail.contains("direct ChatGPT Responses")
                && !entry.active
        }));
        assert!(resources.connections.iter().any(|entry| {
            entry.id == "google"
                && entry.detail.contains("AI Studio API key")
                && entry.detail.contains("native Gemini endpoint")
                && !entry.active
        }));
        assert!(!resources.providers.iter().any(|entry| entry.id == "chatgpt"));
        assert!(!resources.models.iter().any(|entry| entry.id.starts_with("chatgpt/")));
        assert!(!resources.disconnections.iter().any(|entry| entry.id == "chatgpt"));
    }

    #[test]
    fn configured_request_budgets_are_labeled_in_resource_metadata() {
        let budget = smith_config::output_budget::OutputBudget {
            request_tokens: 8_192,
            request_origin: smith_config::output_budget::OutputBudgetOrigin::Configured,
            output_reserve: 8_192,
            reserve_origin: smith_config::output_budget::OutputBudgetOrigin::Automatic,
        };

        assert_eq!(
            render_optional_output_budget(Some(&budget)),
            "8192 [configured]"
        );
    }

    #[test]
    fn grok_shaped_catalog_limits_are_selectable_unless_an_explicit_reserve_conflicts() {
        let snapshot: smith_config::catalog::CatalogSnapshot =
            serde_json::from_str(smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED)
                .expect("embedded catalog");
        let xai = snapshot
            .providers
            .get(smith_config::catalog::XAI_CATALOG_PROVIDER)
            .expect("the embedded catalog has xAI");
        let model = xai
            .models
            .values()
            .find(|model| {
                model.limits.as_ref().is_some_and(|limits| {
                    limits.context_tokens == 500_000 && limits.max_output_tokens == 500_000
                })
            })
            .expect("the embedded catalog retains a Grok-shaped limit fixture");
        let pair = format!("{}/{}", smith_config::setup::XAI_PROVIDER, model.id);

        let resolve_resources = |reserve: Option<u32>| {
            let home = tempfile::tempdir().expect("home");
            let project = tempfile::tempdir().expect("project");
            std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
            let reserve = reserve.map_or_else(String::new, |tokens| {
                format!("\n[context]\noutput_reserve = {tokens}\n")
            });
            std::fs::write(
                project.path().join(".smith/config.toml"),
                format!(
                    r#"
default_profile = "grok"

[profiles.grok]
provider = "{provider}"
model = "{model}"

[providers.{provider}]
kind = "{kind}"
base_url = "{endpoint}"
credential = "env:XAI_API_KEY"
{reserve}
"#,
                    provider = smith_config::setup::XAI_PROVIDER,
                    model = model.id,
                    kind = smith_config::model::KIND_XAI_RESPONSES,
                    endpoint = smith_config::setup::XAI_ENDPOINT,
                ),
            )
            .expect("config");
            let resolution =
                resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
                    .expect("resolution");
            let inventory = local_inventory_with_catalog(
                &resolution,
                AVAILABLE_ADAPTER_KINDS,
                Some(&snapshot),
            )
            .expect("catalog inventory");
            runtime_resources(
                inventory,
                Vec::new(),
                "session",
                project.path(),
                &resolution.config.agent,
                &smith_runtime::reasoning::ReasoningRuntimePolicy::default(),
                None,
                None,
            )
        };

        let resources = resolve_resources(None);
        let entry = resources
            .models
            .iter()
            .find(|entry| entry.id == pair)
            .expect("the Grok-shaped model is listed");
        assert!(entry.disabled_reason.is_none(), "{}", entry.detail);
        assert!(entry.detail.contains("output ceiling 500k"), "{}", entry.detail);
        assert!(
            entry.detail.contains("request 32768 [automatic]"),
            "{}",
            entry.detail
        );

        let resources = resolve_resources(Some(500_000));
        let entry = resources
            .models
            .iter()
            .find(|entry| entry.id == pair)
            .expect("the conflicting model remains visible");
        assert!(
            entry
                .disabled_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("leaves no input budget")),
            "{:?}",
            entry.disabled_reason
        );
    }
