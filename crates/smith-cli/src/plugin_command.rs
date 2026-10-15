//! Explicit plugin transactions: print the inspected snapshot before trusting it.

use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use anyhow::Result;
use smith_config::resolve::{ConfigReadiness, ResolveRequest, inspect, resolve_plugin_selection};
use smith_config::trust::{ExecutableKind, TrustDecision, TrustStore};
use smith_plugin::{PathVariables, PluginSource, PluginStore, PreparedPlugin, PreparedUpdate};

use crate::cli::{MarketplaceAction, PluginAction};
use crate::plugins::{
    PluginCatalog, inventory_components, mcp_target, open_store, record_trust, source_label,
};

pub(crate) fn run(action: PluginAction) -> Result<()> {
    let project = std::env::current_dir()?;
    let request = ResolveRequest::new(&project)
        .with_env(std::env::vars())
        .with_known_modules(crate::modules::known_modules());
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    run_with(
        action,
        request,
        terminal,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
    )
}

pub(crate) fn run_with(
    action: PluginAction,
    mut request: ResolveRequest,
    terminal: bool,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<()> {
    let user_root = match &request.home_dir {
        Some(home) => home.join(".smith"),
        None => smith_config::credential::user_state_root()?,
    };
    let project = request.start_dir.canonicalize()?;
    let store = open_store(&user_root);
    match action {
        PluginAction::Add { source, yes } => {
            let prepared = store.prepare_install(&PluginSource::parse(&source)?, None)?;
            let name = &prepared.inventory().manifest.name;
            if store
                .list()?
                .iter()
                .any(|installed| installed.name == *name)
            {
                update(
                    &store,
                    &user_root,
                    &project,
                    store.prepare_replacement(prepared)?,
                    yes,
                    terminal,
                    input,
                    output,
                )?;
            } else {
                show_inventory(&store, &project, &prepared, output)?;
                if confirm("Install and trust? [y/N]", yes, terminal, input, output)? {
                    let installed = store.commit_install(prepared)?;
                    record_trust(
                        &user_root,
                        &installed.name,
                        &installed.digest,
                        TrustDecision::Allow,
                    )?;
                    writeln!(output, "Installed and trusted `{}`", installed.name)?;
                }
            }
        }
        PluginAction::List => {
            if request.catalog.is_none() {
                request = request.with_catalog(
                    smith_runtime::model_catalog::CatalogLoader::production(&user_root)?
                        .snapshot()?,
                );
            }

            let catalog = PluginCatalog::discover(&user_root)?;
            request.known_plugins = catalog.known_plugins();
            let report = match inspect(&request) {
                ConfigReadiness::Ready(resolution) => {
                    catalog.resolve(&resolution.config, &project)?.report(
                        &user_root,
                        &resolution.config.plugins,
                        &resolution.config.unknown_plugins,
                        &project,
                    )?
                }
                ConfigReadiness::Unconfigured(_) => {
                    let selection = resolve_plugin_selection(&request)?;
                    catalog.selection_report(
                        &selection.plugins,
                        &selection.unknown_plugins,
                        &project,
                    )?
                }
                ConfigReadiness::Invalid(error) => return Err(error.into()),
            };
            writeln!(
                output,
                "{}",
                smith_client::plugins_report::render_plain(&report)
            )?;
        }
        PluginAction::Update { name, yes } => {
            let names = match name {
                Some(name) => vec![name],
                None => store
                    .list()?
                    .into_iter()
                    .map(|installed| installed.name)
                    .collect(),
            };
            if names.is_empty() {
                writeln!(output, "no plugins are installed")?;
            }
            for name in names {
                update(
                    &store,
                    &user_root,
                    &project,
                    store.prepare_update(&name)?,
                    yes,
                    terminal,
                    input,
                    output,
                )?;
            }
        }
        PluginAction::Remove { name, delete_data } => {
            let mut trust = TrustStore::open(&user_root)?;
            let installed = store.remove(&name, delete_data)?;
            trust.forget_executable(
                &user_root.join("plugins"),
                ExecutableKind::Extension,
                &installed.name,
            )?;
            writeln!(
                output,
                "Removed `{name}`; data {}",
                if delete_data { "deleted" } else { "kept" }
            )?;
        }
        PluginAction::Marketplace(action) => marketplace(&store, action, output)?,
    }
    Ok(())
}

fn confirm(
    prompt: &str,
    yes: bool,
    terminal: bool,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    anyhow::ensure!(
        terminal,
        "plugin installation and trust require a terminal confirmation; pass --yes to confirm the printed inventory without a terminal"
    );
    write!(output, "{prompt} ")?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    let accepted = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    if !accepted {
        writeln!(output, "Cancelled; installed content and trust unchanged")?;
    }
    Ok(accepted)
}

fn show_inventory(
    store: &PluginStore,
    project: &Path,
    prepared: &PreparedPlugin,
    output: &mut impl Write,
) -> Result<()> {
    let inventory = prepared.inventory();
    let name = &inventory.manifest.name;
    let root = store.install_directory(name, &inventory.digest)?;
    let data = store.data_directory(name)?;
    writeln!(
        output,
        "Plugin {name} · version {}",
        inventory
            .manifest
            .version
            .as_deref()
            .unwrap_or("unspecified")
    )?;
    writeln!(output, "source: {}", source_label(prepared.source()))?;
    writeln!(
        output,
        "resolved commit: {}",
        prepared.resolved_commit().unwrap_or("none (local source)")
    )?;
    let (loaded, not_loaded) = inventory_components(
        inventory,
        PathVariables {
            plugin_root: &root,
            plugin_data: &data,
            project_dir: project,
        },
    );
    for (label, components) in [("will load", loaded), ("will not load", not_loaded)] {
        for component in components {
            writeln!(
                output,
                "  {label}: {} {}{}",
                component.kind,
                component.name,
                component
                    .detail
                    .map_or_else(String::new, |detail| format!(" · {detail}"))
            )?;
        }
    }
    writeln!(output, "digest: {}", inventory.digest)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn update(
    store: &PluginStore,
    user_root: &Path,
    project: &Path,
    update: PreparedUpdate,
    yes: bool,
    terminal: bool,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<()> {
    let unchanged = update.installed.digest == update.prepared.inventory().digest;
    // An unchanged source can still restore locally edited files or explicitly
    // approve a bundle whose previous installation has no allow decision.
    if unchanged
        && !update.discards_local_changes
        && crate::plugins::trust_status(
            user_root,
            &update.installed.name,
            &update.installed.digest,
        )?
        .allows_execution()
    {
        writeln!(output, "{}: Already up to date", update.installed.name)?;
        return Ok(());
    }
    show_inventory(store, project, &update.prepared, output)?;
    if let Some(reason) = &update.previous_failure {
        writeln!(
            output,
            "Installed inventory unavailable: {reason}; the fetched inventory replaces it on confirmation."
        )?;
    }
    if update.discards_local_changes {
        writeln!(
            output,
            "Local edits to the installed plugin will be discarded on confirmation."
        )?;
    }
    let diff = &update.difference;
    for (kind, label, names) in [
        ("skill", "added", &diff.added_skills),
        ("skill", "removed", &diff.removed_skills),
        ("skill", "changed", &diff.changed_skills),
        ("command", "added", &diff.added_commands),
        ("command", "removed", &diff.removed_commands),
        ("command", "changed", &diff.changed_commands),
    ] {
        for name in names {
            writeln!(output, "  {label}: {kind} {name}")?;
        }
    }
    for (label, modules) in [
        ("added", &diff.added_modules),
        ("removed", &diff.removed_modules),
    ] {
        for module in modules {
            writeln!(
                output,
                "  {label} sandboxed module {}: {}",
                module.id,
                module.capabilities.labels().join(", ")
            )?;
        }
    }
    for change in &diff.changed_modules {
        let added = change
            .new
            .capabilities
            .labels()
            .into_iter()
            .filter(|label| !change.old.capabilities.labels().contains(label))
            .collect::<Vec<_>>();
        writeln!(
            output,
            "  changed sandboxed module {}: old [{}]; new [{}]; newly requested capabilities [{}] require confirmation",
            change.name,
            change.old.capabilities.labels().join(", "),
            change.new.capabilities.labels().join(", "),
            added.join(", ")
        )?;
    }
    let data = store.data_directory(&update.installed.name)?;
    let root =
        store.install_directory(&update.installed.name, &update.prepared.inventory().digest)?;
    let old_paths = PathVariables {
        plugin_root: &update.installed.install_directory,
        plugin_data: &data,
        project_dir: project,
    };
    let new_paths = PathVariables {
        plugin_root: &root,
        plugin_data: &data,
        project_dir: project,
    };
    for server in &diff.added_mcp_servers {
        writeln!(
            output,
            "  added MCP {}: {}",
            server.name,
            mcp_target(&server.resolve(new_paths))
        )?;
    }
    for server in &diff.removed_mcp_servers {
        writeln!(
            output,
            "  removed MCP {}: {}",
            server.name,
            mcp_target(&server.resolve(old_paths))
        )?;
    }
    for change in &diff.changed_mcp_servers {
        writeln!(
            output,
            "  changed MCP {}:\n    old: {}\n    new: {}",
            change.name,
            mcp_target(&change.old.resolve(old_paths)),
            mcp_target(&change.new.resolve(new_paths))
        )?;
    }
    for (label, components) in [
        ("added not loaded", &diff.added_not_loaded),
        ("removed not loaded", &diff.removed_not_loaded),
    ] {
        for component in components {
            writeln!(
                output,
                "  {label}: {} {} · {}",
                component.kind, component.name, component.reason
            )?;
        }
    }
    writeln!(
        output,
        "old digest: {}\nnew digest: {}",
        update.installed.digest,
        update.prepared.inventory().digest
    )?;
    if confirm("Update and trust? [y/N]", yes, terminal, input, output)? {
        let installed = store.commit_update(update)?;
        record_trust(
            user_root,
            &installed.name,
            &installed.digest,
            TrustDecision::Allow,
        )?;
        writeln!(output, "Updated and trusted `{}`", installed.name)?;
    }
    Ok(())
}

fn marketplace(
    store: &PluginStore,
    action: MarketplaceAction,
    output: &mut impl Write,
) -> Result<()> {
    match action {
        MarketplaceAction::Add(source) => {
            let record = store.add_marketplace(&PluginSource::parse(&source)?)?;
            writeln!(
                output,
                "Registered marketplace `{}` · {}",
                record.name,
                source_label(&record.source)
            )?;
        }
        MarketplaceAction::List => {
            let records = store.list_marketplaces()?;
            if records.is_empty() {
                writeln!(output, "no marketplaces are registered")?;
            }
            for record in records {
                writeln!(
                    output,
                    "{} · {} · commit {}",
                    record.name,
                    source_label(&record.source),
                    record
                        .resolved_commit
                        .as_deref()
                        .unwrap_or("none (local source)")
                )?;
                for plugin in store.marketplace(&record.name)?.plugins {
                    writeln!(output, "  {}", plugin.name)?;
                }
            }
        }
        MarketplaceAction::Update(name) => {
            let names = match name {
                Some(name) => vec![name],
                None => store
                    .list_marketplaces()?
                    .into_iter()
                    .map(|record| record.name)
                    .collect(),
            };
            for name in names {
                let record = store.update_marketplace(&name)?;
                writeln!(
                    output,
                    "Updated marketplace `{}` · {}",
                    record.name,
                    source_label(&record.source)
                )?;
            }
        }
        MarketplaceAction::Remove(name) => {
            store.remove_marketplace(&name)?;
            writeln!(output, "Removed marketplace `{name}`")?;
        }
    }
    Ok(())
}
