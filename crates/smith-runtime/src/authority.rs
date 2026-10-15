//! Smith's authoritative policy for its prepared built-in tools.
//!
//! Agent Runtime owns composition and enforcement. Smith owns the product
//! answer for the concrete authority its tools advertise: exact project reads
//! may proceed unattended, while every write/create/delete, process, network,
//! or data-egress action — and any filesystem access outside the project
//! root — remains eligible only after the configured approval policy answers.
//!
//! Polling a background task's spool (`task_output`) joins session-artifact
//! reads on the unattended side. It reads runtime-owned session state the
//! agent already caused by starting the task, addressed by task ID rather
//! than by a path the model chooses, so prompting on every poll would ask the
//! user to re-approve output they already authorized producing.

use agent_runtime::harness::ARTIFACT_READ_PERMISSION;
use agent_runtime::registry::Permission;
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::grant::{
    DecisionCode, GrantConstraints, SecurityCheck, SecurityCheckId, SecurityCheckOutcome,
    SecurityCheckRevision,
};
use agent_runtime_core::security::{AuthorizationRequest, PermissionSet, SecurityResource};
use async_trait::async_trait;
use smith_tools::{BACKGROUND_TASK_RESOURCE_KIND, HOST_SHELL_RESOURCE_KIND};

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
            revision: SecurityCheckRevision::new("v5"),
            workspace_mount: workspace_mount.into(),
            coverage: [
                Permission::FsRead,
                Permission::FsWrite,
                Permission::FsCreate,
                Permission::FsDelete,
                Permission::HostFsRead,
                Permission::HostFsWrite,
                Permission::ExternalRead,
                Permission::ExternalWrite,
                Permission::ProcessSpawn,
                Permission::StdioRead,
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
        if filesystem_permission {
            match &request.resource {
                SecurityResource::Filesystem { mount, .. } => {
                    if mount != &self.workspace_mount {
                        // Outside the project boundary: never unattended,
                        // whatever the permission set. The approval policy —
                        // a prompt, allow-all, or deny — makes the call.
                        return SecurityCheckOutcome::RequireApproval {
                            constraints: GrantConstraints::unconstrained(),
                        };
                    }
                }
                _ => {
                    return SecurityCheckOutcome::Deny {
                        code: DecisionCode::other("smith.workspace_resource_mismatch"),
                    };
                }
            }
        }

        let host_filesystem_permission = request.requested.iter().any(|permission| {
            matches!(permission, Permission::HostFsRead | Permission::HostFsWrite)
        });
        if host_filesystem_permission {
            if !matches!(
                &request.resource,
                SecurityResource::Other { kind, id }
                    if kind == HOST_SHELL_RESOURCE_KIND
                        || (kind == "smith:image-files"
                            && (!request.requested.contains(&Permission::HostFsWrite)
                                || id == "smith:generated-images"))
            ) {
                return SecurityCheckOutcome::Deny {
                    code: DecisionCode::other("smith.host_resource_mismatch"),
                };
            }
            // The shell is intentionally host-wide. Image reads may include
            // Smith's generated-image directory; image writes are bound to
            // that directory's exact resource id. All require approval.
            return SecurityCheckOutcome::RequireApproval {
                constraints: GrantConstraints::unconstrained(),
            };
        }

        let external_permission = request.requested.iter().any(|permission| {
            matches!(
                permission,
                Permission::ExternalRead | Permission::ExternalWrite
            )
        });
        if external_permission {
            if !matches!(
                &request.resource,
                SecurityResource::Other { kind, .. } if kind == "external-service"
            ) {
                return SecurityCheckOutcome::Deny {
                    code: DecisionCode::other("smith.external_resource_mismatch"),
                };
            }
            return SecurityCheckOutcome::RequireApproval {
                constraints: GrantConstraints::unconstrained(),
            };
        }

        // Stdio authority is Smith's background-task spool and nothing else:
        // bound to the task resource so it can never be borrowed to read some
        // other stream.
        let stdio_permission = request.requested.contains(&Permission::StdioRead);
        if stdio_permission
            && !matches!(
                &request.resource,
                SecurityResource::Other { kind, .. } if kind == BACKGROUND_TASK_RESOURCE_KIND
            )
        {
            return SecurityCheckOutcome::Deny {
                code: DecisionCode::other("smith.background_task_resource_mismatch"),
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
        let task_output = request.requested.len() == 1 && stdio_permission;
        if read_only || artifact_read || task_output {
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
    use std::sync::Arc;

    use agent_runtime::registry::{Fingerprint, TrustClass};
    use agent_runtime_core::clock::{Deadline, SystemClock};
    use agent_runtime_core::error::RuntimeError;
    use agent_runtime_core::ids::{RequestId, SessionId, TenantId, ToolCallId};
    use agent_runtime_core::security::{
        CheckSetRevision, SecurityAction, SecurityContext, SecurityEvidence, SecuritySubject,
    };
    use agent_runtime_core::tool::{InvocationContext, PreparationContext, Tool};
    use serde_json::json;
    use smith_tools::{
        GenerateImageTool, ImageGenerationBackend, ImageGenerationRequest, RecentImageSource,
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

    #[derive(Debug)]
    struct UnusedImageBackend;

    #[async_trait]
    impl ImageGenerationBackend for UnusedImageBackend {
        async fn generate(
            &self,
            _: ImageGenerationRequest,
            _: &InvocationContext,
        ) -> Result<Vec<u8>, RuntimeError> {
            panic!("preparation must not call the image backend")
        }

        fn endpoint(&self) -> &str {
            "https://images.example/v1"
        }
    }

    #[derive(Debug)]
    struct NoRecentImages;

    impl RecentImageSource for NoRecentImages {
        fn recent_images(&self, _: &SessionId, _: usize) -> Result<Vec<String>, RuntimeError> {
            panic!("preparation must not read recent images")
        }
    }

    #[tokio::test]
    async fn prepared_generate_image_requires_approval_with_and_without_references() {
        let temporary = tempfile::tempdir().unwrap();
        let workspace = Arc::new(smith_host::ProjectWorkspace::new(temporary.path()).unwrap());
        let tool = GenerateImageTool::new(
            Arc::new(UnusedImageBackend),
            Arc::new(NoRecentImages),
            temporary.path().join("generated_images"),
            "gpt-image-2",
            "auto",
            "auto",
        );
        let context = PreparationContext {
            session: SessionId::new("session"),
            turn: None,
            call_id: ToolCallId::new("image"),
            request: RequestId::new("request"),
            workspace,
            clock: Arc::new(SystemClock),
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
        };
        let authority = SmithToolAuthority::new(context.workspace.root());
        for arguments in [
            json!({"prompt":"A small blue bird"}),
            json!({"prompt":"A small blue bird", "reference_paths":["reference.png"]}),
        ] {
            let has_references = arguments.get("reference_paths").is_some();
            let prepared = tool.prepare(arguments, &context).await.unwrap();
            assert_eq!(
                prepared.resource(),
                &SecurityResource::other("smith:image-files", "smith:generated-images"),
            );
            assert_eq!(
                prepared
                    .effects()
                    .host_writes()
                    .map(|(kind, scope)| (kind, scope.as_str()))
                    .collect::<Vec<_>>(),
                [("smith:image-files", "smith:generated-images")],
            );
            for permission in [
                Permission::HostFsWrite,
                Permission::NetHttp,
                Permission::DataEgress,
            ] {
                assert!(prepared.required_permissions().contains(&permission));
            }
            assert_eq!(
                prepared
                    .required_permissions()
                    .contains(&Permission::HostFsRead),
                has_references,
            );
            let outcome = authority
                .evaluate(
                    &request(
                        prepared.required_permissions().iter().cloned(),
                        prepared.resource().clone(),
                    ),
                    &Cancellation::new(),
                )
                .await;
            assert!(
                matches!(outcome, SecurityCheckOutcome::RequireApproval { .. }),
                "generate_image must reach approval: {outcome:?}"
            );
        }
    }

    #[tokio::test]
    async fn image_host_reads_require_approval_without_granting_host_writes() {
        let authority = SmithToolAuthority::new("/repo");
        for permissions in [
            vec![Permission::HostFsRead],
            vec![
                Permission::HostFsRead,
                Permission::NetHttp,
                Permission::DataEgress,
            ],
        ] {
            let outcome = authority
                .evaluate(
                    &request(
                        permissions,
                        SecurityResource::other("smith:image-files", "view_image"),
                    ),
                    &Cancellation::new(),
                )
                .await;
            assert!(matches!(
                outcome,
                SecurityCheckOutcome::RequireApproval { .. }
            ));
        }

        for (permissions, kind) in [
            (vec![Permission::HostFsRead], "unknown-host-resource"),
            (vec![Permission::HostFsWrite], "smith:image-files"),
            (
                vec![Permission::HostFsRead, Permission::HostFsWrite],
                "smith:image-files",
            ),
        ] {
            let outcome = authority
                .evaluate(
                    &request(permissions, SecurityResource::other(kind, "view_image")),
                    &Cancellation::new(),
                )
                .await;
            assert!(matches!(outcome, SecurityCheckOutcome::Deny { .. }));
        }
    }

    #[tokio::test]
    async fn image_host_writes_require_approval_only_for_generated_images() {
        let authority = SmithToolAuthority::new("/repo");
        for permissions in [
            vec![Permission::HostFsWrite],
            vec![Permission::HostFsRead, Permission::HostFsWrite],
            vec![
                Permission::HostFsWrite,
                Permission::NetHttp,
                Permission::DataEgress,
            ],
            vec![
                Permission::HostFsRead,
                Permission::HostFsWrite,
                Permission::NetHttp,
                Permission::DataEgress,
            ],
        ] {
            let outcome = authority
                .evaluate(
                    &request(
                        permissions,
                        SecurityResource::other("smith:image-files", "smith:generated-images"),
                    ),
                    &Cancellation::new(),
                )
                .await;
            assert!(matches!(
                outcome,
                SecurityCheckOutcome::RequireApproval { .. }
            ));
        }

        for (kind, target) in [
            ("unknown-host-resource", "smith:generated-images"),
            ("smith:image-files", "generate_image"),
            ("smith:image-files", "view_image"),
            ("smith:image-files", "/etc/arbitrary.png"),
            ("smith:image-files", "smith:generated-images/../other"),
            ("smith:image-files", "smith:generated-images-other"),
        ] {
            for permissions in [
                vec![Permission::HostFsWrite],
                vec![Permission::HostFsRead, Permission::HostFsWrite],
            ] {
                let outcome = authority
                    .evaluate(
                        &request(permissions, SecurityResource::other(kind, target)),
                        &Cancellation::new(),
                    )
                    .await;
                assert!(
                    matches!(outcome, SecurityCheckOutcome::Deny { code }
                        if code == DecisionCode::other("smith.host_resource_mismatch")),
                    "host write must be denied for {kind}:{target}"
                );
            }
        }
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
                    SecurityResource::filesystem("/", vec!["elsewhere".into(), "secret".into()]),
                ),
                &Cancellation::new(),
            )
            .await;
        assert!(
            matches!(escaped, SecurityCheckOutcome::RequireApproval { .. }),
            "an out-of-workspace read is the user's call, not an unattended allow or a flat deny"
        );

        let structural_mismatch = authority
            .evaluate(
                &request(
                    [Permission::FsRead],
                    SecurityResource::other("project-file", "a-reference"),
                ),
                &Cancellation::new(),
            )
            .await;
        assert!(matches!(
            structural_mismatch,
            SecurityCheckOutcome::Deny { .. }
        ));

        let host_shell = authority
            .evaluate(
                &request(
                    [
                        Permission::HostFsRead,
                        Permission::HostFsWrite,
                        Permission::ProcessSpawn,
                        Permission::NetHttp,
                        Permission::DataEgress,
                    ],
                    SecurityResource::other(HOST_SHELL_RESOURCE_KIND, "sha256:action"),
                ),
                &Cancellation::new(),
            )
            .await;
        assert!(matches!(
            host_shell,
            SecurityCheckOutcome::RequireApproval { .. }
        ));

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

        // Polling a task the agent already started is unattended, or every
        // `task_output` call in a background build would raise a prompt.
        let task_output = authority
            .evaluate(
                &request(
                    [Permission::StdioRead],
                    SecurityResource::other(BACKGROUND_TASK_RESOURCE_KIND, "task_1"),
                ),
                &Cancellation::new(),
            )
            .await;
        assert!(matches!(task_output, SecurityCheckOutcome::Allow { .. }));

        // ...but only over a background task. Stdio authority is not a
        // general licence to read streams.
        let wrong_task_resource = authority
            .evaluate(
                &request(
                    [Permission::StdioRead],
                    SecurityResource::other(HOST_SHELL_RESOURCE_KIND, "sha256:action"),
                ),
                &Cancellation::new(),
            )
            .await;
        assert!(matches!(
            wrong_task_resource,
            SecurityCheckOutcome::Deny { .. }
        ));

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
