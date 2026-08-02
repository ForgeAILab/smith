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
        );
        assert_eq!(resources.models.len(), 335);
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
        assert!(!resources.providers.iter().any(|entry| entry.id == "chatgpt"));
        assert!(!resources.models.iter().any(|entry| entry.id.starts_with("chatgpt/")));
        assert!(!resources.disconnections.iter().any(|entry| entry.id == "chatgpt"));
    }
