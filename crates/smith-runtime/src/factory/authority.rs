//! Workspace and approval-authority resolution stage.

use super::*;
use async_trait::async_trait;

pub(super) struct Stage {
    pub(super) workspace: Arc<dyn Workspace>,
    pub(super) approval: Arc<dyn ApprovalPolicy>,
}

pub(super) fn prepare(request: &RuntimeRequest) -> Result<Stage, FactoryError> {
    Ok(Stage {
        workspace: require_workspace(request)?,
        approval: approval(request)?,
    })
}

pub(super) fn require_workspace(
    request: &RuntimeRequest,
) -> Result<Arc<dyn Workspace>, FactoryError> {
    request
        .workspace
        .clone()
        .ok_or_else(|| FactoryError::MissingHostPolicy {
            what: "workspace",
            message: "tools resolve every path through it, so a run without one could only \
                      deny each call with no explanation"
                .to_owned(),
        })
}

/// Chooses the approval policy.
///
/// An injected surface always wins. Without one, `deny` and `allow-all` are
/// complete answers on their own, but `ask` is not: the run would either hang
/// on a question nobody receives or deny every call without saying why.
pub(super) fn approval(request: &RuntimeRequest) -> Result<Arc<dyn ApprovalPolicy>, FactoryError> {
    let policy: Arc<dyn ApprovalPolicy> = if let Some(policy) = request.approval.clone() {
        policy
    } else {
        match request.config.approval.mode.value {
            ApprovalMode::Deny => Arc::new(DenyAll),
            ApprovalMode::AllowAll => Arc::new(AllowAll),
            ApprovalMode::Ask => {
                return Err(FactoryError::MissingHostPolicy {
                    what: "approval surface",
                    message: format!(
                        "`approval.mode` is `{}`, so the host must supply what answers the question; \
                         set `approval.mode` to `{}` or `{}` for an unattended run",
                        ApprovalMode::Ask.as_str(),
                        ApprovalMode::Deny.as_str(),
                        ApprovalMode::AllowAll.as_str()
                    ),
                });
            }
        }
    };

    if request.config.approval.auto.is_empty() {
        Ok(policy)
    } else {
        let workspace_mount =
            request
                .workspace
                .as_ref()
                .ok_or_else(|| FactoryError::MissingHostPolicy {
                    what: "workspace",
                    message: "scoped automatic approval requires the exact workspace mount"
                        .to_owned(),
                })?;
        let rules = request
            .config
            .approval
            .auto
            .iter()
            .map(ScopedAutoApprovalRule::compile)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Arc::new(ScopedAutoApprove {
            workspace_mount: workspace_mount.root().to_owned(),
            rules,
            fallback: policy,
        }))
    }
}

/// Allows only exact, bounded prepared calls before consulting the host gate.
#[derive(Debug)]
struct ScopedAutoApprove {
    workspace_mount: String,
    rules: Vec<ScopedAutoApprovalRule>,
    fallback: Arc<dyn ApprovalPolicy>,
}

#[async_trait]
impl ApprovalPolicy for ScopedAutoApprove {
    async fn decide(
        &self,
        request: &agent_runtime_core::approval::ApprovalRequest,
    ) -> agent_runtime_core::approval::ApprovalDecision {
        if self.rules.iter().any(|rule| {
            rule.matches_and_consumes(request.prepared(), self.workspace_mount.as_str())
        }) {
            agent_runtime_core::approval::ApprovalDecision::Allow
        } else {
            self.fallback.decide(request).await
        }
    }
}

#[derive(Debug)]
pub(super) struct ScopedAutoApprovalRule {
    tool: String,
    operations: BTreeSet<String>,
    permission_ceiling: PermissionSet,
    max_risk: AutoApprovalRisk,
    mount: AutoApprovalMount,
    paths: globset::GlobSet,
    expires_at_unix_ms: Option<i128>,
    remaining_uses: Option<AtomicU32>,
}

impl ScopedAutoApprovalRule {
    pub(super) fn compile(rule: &Sourced<AutoApprovalRule>) -> Result<Self, FactoryError> {
        let mut paths = globset::GlobSetBuilder::new();
        for pattern in &rule.value.paths {
            let pattern = globset::GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map_err(|error| {
                    FactoryError::Runtime(RuntimeError::config(format!(
                        "resolved automatic approval rule contains an invalid path pattern: {error}"
                    )))
                })?;
            paths.add(pattern);
        }
        let paths = paths.build().map_err(|error| {
            FactoryError::Runtime(RuntimeError::config(format!(
                "resolved automatic approval path set is invalid: {error}"
            )))
        })?;
        Ok(Self {
            tool: rule.value.tool.clone(),
            operations: rule
                .value
                .operations
                .iter()
                .map(|operation| operation.as_str().to_owned())
                .collect(),
            permission_ceiling: rule
                .value
                .permissions
                .iter()
                .copied()
                .map(auto_approval_permission)
                .collect(),
            max_risk: rule.value.max_risk,
            mount: rule.value.mount,
            paths,
            expires_at_unix_ms: rule.value.expires_at_unix_ms,
            remaining_uses: rule.value.max_uses.map(AtomicU32::new),
        })
    }

