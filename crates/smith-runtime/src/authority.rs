//! Smith's authoritative policy for its prepared built-in tools.
//!
//! Agent Runtime owns composition and enforcement. Smith owns the product
//! answer for the concrete authority its tools advertise: exact project reads
//! may proceed unattended, while every write/create/delete, process, network,
//! or data-egress action remains eligible only after the configured approval
//! policy answers.

use agent_runtime::harness::ARTIFACT_READ_PERMISSION;
use agent_runtime::registry::Permission;
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::grant::{
    DecisionCode, GrantConstraints, SecurityCheck, SecurityCheckId, SecurityCheckOutcome,
    SecurityCheckRevision,
};
use agent_runtime_core::security::{AuthorizationRequest, PermissionSet, SecurityResource};
use async_trait::async_trait;

/// Authoritative coverage for every typed permission Smith's built-ins declare.
#[derive(Debug)]
pub(crate) struct SmithToolAuthority {
    id: SecurityCheckId,
    revision: SecurityCheckRevision,
    workspace_mount: String,
    coverage: PermissionSet,
}

impl SmithToolAuthority {
    pub(crate) fn new(workspace_mount: impl Into<String>) -> Self {
        Self {
            id: SecurityCheckId::new("smith-built-in-tool-authority"),
            revision: SecurityCheckRevision::new("v2"),
            workspace_mount: workspace_mount.into(),
            coverage: [
                Permission::FsRead,
                Permission::FsWrite,
                Permission::FsCreate,
                Permission::FsDelete,
                Permission::ProcessSpawn,
                Permission::NetHttp,
                Permission::DataEgress,
                Permission::other(ARTIFACT_READ_PERMISSION),
            ]
            .into_iter()
            .collect(),
        }
    }

    pub(crate) fn coverage(&self) -> &PermissionSet {
        &self.coverage
    }
}

#[async_trait]
impl SecurityCheck for SmithToolAuthority {
    fn id(&self) -> &SecurityCheckId {
        &self.id
    }

    fn revision(&self) -> &SecurityCheckRevision {
        &self.revision
    }

    fn declared_coverage(&self) -> Option<PermissionSet> {
        Some(self.coverage.clone())
    }

    async fn evaluate(
        &self,
        request: &AuthorizationRequest,
        _cancel: &Cancellation,
    ) -> SecurityCheckOutcome {
        if !request
            .requested
            .iter()
            .any(|permission| self.coverage.contains(permission))
        {
            return SecurityCheckOutcome::NotApplicable;
        }

        let filesystem_permission = request.requested.iter().any(|permission| {
            matches!(
                permission,
                Permission::FsRead
                    | Permission::FsWrite
                    | Permission::FsCreate
                    | Permission::FsDelete
            )
        });
        if filesystem_permission
            && !matches!(
                &request.resource,
                SecurityResource::Filesystem { mount, .. } if mount == &self.workspace_mount
            )
        {
            return SecurityCheckOutcome::Deny {
                code: DecisionCode::other("smith.workspace_resource_mismatch"),
            };
        }

        let artifact_permission = Permission::other(ARTIFACT_READ_PERMISSION);
        let artifact_requested = request.requested.contains(&artifact_permission);
        if artifact_requested
            && !matches!(
                &request.resource,
                SecurityResource::Other { kind, .. } if kind == "session-artifact"
            )
        {
            return SecurityCheckOutcome::Deny {
                code: DecisionCode::other("smith.artifact_resource_mismatch"),
            };
        }

        let read_only = request.requested.len() == 1
            && request.requested.contains(&Permission::FsRead)
            && matches!(
                &request.resource,
                SecurityResource::Filesystem { mount, .. } if mount == &self.workspace_mount
            );
        let artifact_read = request.requested.len() == 1 && artifact_requested;
        if read_only || artifact_read {
            SecurityCheckOutcome::Allow {
                constraints: GrantConstraints::unconstrained(),
            }
        } else {
            SecurityCheckOutcome::RequireApproval {
                constraints: GrantConstraints::unconstrained(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::registry::{Fingerprint, TrustClass};
    use agent_runtime_core::clock::Deadline;
    use agent_runtime_core::ids::{SessionId, TenantId};
    use agent_runtime_core::security::{
        CheckSetRevision, SecurityAction, SecurityContext, SecurityEvidence, SecuritySubject,
    };

    fn request(
        permissions: impl IntoIterator<Item = Permission>,
        resource: SecurityResource,
    ) -> AuthorizationRequest {
        AuthorizationRequest::new(
            SecurityContext::new(
                SecuritySubject::new("smith"),
                SessionId::new("session"),
                TenantId::new("tenant"),
                CheckSetRevision::new("checks"),
            ),
            SecurityAction::new("tool.invoke"),
            resource,
            permissions.into_iter().collect(),
            Deadline::never(),
            SecurityEvidence::new(TrustClass::ExternalContent, Fingerprint::of("arguments")),
        )
    }

    #[tokio::test]
    async fn only_exact_workspace_reads_bypass_approval() {
        let authority = SmithToolAuthority::new("/repo");
        let read = authority
            .evaluate(
                &request(
                    [Permission::FsRead],
                    SecurityResource::filesystem("/repo", vec!["src".into(), "lib.rs".into()]),
                ),
                &Cancellation::new(),
            )
            .await;
        assert!(matches!(read, SecurityCheckOutcome::Allow { .. }));

        let edit = authority
            .evaluate(
                &request(
                    [Permission::FsRead, Permission::FsWrite],
                    SecurityResource::filesystem("/repo", vec!["src".into(), "lib.rs".into()]),
                ),
                &Cancellation::new(),
            )
            .await;
        assert!(matches!(edit, SecurityCheckOutcome::RequireApproval { .. }));

        let escaped = authority
            .evaluate(
                &request(
                    [Permission::FsRead],
                    SecurityResource::filesystem("/elsewhere", vec!["secret".into()]),
                ),
                &Cancellation::new(),
            )
            .await;
        assert!(matches!(escaped, SecurityCheckOutcome::Deny { .. }));

        let artifact = authority
            .evaluate(
                &request(
                    [Permission::other(ARTIFACT_READ_PERMISSION)],
                    SecurityResource::other("session-artifact", "a-reference"),
                ),
                &Cancellation::new(),
            )
            .await;
        assert!(matches!(artifact, SecurityCheckOutcome::Allow { .. }));

        let wrong_artifact_resource = authority
            .evaluate(
                &request(
                    [Permission::other(ARTIFACT_READ_PERMISSION)],
                    SecurityResource::other("project-file", "a-reference"),
                ),
                &Cancellation::new(),
            )
            .await;
        assert!(matches!(
            wrong_artifact_resource,
            SecurityCheckOutcome::Deny { .. }
        ));
    }
}
