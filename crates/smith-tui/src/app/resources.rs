//! Palette, runtime-resource selection, and local-result transitions.

use crossterm::event::{KeyCode, KeyEvent};

use crate::commands::{self, CommandAction, GoalAction};
use crate::picker::{PickerOutcome, ResourceEntry, ResourcePicker};
use crate::status::Activity;
use crate::transcript::LocalResultState;

use super::state::*;

impl App {
    pub(super) fn on_resource_picker_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.code == KeyCode::Char('@')
            && matches!(
                self.overlay,
                Some(Overlay::ResourcePicker {
                    target: ResourceTarget::Reference,
                    ref picker,
                    ..
                }) if picker.query.is_empty()
            )
        {
            self.overlay = None;
            self.composer.insert_str("@@");
            return None;
        }
        let outcome = match &mut self.overlay {
            Some(Overlay::ResourcePicker { picker, .. }) => picker.on_key(key),
            _ => return None,
        };
        match outcome {
            PickerOutcome::Pending => None,
            PickerOutcome::Cancelled => {
                if let Some(Overlay::ResourcePicker {
                    target,
                    restore_on_escape,
                    ..
                }) = self.overlay.take()
                    && target != ResourceTarget::Reference
                {
                    self.composer.replace(restore_on_escape);
                }
                None
            }
            PickerOutcome::Selected(id) => {
                let (target, restore) = match self.overlay.take() {
                    Some(Overlay::ResourcePicker {
                        target,
                        restore_on_escape,
                        ..
                    }) => (target, restore_on_escape),
                    _ => return None,
                };
                self.apply_resource_selection(target, id, restore)
            }
        }
    }

    /// Replaces the credential-pool entries the account picker lists.
    ///
    /// Separate from `set_resources` because pool state changes on its own
    /// clock — a snapshot arrives, a window resets — and rebuilding every
    /// model, session, and file index to redraw one usage meter would be
    /// wasteful and would stamp on pickers the user has open.
    pub fn set_accounts(&mut self, accounts: Vec<ResourceEntry>) {
        self.resources.accounts = accounts;
    }

    pub(super) fn open_resource_picker(
        &mut self,
        target: ResourceTarget,
        entries: Vec<ResourceEntry>,
        empty_guidance: &str,
        restore_on_escape: String,
        initial_query: Option<&str>,
    ) {
        let title = match target {
            ResourceTarget::Model => "Choose model",
            ResourceTarget::Provider => "Choose provider",
            ResourceTarget::Connect => "Connect provider",
            ResourceTarget::Disconnect => "Disconnect provider",
            ResourceTarget::Profile => "Choose profile",
            ResourceTarget::Resume => "Resume session",
            ResourceTarget::Think => "Choose thinking state",
            ResourceTarget::Effort => "Choose reasoning effort",
            ResourceTarget::Reference => "Attach file or invoke agent",
            ResourceTarget::Account => "Choose account",
        };
        let mut picker = ResourcePicker::new(title, entries, empty_guidance);
        if let Some(query) = initial_query {
            picker.query = query.to_owned();
        }
        self.overlay = Some(Overlay::ResourcePicker {
            picker,
            target,
            restore_on_escape,
        });
    }

    pub(super) fn open_target_picker(&mut self, target: ResourceTarget, restore: String) {
        let (entries, guidance) = match target {
            ResourceTarget::Model => (
                self.resources.models.clone(),
                "No local model is selectable · run smith setup add-model",
            ),
            ResourceTarget::Provider => (
                self.resources.providers.clone(),
                "No provider is selectable · run smith setup add-provider",
            ),
            ResourceTarget::Connect => (
                self.resources.connections.clone(),
                "No supported provider connection is available",
            ),
            ResourceTarget::Disconnect => (
                self.resources.disconnections.clone(),
                "No provider is currently connected",
            ),
            ResourceTarget::Profile => (
                self.resources.profiles.clone(),
                "No profile is selectable · run smith setup",
            ),
            ResourceTarget::Resume => (
                self.resources.sessions.clone(),
                "Nothing to resume for this project · use /new",
            ),
            ResourceTarget::Think => (
                self.resources.thinking.clone(),
                "Thinking is not adjustable for this provider/model",
            ),
            ResourceTarget::Effort => (
                self.resources.efforts.clone(),
                "Effort is not adjustable for this provider/model",
            ),
            ResourceTarget::Account => (
                self.resources.accounts.clone(),
                "This provider declares a single credential · add `credentials` to pool accounts",
            ),
            ResourceTarget::Reference => {
                let mut entries = self
                    .resources
                    .child_agents
                    .iter()
                    .chain(&self.resources.files)
                    .cloned()
                    .collect::<Vec<_>>();
                entries.extend(self.children.iter().map(|(child, summary)| {
                    ResourceEntry::new(
                        format!("agent:{child}"),
                        child.clone(),
                        format!(
                            "existing child · {}{}",
                            summary.state,
                            summary
                                .detail
                                .as_deref()
                                .map(|detail| format!(" · {detail}"))
                                .unwrap_or_default()
                        ),
                    )
                }));
                (
                    entries,
                    "No matching file, child-enabled profile, or existing child in the bounded local index",
                )
            }
        };
        self.open_resource_picker(target, entries, guidance, restore, None);
    }

    pub(super) fn apply_resource_selection(
        &mut self,
        target: ResourceTarget,
        id: String,
        restore: String,
    ) -> Option<Action> {
        match target {
            ResourceTarget::Model => self.apply_model_id(&id),
            ResourceTarget::Provider => {
                let models: Vec<ResourceEntry> = self
                    .resources
                    .models
                    .iter()
                    .filter(|entry| {
                        model_pair(&self.resources.providers, &entry.id)
                            .is_some_and(|(provider, _)| provider == id)
                    })
                    .cloned()
                    .collect();
                match models.as_slice() {
                    [] => {
                        self.transcript.push_error(format!(
                            "provider `{id}` has no selectable local model; run `smith setup add-model --provider {id}`"
                        ));
                        None
                    }
                    [only] => self.apply_model_id(&only.id),
                    _ => {
                        self.open_resource_picker(
                            ResourceTarget::Model,
                            models,
                            "This provider has no selectable model · run smith setup add-model",
                            restore,
                            None,
                        );
                        None
                    }
                }
            }
            ResourceTarget::Account => {
                self.composer.clear();
                let Ok(position) = id.parse::<usize>() else {
                    self.transcript
                        .push_error("the account picker returned an invalid pool position");
                    return None;
                };
                Some(Action::Reconfigure(PaletteCommand::Account(position)))
            }
            ResourceTarget::Connect => {
                self.composer.clear();
                Some(Action::Reconfigure(PaletteCommand::Connect(id)))
            }
            ResourceTarget::Disconnect => {
                self.composer.clear();
                Some(Action::Reconfigure(PaletteCommand::Disconnect(id)))
            }
            ResourceTarget::Profile => {
                self.composer.clear();
                Some(Action::Reconfigure(profile_palette_command(id)))
            }
            ResourceTarget::Resume => {
                self.composer.clear();
                if self.resources.current_session.as_deref() == Some(id.as_str()) {
                    self.transcript
                        .push_notice("resume", "already in the selected session");
                    None
                } else {
                    Some(Action::Reconfigure(PaletteCommand::Resume(id)))
                }
            }
            ResourceTarget::Think => {
                self.composer.clear();
                Some(Action::Reconfigure(PaletteCommand::Think(
                    match id.as_str() {
                        "default" => None,
                        "on" => Some(true),
                        "off" => Some(false),
                        _ => {
                            self.transcript
                                .push_error("thinking picker returned an invalid typed value");
                            return None;
                        }
                    },
                )))
            }
            ResourceTarget::Effort => {
                self.composer.clear();
                Some(Action::Reconfigure(PaletteCommand::Effort(
                    (id != "default").then_some(id),
                )))
            }
            ResourceTarget::Reference => {
                let selected = id
                    .strip_prefix("file:")
                    .map(|identity| ("file", identity))
                    .or_else(|| {
                        id.strip_prefix("agent:")
                            .map(|identity| ("agent", identity))
                    });
                match selected {
                    Some((kind, identity)) => {
                        let collides = self
                            .resources
                            .files
                            .iter()
                            .any(|entry| entry.id.strip_prefix("file:") == Some(identity))
                            && self
                                .resources
                                .child_agents
                                .iter()
                                .any(|entry| entry.id.strip_prefix("agent:") == Some(identity));
                        if collides {
                            self.composer.insert_str(&format!("@{kind}:{identity} "));
                        } else {
                            self.composer.insert_str(&format!("@{identity} "));
                        }
                    }
                    None => self
                        .transcript
                        .push_error("reference picker returned an invalid typed identity"),
                }
                None
            }
        }
    }

    pub(super) fn cycle_agent_profile(&mut self, backwards: bool) -> Option<Action> {
        let selectable = self
            .resources
            .main_profiles
            .iter()
            .filter(|entry| entry.disabled_reason.is_none())
            .collect::<Vec<_>>();
        if selectable.len() < 2 {
            return None;
        }
        // When the active profile is itself unselectable, cycling starts at
        // the first selectable entry instead of silently skipping it.
        let next = match selectable.iter().position(|entry| entry.active) {
            Some(current) if backwards => current.checked_sub(1).unwrap_or(selectable.len() - 1),
            Some(current) => (current + 1) % selectable.len(),
            None if backwards => selectable.len() - 1,
            None => 0,
        };
        Some(Action::Reconfigure(profile_palette_command(
            selectable[next].id.clone(),
        )))
    }

    pub(super) fn apply_model_id(&mut self, id: &str) -> Option<Action> {
        let Some((provider, model)) = model_pair(&self.resources.providers, id) else {
            self.transcript
                .push_error(format!("model choice `{id}` has no provider identity"));
            return None;
        };
        self.composer.clear();
        Some(Action::Reconfigure(PaletteCommand::Model {
            provider,
            model,
        }))
    }

    pub(super) fn direct_model(&mut self, value: &str, restore: String) -> Option<Action> {
        if self.resources.models.iter().any(|entry| {
            entry.id == value
                && entry.disabled_reason.is_none()
                && model_pair(&self.resources.providers, &entry.id).is_some()
        }) {
            self.accept_composer_input();
            return self.apply_model_id(value);
        }
        let mut matches: Vec<ResourceEntry> = self
            .resources
            .models
            .iter()
            .filter(|entry| {
                entry.disabled_reason.is_none()
                    && model_pair(&self.resources.providers, &entry.id)
                        .is_some_and(|(_, model)| model == value)
            })
            .cloned()
            .collect();
        if let Some(active_provider) = self.status.provider.as_deref()
            && let Some(position) = matches.iter().position(|entry| {
                model_pair(&self.resources.providers, &entry.id)
                    .is_some_and(|(provider, _)| provider == active_provider)
            })
        {
            let selected = matches.remove(position);
            self.accept_composer_input();
            return self.apply_model_id(&selected.id);
        }
        match matches.as_slice() {
            [only] => {
                let id = only.id.clone();
                self.accept_composer_input();
                self.apply_model_id(&id)
            }
            [] => {
                self.transcript.push_error(format!(
                    "model `{value}` is not locally selectable; run `smith setup add-model`"
                ));
                None
            }
            _ => {
                self.transcript.push_error(format!(
                    "model `{value}` is available from multiple providers; choose a qualified pair"
                ));
                self.accept_composer_input();
                self.open_resource_picker(
                    ResourceTarget::Model,
                    matches,
                    "No matching provider/model pair",
                    restore,
                    Some(value),
                );
                None
            }
        }
    }

    pub(super) fn apply_direct_reasoning_choice(
        &mut self,
        target: ResourceTarget,
        value: &str,
        restore: String,
    ) -> Option<Action> {
        let normalized = value.trim().to_ascii_lowercase();
        let entries = match target {
            ResourceTarget::Think => &self.resources.thinking,
            ResourceTarget::Effort => &self.resources.efforts,
            _ => unreachable!("reasoning choice helper receives only reasoning targets"),
        };
        if entries
            .iter()
            .any(|entry| entry.id == normalized && entry.disabled_reason.is_none())
        {
            self.accept_composer_input();
            return self.apply_resource_selection(target, normalized, restore);
        }
        let reason = entries
            .iter()
            .find(|entry| entry.id == normalized)
            .and_then(|entry| entry.disabled_reason.as_deref())
            .map_or_else(
                || match target {
                    ResourceTarget::Think => {
                        "use `on`, `off`, or `default`, subject to the active model".to_owned()
                    }
                    ResourceTarget::Effort => {
                        let supported = entries
                            .iter()
                            .filter(|entry| entry.disabled_reason.is_none())
                            .map(|entry| entry.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("supported values: {supported}")
                    }
                    _ => unreachable!(),
                },
                str::to_owned,
            );
        self.transcript.push_error(format!(
            "reasoning choice `{value}` is unavailable: {reason}"
        ));
        None
    }

    pub(super) fn dispatch_command(&mut self, command: CommandAction) -> Option<Action> {
        let Some(spec) = commands::COMMANDS.iter().find(|spec| {
            let name = match &command {
                CommandAction::Help => "help",
                CommandAction::Status => "status",
                CommandAction::Goal(_) => "goal",
                CommandAction::Context => "context",
                CommandAction::Details => "details",
                CommandAction::Timeline => "timeline",
                CommandAction::NewSession => "new",
                CommandAction::Resume(_) => "resume",
                CommandAction::Profile(_) => "profile",
                CommandAction::Provider(_) => "provider",
                CommandAction::Connect(_) => "connect",
                CommandAction::Disconnect(_) => "disconnect",
                CommandAction::Model(_) => "model",
                CommandAction::Think(_) => "think",
                CommandAction::Effort(_) => "effort",
                CommandAction::Account(_) => "account",
                CommandAction::Agent(_) | CommandAction::AgentResume(_) => "agent",
                CommandAction::Mcp(_) => "mcp",
                CommandAction::Diff(_) => "diff",
                CommandAction::Review(_) => "review",
                CommandAction::Undo => "undo",
                CommandAction::Redo => "redo",
                CommandAction::Revert(_) => "revert",
                CommandAction::Quit => "quit",
            };
            spec.name == name
        }) else {
            unreachable!("parsed commands always have registry entries");
        };

        if spec.requires_idle && (self.is_busy() || self.has_pending_input()) {
            self.overlay = None;
            self.transcript.push_notice(
                "smith",
                format!(
                    "/{name} requires an idle turn; draft preserved",
                    name = spec.name
                ),
            );
            return None;
        }
        if self.is_busy()
            && matches!(
                command,
                CommandAction::Goal(
                    GoalAction::Create(_)
                        | GoalAction::Edit(_)
                        | GoalAction::Budget(_)
                        | GoalAction::Resume
                        | GoalAction::Clear
                )
            )
        {
            self.overlay = None;
            self.transcript.push_notice(
                "goal",
                "this goal change requires an idle turn; command preserved",
            );
            return None;
        }

        self.overlay = None;
        let restore = self.composer.text().to_owned();
        match command {
            CommandAction::Help => {
                self.accept_composer_input();
                self.show_local_result("help", commands::help());
                None
            }
            CommandAction::Details => {
                self.accept_composer_input();
                self.toggle_work_details();
                self.transcript.push_notice(
                    "details",
                    if self.work_details {
                        "bounded tool details shown"
                    } else {
                        "bounded tool details hidden"
                    },
                );
                None
            }
            CommandAction::Quit => {
                self.accept_composer_input();
                self.request_exit()
            }
            CommandAction::NewSession => {
                self.accept_composer_input();
                Some(Action::Reconfigure(PaletteCommand::NewSession))
            }
            CommandAction::Resume(None) => {
                self.accept_composer_input();
                self.open_target_picker(ResourceTarget::Resume, restore);
                None
            }
            CommandAction::Account(None) => {
                self.accept_composer_input();
                self.open_target_picker(ResourceTarget::Account, restore);
                None
            }
            CommandAction::Account(Some(value)) => {
                self.accept_composer_input();
                // The argument is the number the picker shows, which is
                // 1-based; pool positions are not.
                let listed = value.trim().parse::<usize>().ok().filter(|n| *n > 0);
                let Some(position) = listed.map(|n| n - 1) else {
                    self.transcript
                        .push_error("name an account by its number, as `/account 2`");
                    return None;
                };
                let entry = self
                    .resources
                    .accounts
                    .iter()
                    .find(|entry| entry.id == position.to_string());
                match entry {
                    Some(entry) if entry.active => {
                        self.transcript
                            .push_notice("account", "already using that account");
                        None
                    }
                    Some(entry) => {
                        // A spent account stays selectable: the cooldown is
                        // Smith's estimate, and the user may know better.
                        if let Some(reason) = &entry.disabled_reason {
                            self.transcript.push_notice("account", reason.clone());
                        }
                        Some(Action::Reconfigure(PaletteCommand::Account(position)))
                    }
                    None => {
                        self.transcript
                            .push_error(format!("this provider has no account {value}"));
                        None
                    }
                }
            }
            CommandAction::Resume(Some(id)) => {
                let selectable = self
                    .resources
                    .sessions
                    .iter()
                    .any(|entry| entry.id == id && entry.disabled_reason.is_none());
                if selectable {
                    self.accept_composer_input();
                    self.apply_resource_selection(ResourceTarget::Resume, id, restore)
                } else {
                    self.transcript.push_error(format!(
                        "session `{id}` is not available for this project; use `/resume` to choose"
                    ));
                    None
                }
            }
            CommandAction::Profile(None) => {
                self.accept_composer_input();
                self.open_target_picker(ResourceTarget::Profile, restore);
                None
            }
            CommandAction::Profile(Some(name)) => {
                let selectable = self
                    .resources
                    .profiles
                    .iter()
                    .find(|entry| {
                        (entry.id == name
                            || entry.id.strip_prefix(LEGACY_AGENT_PROFILE_PREFIX)
                                == Some(name.as_str()))
                            && entry.disabled_reason.is_none()
                    })
                    .map(|entry| entry.id.clone());
                if let Some(id) = selectable {
                    self.accept_composer_input();
                    self.apply_resource_selection(ResourceTarget::Profile, id, restore)
                } else {
                    self.transcript.push_error(format!(
                        "profile `{name}` is not locally selectable; use `/profile` to choose"
                    ));
                    None
                }
            }
            CommandAction::Provider(None) => {
                self.accept_composer_input();
                self.open_target_picker(ResourceTarget::Provider, restore);
                None
            }
            CommandAction::Provider(Some(name)) => {
                let selectable = self
                    .resources
                    .providers
                    .iter()
                    .any(|entry| entry.id == name && entry.disabled_reason.is_none());
                if selectable {
                    let has_model = self.resources.models.iter().any(|entry| {
                        model_pair(&self.resources.providers, &entry.id)
                            .is_some_and(|(provider, _)| provider == name)
                    });
                    if !has_model {
                        return self.apply_resource_selection(
                            ResourceTarget::Provider,
                            name,
                            restore,
                        );
                    }
                    self.accept_composer_input();
                    self.apply_resource_selection(ResourceTarget::Provider, name, restore)
                } else {
                    self.transcript.push_error(format!(
                        "provider `{name}` is not locally selectable; run `smith setup add-provider`"
                    ));
                    None
                }
            }
            CommandAction::Connect(None) => {
                self.accept_composer_input();
                self.open_target_picker(ResourceTarget::Connect, restore);
                None
            }
            CommandAction::Connect(Some(name)) => {
                let selectable = self
                    .resources
                    .connections
                    .iter()
                    .any(|entry| entry.id == name && entry.disabled_reason.is_none());
                if selectable {
                    self.accept_composer_input();
                    self.apply_resource_selection(ResourceTarget::Connect, name, restore)
                } else {
                    self.transcript.push_error(format!(
                        "connection `{name}` is unavailable; use `/connect` to choose"
                    ));
                    None
                }
            }
            CommandAction::Disconnect(None) => {
                self.accept_composer_input();
                self.open_target_picker(ResourceTarget::Disconnect, restore);
                None
            }
            CommandAction::Disconnect(Some(name)) => {
                let selectable = self
                    .resources
                    .disconnections
                    .iter()
                    .any(|entry| entry.id == name && entry.disabled_reason.is_none());
                if selectable {
                    self.accept_composer_input();
                    self.apply_resource_selection(ResourceTarget::Disconnect, name, restore)
                } else {
                    self.transcript.push_error(format!(
                        "connection `{name}` is not active; use `/disconnect` to choose"
                    ));
                    None
                }
            }
            CommandAction::Model(None) => {
                self.accept_composer_input();
                self.open_target_picker(ResourceTarget::Model, restore);
                None
            }
            CommandAction::Model(Some(name)) => self.direct_model(&name, restore),
            CommandAction::Think(None) => {
                self.accept_composer_input();
                self.open_target_picker(ResourceTarget::Think, restore);
                None
            }
            CommandAction::Think(Some(value)) => {
                self.apply_direct_reasoning_choice(ResourceTarget::Think, &value, restore)
            }
            CommandAction::Effort(None) => {
                self.accept_composer_input();
                self.open_target_picker(ResourceTarget::Effort, restore);
                None
            }
            CommandAction::Effort(Some(value)) => {
                self.apply_direct_reasoning_choice(ResourceTarget::Effort, &value, restore)
            }
            CommandAction::AgentResume(child_id) => {
                if self.is_busy() {
                    self.overlay = None;
                    self.transcript.push_notice(
                        "agent",
                        "exact child resume requires an idle root turn; draft preserved",
                    );
                    return None;
                }
                let Some(summary) = self.children.get(&child_id) else {
                    self.transcript.push_error(format!(
                        "No child named `{child_id}`; use `/agent` to list retained children."
                    ));
                    return None;
                };
                let resumable = summary.state == "interrupted"
                    && summary.detail.as_deref().is_some_and(|detail| {
                        detail.contains("resumable") || detail.contains("exact resume available")
                    });
                if !resumable {
                    self.transcript.push_error(format!(
                        "`{child_id}` has no compatible interrupted checkpoint; inspect it with `/agent {child_id}`"
                    ));
                    return None;
                }
                self.accept_composer_input();
                self.overlay = Some(Overlay::AgentResumeConfirm {
                    child_id: child_id.clone(),
                    content: format!(
                        "child: {child_id}\noperation: continue exact interrupted checkpoint\nnew task: no\nturn slot consumed: no\nprovider spend: may continue\nside effects: committed work is not replayed"
                    ),
                });
                None
            }
            CommandAction::Goal(GoalAction::Pause) => {
                if self.is_busy() {
                    self.status.activity = Activity::Interrupting;
                }
                self.accept_composer_input();
                Some(Action::Command(CommandAction::Goal(GoalAction::Pause)))
            }
            local => {
                self.accept_composer_input();
                Some(Action::Command(local))
            }
        }
    }

    /// Appends bounded informational command output to the transcript.
    pub fn show_local_result(&mut self, title: impl Into<String>, content: impl Into<String>) {
        self.follow_newest();
        self.transcript
            .push_local_result(title, content, LocalResultState::Info);
    }

    /// Appends an explicit empty informational result to the transcript.
    pub fn show_local_empty(&mut self, title: impl Into<String>, content: impl Into<String>) {
        self.follow_newest();
        self.transcript
            .push_local_result(title, content, LocalResultState::Empty);
    }

    /// Appends a titled local command failure to the transcript.
    pub fn show_local_error(&mut self, title: impl Into<String>, content: impl Into<String>) {
        self.follow_newest();
        self.transcript
            .push_local_result(title, content, LocalResultState::Error);
    }

    /// Shows an exact undo preview with no default action.
    pub fn confirm_undo(&mut self, content: impl Into<String>) {
        self.overlay = Some(Overlay::UndoConfirm {
            content: content.into(),
        });
    }

    /// Shows an exact redo preview with no default action.
    pub fn confirm_redo(&mut self, content: impl Into<String>) {
        self.overlay = Some(Overlay::RedoConfirm {
            content: content.into(),
        });
    }

    /// Shows an exact selective-revert preview with no default action.
    pub fn confirm_revert(
        &mut self,
        scope: impl Into<String>,
        fingerprint: impl Into<String>,
        content: impl Into<String>,
    ) {
        self.overlay = Some(Overlay::RevertConfirm {
            scope: scope.into(),
            fingerprint: fingerprint.into(),
            content: content.into(),
        });
    }

    /// Shows one MCP server's resolved invocation and content identity, with no
    /// default action: a repository asking Smith to run a program is exactly
    /// the decision that must never be made by pressing Enter.
    pub fn confirm_mcp_trust(&mut self, server: impl Into<String>, content: impl Into<String>) {
        self.overlay = Some(Overlay::McpTrustConfirm {
            server: server.into(),
            content: content.into(),
        });
    }

    /// Shows review scope and provider spend before dispatch.
    pub fn confirm_review(&mut self, scope: impl Into<String>, content: impl Into<String>) {
        self.overlay = Some(Overlay::ReviewConfirm {
            scope: scope.into(),
            content: content.into(),
        });
    }
}