    pub(super) fn matches_and_consumes(
        &self,
        prepared: &agent_runtime_core::tool::PreparedToolCall,
        workspace_mount: &str,
    ) -> bool {
        if self.tool != format!("smith/{}", prepared.tool())
            || self.is_expired()
            || categorically_ineligible(prepared)
        {
            return false;
        }
        let Some(operation) = prepared
            .arguments()
            .get("operation")
            .and_then(|v| v.as_str())
        else {
            return false;
        };
        if !self.operations.contains(operation)
            || !prepared
                .required_permissions()
                .is_subset(&self.permission_ceiling)
            || prepared_risk(prepared.required_permissions()) > self.max_risk
        {
            return false;
        }
        let SecurityResource::Filesystem { mount, segments } = prepared.resource() else {
            return false;
        };
        if self.mount != AutoApprovalMount::Workspace
            || mount != workspace_mount
            || segments.iter().any(|segment| {
                segment.is_empty()
                    || matches!(segment.as_str(), "." | "..")
                    || segment.contains('/')
            })
            || !self.paths.is_match(segments.join("/"))
        {
            return false;
        }
        self.consume_use()
    }

    fn is_expired(&self) -> bool {
        self.expires_at_unix_ms.is_some_and(|expires_at| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| {
                    i128::try_from(duration.as_millis()).unwrap_or(i128::MAX)
                });
            now > expires_at
        })
    }

    fn consume_use(&self) -> bool {
        let Some(remaining) = &self.remaining_uses else {
            return true;
        };
        remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_sub(1)
            })
            .is_ok()
    }
}

pub(super) fn categorically_ineligible(
    prepared: &agent_runtime_core::tool::PreparedToolCall,
) -> bool {
    let effects = prepared.effects();
    effects.spawns_process()
        || effects.has_network()
        || effects.has_data_egress()
        || effects.host_read_kinds().next().is_some()
        || effects.host_writes().next().is_some()
        || effects.external_reads().next().is_some()
        || effects.external_writes().next().is_some()
        || prepared.required_permissions().iter().any(|permission| {
            !matches!(
                permission,
                Permission::FsRead
                    | Permission::FsWrite
                    | Permission::FsCreate
                    | Permission::FsDelete
            )
        })
}

pub(super) fn prepared_risk(permissions: &PermissionSet) -> AutoApprovalRisk {
    if permissions.contains(&Permission::FsDelete) {
        AutoApprovalRisk::High
    } else if permissions.contains(&Permission::FsWrite)
        || permissions.contains(&Permission::FsCreate)
    {
        AutoApprovalRisk::Medium
    } else if permissions.contains(&Permission::FsRead) {
        AutoApprovalRisk::Low
    } else {
        AutoApprovalRisk::None
    }
}

pub(super) fn auto_approval_permission(permission: AutoApprovalPermission) -> Permission {
    match permission {
        AutoApprovalPermission::FsRead => Permission::FsRead,
        AutoApprovalPermission::FsWrite => Permission::FsWrite,
        AutoApprovalPermission::FsCreate => Permission::FsCreate,
        AutoApprovalPermission::FsDelete => Permission::FsDelete,
        AutoApprovalPermission::HostFsRead => Permission::HostFsRead,
        AutoApprovalPermission::HostFsWrite => Permission::HostFsWrite,
        AutoApprovalPermission::ExternalRead => Permission::ExternalRead,
        AutoApprovalPermission::ExternalWrite => Permission::ExternalWrite,
        AutoApprovalPermission::ProcessSpawn => Permission::ProcessSpawn,
        AutoApprovalPermission::NetHttp => Permission::NetHttp,
        AutoApprovalPermission::DataEgress => Permission::DataEgress,
        AutoApprovalPermission::CredentialUse => Permission::CredentialUse,
        AutoApprovalPermission::StdioRead => Permission::StdioRead,
        AutoApprovalPermission::StdioWrite => Permission::StdioWrite,
        AutoApprovalPermission::ClockRead => Permission::ClockRead,
        AutoApprovalPermission::RandomRead => Permission::RandomRead,
    }
}
