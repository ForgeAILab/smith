use super::{
    AVAILABLE_ADAPTER_KINDS, CatalogLoader, ConfigReadiness, Context, Path, PathBuf,
    ResolveRequest, Result, Selection, SetupContext, SetupMode, inspect,
};

pub(super) async fn setup_context(
    mut selection: Selection,
    mode: &SetupMode,
) -> Result<SetupContext> {
    let start = canonical_start(selection.project.as_deref())?;
    let request = ResolveRequest::new(&start)
        .with_known_modules(crate::modules::known_modules())
        .with_env(std::env::vars())
        .with_cli(selection.overrides());
    let (layout, resolution) = match inspect(&request) {
        ConfigReadiness::Ready(resolution) => (resolution.layout.clone(), Some(*resolution)),
        ConfigReadiness::Unconfigured(context) => (context.layout, None),
        ConfigReadiness::Invalid(error) => {
            return Err(anyhow::anyhow!("{error}"))
                .context("configuration is invalid; guided setup will not overwrite it");
        }
    };
    if matches!(mode, SetupMode::FirstRun) && resolution.is_some() {
        anyhow::bail!("Smith is already configured; run `smith setup` to add or change a choice");
    }
    let catalog = CatalogLoader::production(&layout.user_dir)
        .map_err(|error| anyhow::anyhow!(error))
        .context("preparing the provider model catalog")?
        // Setup consumes only the embedded/last-good reviewed snapshot. It
        // never refreshes Models.dev while a credential is being handled.
        .prepare(false)
        .await
        .map_err(|error| anyhow::anyhow!(error))
        .context("loading the provider model catalog")?
        .snapshot;
    let inventory = resolution
        .as_ref()
        .map(|resolution| {
            smith_config::inventory::local_inventory_with_catalog(
                resolution,
                AVAILABLE_ADAPTER_KINDS,
                Some(&catalog),
            )
        })
        .transpose()
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .unwrap_or_default();
    if let SetupMode::Credential { provider } = mode {
        let provider_entry = inventory
            .providers
            .iter()
            .find(|entry| entry.name == *provider)
            .ok_or_else(|| anyhow::anyhow!("provider `{provider}` is not configured"))?;
        if !provider_entry.selectable {
            anyhow::bail!(
                "provider `{provider}` has no selectable model to preflight; finish its endpoint \
                 and model setup first"
            );
        }
        let model = inventory
            .models
            .iter()
            .find(|entry| entry.provider == *provider && entry.active)
            .or_else(|| {
                inventory
                    .models
                    .iter()
                    .find(|entry| entry.provider == *provider)
            })
            .expect("a selectable provider has at least one selectable model");
        selection.provider = Some(provider.clone());
        selection.model = Some(model.model.clone());
    }
    let project = layout.project_root.clone().unwrap_or(start);
    Ok(SetupContext {
        selection,
        user_dir: layout.user_dir,
        project,
        inventory,
        catalog,
        unconfigured: resolution.is_none(),
    })
}

pub(super) fn canonical_start(project: Option<&Path>) -> Result<PathBuf> {
    let start = match project {
        Some(project) => project.to_path_buf(),
        None => std::env::current_dir().context("reading the current directory")?,
    };
    let start = start
        .canonicalize()
        .with_context(|| format!("resolving project path `{}`", start.display()))?;
    if !start.is_dir() {
        anyhow::bail!("project path `{}` is not a directory", start.display());
    }
    Ok(start)
}
