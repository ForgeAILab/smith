//! Runtime inventory, workspace/session resources, and resume selection.

use super::*;

pub(super) fn runtime_resources(
    inventory: SelectionInventory,
    sessions: Vec<SessionListing>,
    current_session: &str,
    project: &std::path::Path,
    agents: &ResolvedAgent,
    reasoning: &smith_runtime::reasoning::ReasoningRuntimePolicy,
) -> RuntimeResources {
    let model_limits = inventory
        .models
        .iter()
        .map(|model| {
            (
                model.id(),
                format!(
                    "ctx {} · input {} · output {}",
                    render_optional_inventory_limit(model.context_tokens.as_ref()),
                    render_optional_inventory_limit(model.max_input_tokens.as_ref()),
                    render_optional_inventory_limit(model.max_output_tokens.as_ref()),
                ),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let profile_inventory = inventory.profiles;
    let profiles = profile_inventory
        .iter()
        .filter(|profile| profile.uses.contains(&ProfileUse::Main))
        .map(|profile| {
            let pair = profile
                .pair()
                .unwrap_or_else(|| "incomplete provider/model selection".to_owned());
            let description = profile.description.as_deref().unwrap_or("agent profile");
            let placements = profile
                .uses
                .iter()
                .map(|placement| placement.as_str())
                .collect::<Vec<_>>()
                .join("+");
            let revision = bounded_text(&profile.revision, 12);
            let source = profile
                .source
                .as_ref()
                .map_or_else(|| "unknown source".to_owned(), ToString::to_string);
            let legacy = if profile.legacy {
                format!(
                    " · legacy from {}; migrate to [profiles.{}]",
                    bounded_text(&source, 48),
                    profile.name
                )
            } else {
                format!(" · source {}", bounded_text(&source, 48))
            };
            let detail = format!(
                "{} · use {placements} · {pair} · {description} · rev {revision}{legacy}",
                profile.posture.as_str(),
            );
            let id = if profile.legacy {
                format!("{LEGACY_AGENT_PROFILE_PREFIX}{}", profile.name)
            } else {
                profile.name.clone()
            };
            let entry = ResourceEntry::new(id, profile.name.clone(), detail).active(profile.active);
            if profile.selectable {
                entry
            } else {
                entry.disabled("profile does not resolve to a usable provider/model pair")
            }
        })
        .collect();
    let mut connections = inventory
        .providers
        .iter()
        .map(|provider| {
            let authentication = if provider.kind.as_deref()
                == Some(smith_config::model::KIND_CHATGPT_RESPONSES)
            {
                "Smith OAuth · experimental direct Responses"
            } else if provider.kind.as_deref()
                == Some(smith_config::model::KIND_GEMINI_INTERACTIONS)
            {
                "AI Studio API key · native Gemini Interactions"
            } else if provider.kind.as_deref() == Some(smith_config::model::KIND_XAI_RESPONSES) {
                "browser login · renewable session"
            } else if provider.kind.as_deref() == Some(smith_config::model::KIND_OPENAI_RESPONSES) {
                "API key · stateless Responses"
            } else {
                "API key"
            };
            let entry = ResourceEntry::new(
                provider.name.clone(),
                provider.name.clone(),
                format!(
                    "{authentication} · {} · {}",
                    provider.kind.as_deref().unwrap_or("unknown adapter"),
                    if provider.active {
                        "active"
                    } else {
                        "configured"
                    }
                ),
            )
            .active(provider.active);
            if provider.selectable {
                entry
            } else {
                entry.disabled("configure an available adapter and at least one model first")
            }
        })
        .collect::<Vec<_>>();
    let disconnections = inventory
        .providers
        .iter()
        .map(|provider| {
            ResourceEntry::new(
                provider.name.clone(),
                provider.name.clone(),
                format!(
                    "remove only the configured authentication source · {}",
                    provider.kind.as_deref().unwrap_or("unknown adapter")
                ),
            )
            .active(provider.active)
        })
        .collect::<Vec<_>>();
    if !connections.iter().any(|entry| entry.id == "openrouter") {
        connections.push(ResourceEntry::new(
            "openrouter",
            "OpenRouter",
            "API key · fixed OpenRouter endpoint · adds a reviewed model",
        ));
    }
    if !connections.iter().any(|entry| entry.id == "chatgpt") {
        connections.push(ResourceEntry::new(
            "chatgpt",
            "ChatGPT (experimental)",
            "Smith OAuth · direct ChatGPT Responses · unsupported public API boundary",
        ));
    }
    if !connections.iter().any(|entry| entry.id == "xai") {
        connections.push(ResourceEntry::new(
            "xai",
            "xAI Grok",
            "browser login or API key · fixed xAI Responses endpoint · catalog-backed model",
        ));
    }
    if !connections.iter().any(|entry| entry.id == "google") {
        connections.push(ResourceEntry::new(
            "google",
            "Google Gemini",
            "AI Studio API key · fixed native Gemini endpoint · catalog-backed model",
        ));
    }
    let providers = inventory
        .providers
        .into_iter()
        .map(|provider| {
            let kind = provider.kind.as_deref().unwrap_or("missing adapter kind");
            let detail = format!(
                "{kind} · {} {}",
                provider.model_count,
                if provider.model_count == 1 {
                    "model"
                } else {
                    "models"
                }
            );
            let entry = ResourceEntry::new(provider.name.clone(), provider.name, detail)
                .active(provider.active);
            if provider.selectable {
                entry
            } else {
                entry.disabled("adapter unavailable or no model with enforceable limits")
            }
        })
        .collect::<Vec<_>>();
    let models = inventory
        .models
        .into_iter()
        .map(|model| {
            let id = model.id();
            let profiles = if model.profiles.is_empty() {
                String::new()
            } else {
                format!(" · profiles {}", model.profiles.join(","))
            };
            let capabilities = [
                model
                    .tool_call
                    .is_some_and(|enabled| enabled)
                    .then_some("tools"),
                model
                    .reasoning
                    .is_some_and(|enabled| enabled)
                    .then_some("reasoning"),
                model
                    .structured_output
                    .is_some_and(|enabled| enabled)
                    .then_some("structured"),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            let capabilities = if capabilities.is_empty() {
                String::new()
            } else {
                format!(" · {}", capabilities.join("+"))
            };
            let provenance = match (
                model.catalog_provider.as_deref(),
                model.catalog_revision.as_deref(),
                model.catalog_retrieved_at_ms,
            ) {
                (Some(provider), Some(revision), Some(retrieved)) => format!(
                    " · models.dev/{provider} advertised · rev {} · {} old",
                    bounded_text(revision, 12),
                    catalog_age(retrieved)
                ),
                _ => String::new(),
            };
            let entry = ResourceEntry::new(
                id.clone(),
                model.label,
                format!(
                    "{id} · ctx {} · input {} · output {}{capabilities}{provenance}{profiles}",
                    render_optional_inventory_limit(model.context_tokens.as_ref()),
                    render_optional_inventory_limit(model.max_input_tokens.as_ref()),
                    render_optional_inventory_limit(model.max_output_tokens.as_ref()),
                ),
            )
            .active(model.active);
            match model.disabled_reason {
                Some(reason) => entry.disabled(reason),
                None if model.selectable => entry,
                None => entry.disabled("model is not locally selectable"),
            }
        })
        .collect::<Vec<_>>();
    let session_entries = session_resource_entries(sessions, Some(current_session));
    let files = workspace_file_entries(project, 4_096);
    let child_agents = profile_inventory
        .iter()
        .filter(|profile| profile.uses.contains(&ProfileUse::Child))
        .map(|profile| {
            let description = profile
                .description
                .as_deref()
                .unwrap_or("read-only child profile");
            let pair = profile.pair().unwrap_or_else(|| {
                format!(
                    "{}/{}",
                    agents
                        .profile
                        .provider
                        .as_ref()
                        .map_or("current", |value| value.value.as_str()),
                    agents
                        .profile
                        .model
                        .as_ref()
                        .map_or("model", |value| value.value.as_str())
                )
            });
            let limits = model_limits
                .get(&pair)
                .map_or("limits inherited from active runtime", String::as_str);
            let instructions = agents
                .profiles
                .get(&profile.name)
                .and_then(|resolved| resolved.instructions.as_ref())
                .map_or("default instructions", |_| "custom instructions configured");
            let revision = bounded_text(&profile.revision, 12);
            let source = profile
                .source
                .as_ref()
                .map_or_else(|| "unknown source".to_owned(), ToString::to_string);
            let legacy = if profile.legacy {
                format!(
                    " · legacy from {}; migrate to [profiles.{}] use=[\"child\"]",
                    bounded_text(&source, 48),
                    profile.name
                )
            } else {
                format!(" · source {}", bounded_text(&source, 48))
            };
            let entry = ResourceEntry::new(
                format!("agent:{}", profile.name),
                profile.name.clone(),
                format!(
                    "child profile · {} · {pair} · {limits} · {instructions} · {description} · rev {revision}{legacy}",
                    profile.posture.as_str(),
                ),
            );
            if profile.selectable {
                entry
            } else {
                entry.disabled("child profile does not resolve a usable provider/model pair")
            }
        })
        .collect();
    let main_profiles = agents
        .profile_order
        .value
        .iter()
        .filter_map(|name| {
            let profile = profile_inventory
                .iter()
                .find(|profile| profile.name == *name)?;
            let description = profile
                .description
                .as_deref()
                .unwrap_or("main agent profile");
            Some(
                ResourceEntry::new(
                    if profile.legacy {
                        format!("{LEGACY_AGENT_PROFILE_PREFIX}{name}")
                    } else {
                        name.clone()
                    },
                    name.clone(),
                    format!(
                        "main profile · {} · {description} · rev {}",
                        profile.posture.as_str(),
                        bounded_text(&profile.revision, 12)
                    ),
                )
                .active(agents.profile.name == *name),
            )
        })
        .collect();

    let capability_reason = || match reasoning.support {
        ReasoningSupport::Unsupported => {
            "this model does not advertise reasoning support".to_owned()
        }
        ReasoningSupport::Fixed => {
            format!("reasoning is fixed; {}", reasoning.capability_source)
        }
        ReasoningSupport::Controllable => format!(
            "the active binding has no explicit switch; {}",
            reasoning.capability_source
        ),
    };
    let mut thinking = vec![
        ResourceEntry::new(
            "default",
            "provider default",
            "clear the session thinking override",
        )
        .active(reasoning.selected_enabled.is_none()),
    ];
    let on = ResourceEntry::new("on", "on", "enable thinking for the next turn")
        .active(reasoning.selected_enabled == Some(true));
    thinking.push(match reasoning.switch {
        smith_runtime::reasoning::ReasoningSwitch::Optional
        | smith_runtime::reasoning::ReasoningSwitch::MandatoryOn
            if reasoning.dialect != Some(smith_config::model::ReasoningDialect::OpenaiEffort)
                || reasoning.selected_effort.is_some()
                || reasoning.default_effort.is_some() =>
        {
            on
        }
        smith_runtime::reasoning::ReasoningSwitch::Optional
        | smith_runtime::reasoning::ReasoningSwitch::MandatoryOn => {
            on.disabled("choose an advertised /effort to turn reasoning on")
        }
        smith_runtime::reasoning::ReasoningSwitch::Unavailable => on.disabled(capability_reason()),
    });
    let off = ResourceEntry::new("off", "off", "disable thinking for the next turn")
        .active(reasoning.selected_enabled == Some(false));
    thinking.push(match reasoning.switch {
        // The OpenAI-effort dialect sends off as the effort `none`, so off is
        // selectable only when that effort is advertised. Mirrors the
        // validation in `smith_runtime::reasoning::resolve_reasoning_policy`.
        smith_runtime::reasoning::ReasoningSwitch::Optional
            if reasoning.dialect == Some(smith_config::model::ReasoningDialect::OpenaiEffort)
                && !reasoning.efforts.iter().any(|effort| effort == "none") =>
        {
            off.disabled("off requires this binding to advertise the `none` effort")
        }
        smith_runtime::reasoning::ReasoningSwitch::Optional => off,
        smith_runtime::reasoning::ReasoningSwitch::MandatoryOn => {
            off.disabled("reasoning is mandatory for this provider/model")
        }
        smith_runtime::reasoning::ReasoningSwitch::Unavailable => off.disabled(capability_reason()),
    });

    let mut efforts = vec![
        ResourceEntry::new(
            "default",
            "provider default",
            "clear the session effort override",
        )
        .active(reasoning.selected_effort.is_none()),
    ];
    efforts.extend(reasoning.efforts.iter().map(|effort| {
        ResourceEntry::new(
            effort.clone(),
            effort.clone(),
            "applies to every request in the next turn",
        )
        .active(reasoning.selected_effort.as_deref() == Some(effort.as_str()))
    }));
    if reasoning.efforts.is_empty() {
        efforts.push(
            ResourceEntry::new("unavailable", "not adjustable", capability_reason())
                .disabled(capability_reason()),
        );
    }

    RuntimeResources {
        models,
        providers,
        connections,
        disconnections,
        profiles,
        sessions: session_entries,
        files,
        child_agents,
        main_profiles,
        thinking,
        efforts,
        current_session: Some(current_session.to_owned()),
    }
}

pub(super) fn workspace_file_entries(
    project: &std::path::Path,
    limit: usize,
) -> Vec<ResourceEntry> {
    let Ok(root) = project.canonicalize() else {
        return Vec::new();
    };
    let mut walker = WalkBuilder::new(&root);
    walker
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git");

    let mut entries = Vec::new();
    for entry in walker.build().filter_map(Result::ok) {
        if entries.len() >= limit {
            break;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(canonical) = entry.path().canonicalize() else {
            continue;
        };
        if !canonical.starts_with(&root) {
            continue;
        }
        let Ok(relative) = canonical.strip_prefix(&root) else {
            continue;
        };
        let path = relative.to_string_lossy().replace('\\', "/");
        if path.is_empty() {
            continue;
        }
        let bytes = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        entries.push(ResourceEntry::new(
            format!("file:{path}"),
            path,
            format!("file · {bytes} bytes"),
        ));
    }
    entries.sort_by(|left, right| left.label.cmp(&right.label));
    entries
}

pub(super) fn session_resource_entries(
    sessions: Vec<SessionListing>,
    current: Option<&str>,
) -> Vec<ResourceEntry> {
    sessions
        .into_iter()
        .map(|session| {
            let id = session.id.as_str().to_owned();
            let active = current.is_some_and(|session| id == session);
            let preview = session
                .user_preview
                .as_deref()
                .map(|preview| bounded_text(preview, 64))
                .unwrap_or_else(|| "No user preview".to_owned());
            let turns = session
                .turn_count
                .map_or_else(|| "? turns".to_owned(), |count| format!("{count} turns"));
            let pair = match (session.provider.as_deref(), session.model.as_deref()) {
                (Some(provider), Some(model)) => format!("{provider}/{model}"),
                _ => "unknown provider/model".to_owned(),
            };
            let updated = session
                .updated
                .map_or_else(|| "unknown update".to_owned(), format_session_updated);
            let entry = ResourceEntry::new(
                &id,
                format!("{} · {preview}", short_session_id(&id)),
                format!("{turns} · {pair} · updated {updated}"),
            )
            .active(active);
            if session.schema_version == SNAPSHOT_SCHEMA_VERSION {
                entry
            } else {
                entry.disabled(format!(
                    "snapshot schema {} is newer than this build",
                    session.schema_version
                ))
            }
        })
        .collect()
}

pub(super) fn format_session_updated(timestamp: Timestamp) -> String {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let now = time::OffsetDateTime::now_utc().to_offset(offset);
    format_session_updated_at(timestamp, offset, now)
}

pub(super) fn format_session_updated_at(
    timestamp: Timestamp,
    offset: time::UtcOffset,
    now: time::OffsetDateTime,
) -> String {
    let millis = timestamp.as_millis();
    let Ok(instant) =
        time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000)
    else {
        return format!("{millis}ms");
    };
    let instant = instant.to_offset(offset);
    if instant <= now && instant.date() == now.date() {
        let seconds = (now - instant).whole_seconds();
        if seconds < 60 {
            return "just now".to_owned();
        }
        let minutes = seconds / 60;
        if minutes < 60 {
            return format!(
                "{minutes} minute{} ago",
                if minutes == 1 { "" } else { "s" }
            );
        }
        let hours = minutes / 60;
        return format!("{hours} hour{} ago", if hours == 1 { "" } else { "s" });
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {}",
        instant.year(),
        u8::from(instant.month()),
        instant.day(),
        instant.hour(),
        instant.minute(),
        instant.second(),
        format_session_offset(instant.offset()),
    )
}

pub(super) fn format_session_offset(offset: time::UtcOffset) -> String {
    let seconds = offset.whole_seconds();
    let absolute = seconds.unsigned_abs();
    format!(
        "{}{hours:02}:{minutes:02}",
        if seconds < 0 { "-" } else { "+" },
        hours = absolute / 3_600,
        minutes = absolute % 3_600 / 60,
    )
}

pub(super) fn render_inventory_limit(limit: &InventoryLimit) -> String {
    let provenance = match &limit.origin {
        ModelLimitOrigin::Configured(source) => source.layer.label().to_owned(),
        ModelLimitOrigin::Trusted { catalog, revision } => {
            format!("{catalog} r{revision}")
        }
        ModelLimitOrigin::Catalog {
            catalog,
            revision: _,
            retrieved_at_ms: _,
        } => catalog.clone(),
    };
    format!("{} [{provenance}]", token_quantity(limit.value))
}

pub(super) fn render_optional_inventory_limit(limit: Option<&InventoryLimit>) -> String {
    limit.map_or_else(|| "unknown".to_owned(), render_inventory_limit)
}

pub(super) fn catalog_age(retrieved_at_ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        });
    let seconds = now.saturating_sub(retrieved_at_ms) / 1_000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 60 * 60 {
        format!("{}m", seconds / 60)
    } else if seconds < 24 * 60 * 60 {
        format!("{}h", seconds / (60 * 60))
    } else {
        format!("{}d", seconds / (24 * 60 * 60))
    }
}

pub(super) fn token_quantity(tokens: u32) -> String {
    if tokens >= 1_000 && tokens.is_multiple_of(1_000) {
        format!("{}k", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

pub(super) fn short_session_id(id: &str) -> String {
    bounded_text(id, 12)
}

pub(super) fn bounded_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let head = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

pub(super) async fn choose_resume_session(
    selection: &Selection,
    no_color: bool,
    no_motion: bool,
) -> Result<Option<String>> {
    let prepared = prepare(selection)?;
    let sessions = smith_runtime::host::list(&prepared.resolution.config, &prepared.project)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("listing project sessions")?;
    let entries = session_resource_entries(sessions, None);
    let mut picker = ResourcePicker::new(
        "Resume session",
        entries,
        "Nothing to resume for this project · Esc to start without resuming",
    );
    let mut theme = Theme::from_env();
    if no_color {
        theme = theme.without_color();
    }
    if no_motion {
        theme = theme.without_motion();
    }

    let mut terminal = terminal::enter().context("entering the resume picker")?;
    let mut events = EventStream::new();
    let result = async {
        terminal.draw(|frame| {
            let area = standalone_picker_area(frame.area(), picker.entries.len());
            draw_resource_picker(frame, area, &picker, theme);
        })?;
        loop {
            let Some(event) = events.next().await else {
                return Ok(None);
            };
            match event.context("reading a terminal event")? {
                TermEvent::Key(key) => match picker.on_key(key) {
                    PickerOutcome::Pending => {}
                    PickerOutcome::Cancelled => return Ok(None),
                    PickerOutcome::Selected(session) => return Ok(Some(session)),
                },
                TermEvent::Paste(text) => picker.paste(&text),
                TermEvent::Resize(_, _) => {}
                _ => continue,
            }
            terminal.draw(|frame| {
                let area = standalone_picker_area(frame.area(), picker.entries.len());
                draw_resource_picker(frame, area, &picker, theme);
            })?;
        }
    }
    .await;
    let restore = terminal.restore().context("restoring the terminal");
    restore?;
    result
}

pub(super) fn standalone_picker_area(area: Rect, entry_count: usize) -> Rect {
    if area.width < 24 || area.height < 8 {
        return area;
    }
    let width = area.width.saturating_sub(4).min(100);
    let height = u16::try_from(entry_count.saturating_add(4))
        .unwrap_or(u16::MAX)
        .clamp(6, area.height.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

pub(super) async fn list_sessions(selection: &Selection) -> Result<()> {
    let prepared = prepare(selection)?;
    let sessions = smith_runtime::host::list(&prepared.resolution.config, &prepared.project)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    for session in sessions {
        let updated = session.updated.map_or_else(
            || "unknown-version".to_owned(),
            |updated| updated.to_string(),
        );
        let turns = session
            .turn_count
            .map_or_else(|| "?".to_owned(), |turns| turns.to_string());
        let provider = session.provider.as_deref().unwrap_or("?");
        let model = session.model.as_deref().unwrap_or("?");
        let preview = session.user_preview.as_deref().unwrap_or("no user preview");
        println!(
            "{}\t{updated}\t{turns}\t{provider}/{model}\t{}",
            session.id.as_str(),
            bounded_text(preview, 80)
        );
    }
    Ok(())
}

/// Shortens a home-relative path to `~/…` for the header.
pub(super) fn abbreviate_home(path: &str) -> String {
    match std::env::var_os("HOME") {
        Some(home) => abbreviate(path, &home.to_string_lossy()),
        None => path.to_owned(),
    }
}

/// The pure half of [`abbreviate_home`].
pub(super) fn abbreviate(path: &str, home: &str) -> String {
    match path.strip_prefix(home) {
        Some("") => "~".to_owned(),
        Some(rest) => format!("~{rest}"),
        None => path.to_owned(),
    }
}
