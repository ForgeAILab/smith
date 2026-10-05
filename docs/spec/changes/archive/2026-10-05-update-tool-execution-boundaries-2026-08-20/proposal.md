---
created_at: 2026-08-20T22:13:02Z
updated_at: 2026-08-20T22:24:01Z
---

## Why

Smith validates workspace paths and prepares precise authority, but several
execution paths currently promise more containment or concurrency safety than
they enforce. A host shell is presented as a workspace filesystem action,
tool-name auto-approval ignores the prepared operation and resource, and edit
publication can lose metadata or overwrite a file changed after approval.

## What Changes

- Model the current `shell` tool truthfully as an unrestricted same-user host
  process action. Its resource and approval display identify inherited host
  filesystem, environment, child-process, network, and egress authority; this
  change does not claim the command is sandboxed.
- **BREAKING** replace tool-name-only `approval.auto_approve` with typed,
  revisioned prepared-call rules. Rules match the module-qualified tool,
  operation, permission subset, risk ceiling, workspace mount/path patterns,
  expiry, and use bound. Unsupported resource kinds and process, network,
  egress, or out-of-workspace authority fail closed.
- Replace pathname-only enforcement in built-in filesystem tools with a
  directory-handle-relative workspace capability. Validation, bounded reads,
  version checks, and publication operate on already-opened handles beneath
  that capability.
- Carry a strong version token from a completed full read to destructive edit
  commit. Revalidate it immediately before overwrite or delete, including file
  identity, size, modification time, and content hash.
- Preserve existing-file metadata during atomic replacement, including at
  least permission and executable bits, and define fail-closed handling for
  platform ACLs and extended attributes. Sync the temporary file and parent
  directory where the filesystem supports durable rename publication.

## Impact

- Affected specs: `tool-execution`, `configuration`
- Affected code: `smith-config` approval schema/resolution, `smith-host`
  workspace capability, `smith-runtime` authority/factory policy,
  `smith-tools` read/edit/shell/support paths, approval presentation, and
  configuration/security documentation
- Compatibility: existing non-empty `approval.auto_approve` string lists fail
  preflight with a migration diagnostic; empty or absent lists remain inert.
  `approval.mode = "allow-all"` remains the explicit unrestricted automation
  choice and must continue to be refused from repository-controlled layers.
- **BREAKING:** built-in read/list/search/edit tools no longer accept an
  absolute or traversed out-of-workspace path after approval. Host filesystem
  access remains available only through an honestly labeled host capability,
  initially the explicitly approved host shell.
- Security posture: this makes current host-shell authority honest and hardens
  built-in filesystem operations. A genuinely workspace-contained shell is a
  separate capability requiring platform sandbox backends and is not implied
  by this change.

## Sequencing

1. Land the truthful shell resource and fail-closed legacy auto-approval guard
   first, so no release continues to advertise the false boundary.
2. Land the workspace capability and versioned read contract.
3. Move edit publication onto that contract and enable typed auto-approval only
   after its matcher and migration tests pass.
