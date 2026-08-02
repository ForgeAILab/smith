//! Configuration readiness, resolution, and explain commands.

use super::*;

pub(super) struct Prepared {
    pub(super) resolution: Resolution,
    pub(super) project: PathBuf,
}

pub(super) fn inspect_selection(selection: &Selection) -> Result<ConfigReadiness> {
    let (_, request) = resolution_request(selection)?;
    Ok(inspect(&request))
}

pub(super) fn prepare(selection: &Selection) -> Result<Prepared> {
    let (start, request) = resolution_request(selection)?;
    let resolution = resolve(&request)
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("resolving Smith configuration")?;
    let project = resolution.layout.project_root.clone().unwrap_or(start);
    Ok(Prepared {
        resolution,
        project,
    })
}

pub(super) fn resolution_request(selection: &Selection) -> Result<(PathBuf, ResolveRequest)> {
    let start = match &selection.project {
        Some(project) => project.clone(),
        None => std::env::current_dir().context("reading the current directory")?,
    };
    let start = start
        .canonicalize()
        .with_context(|| format!("resolving project path `{}`", start.display()))?;
    if !start.is_dir() {
        anyhow::bail!("project path `{}` is not a directory", start.display());
    }

    let request = ResolveRequest::new(&start)
        .with_env(std::env::vars())
        .with_cli(selection.overrides())
        .with_session(selection.session_overrides());
    Ok((start, request))
}

pub(super) fn explain_config(key: &str, selection: &Selection) -> Result<()> {
    let prepared = prepare(selection)?;
    let explanation = prepared
        .resolution
        .provenance
        .explain(key)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    println!("{} = {}", explanation.key, explanation.value);
    println!("source: {}", explanation.source);
    for entry in explanation.overridden {
        println!("overrode: {} from {}", entry.value, entry.source);
    }
    Ok(())
}