/// Routes a profile choice to the legacy root-mode override when the entry is
/// a transition-release adapter, and to the unified profile path otherwise.
fn profile_palette_command(id: String) -> PaletteCommand {
    match id.strip_prefix(LEGACY_AGENT_PROFILE_PREFIX) {
        Some(agent) => PaletteCommand::Agent(agent.to_owned()),
        None => PaletteCommand::Profile(id),
    }
}

/// Splits a provider-qualified model ID without assuming provider names cannot
/// themselves contain `/`. The local provider inventory is authoritative; the
/// first-slash fallback only keeps synthetic test inventories useful.
fn model_pair(providers: &[ResourceEntry], id: &str) -> Option<(String, String)> {
    if let Some(provider) = providers
        .iter()
        .filter(|entry| {
            id.strip_prefix(&entry.id)
                .is_some_and(|suffix| suffix.starts_with('/') && suffix.len() > 1)
        })
        .max_by_key(|entry| entry.id.len())
    {
        let model = id.strip_prefix(&provider.id)?.strip_prefix('/')?;
        return Some((provider.id.clone(), model.to_owned()));
    }
    let (provider, model) = id.split_once('/')?;
    (!provider.is_empty() && !model.is_empty()).then(|| (provider.to_owned(), model.to_owned()))
}
