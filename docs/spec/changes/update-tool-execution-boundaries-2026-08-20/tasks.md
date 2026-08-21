---
created_at: 2026-08-20T22:13:02Z
updated_at: 2026-08-21T01:33:37Z
completed_at:
---

## 0. Safety Baseline

- [x] 0.1 Add regression tests proving the current shell resource is not a
  workspace-containment grant and that an unrestricted host-shell prepared
  call always reaches approval unless the entire run uses explicit
  `allow-all`.
- [x] 0.2 Reject every non-empty legacy `approval.auto_approve` list with a
  bounded migration diagnostic before session state, terminal entry, provider
  I/O, or tool execution.

## 1. Truthful Host Shell

- [x] 1.1 Introduce a stable `host-shell` security resource whose identifier
  binds the prepared command, cwd, background flag, timeout, and environment
  policy without persisting raw secret values.
- [x] 1.2 Change `ShellTool::prepare` and `SmithToolAuthority` so the shell's
  filesystem permissions are recognized as same-user host authority rather
  than rejected as a workspace resource mismatch.
- [x] 1.3 Update approval displays, tool documentation, and security guidance
  to state inherited environment, host filesystem, child-process, network,
  and egress reach explicitly.
- [x] 1.4 Test foreground and background shell preparation, headless refusal,
  interactive prompting, explicit `allow-all`, and attempts to read outside
  the project.

## 2. Capability Workspace

- [x] 2.1 Introduce a Smith-owned directory capability rooted at the canonical
  project and expose handle-relative open, metadata, create, replace, rename,
  remove, and directory-sync operations without ambient absolute-path access.
- [x] 2.2 Implement macOS and Linux backends that refuse symlink/magic-link
  escape and component replacement races; keep canonical paths only for
  display, attribution, and preliminary diagnostics.
- [x] 2.3 Convert bounded reads to open once, inspect metadata on that handle,
  read through a `max_bytes + 1` limiter, and return the bytes plus a version
  token from the same handle.
- [x] 2.4 Migrate built-in read, list, search, and edit filesystem syscalls to
  the capability API and add adversarial symlink/component-swap tests.
- [x] 2.5 Remove the ordinary-tool out-of-workspace pathname exception and
  replace its approval tests with fail-closed containment tests; document the
  separately labeled host-shell escape hatch.

## 3. Versioned Metadata-Preserving Edit Commit

- [x] 3.1 Extend full-read observations with canonical display path, platform
  file identity, size, modification time, and a content hash captured by the
  same open handle that supplied the returned bytes.
- [x] 3.2 Carry the expected version into the prepared destructive call and
  revalidate it at invocation immediately before overwrite rename or delete;
  reject a mismatch as stale without changing the target.
- [x] 3.3 Apply the same identity/version precondition to exact replacement
  between its read and publication so another writer cannot be overwritten.
- [x] 3.4 Snapshot and restore required existing-file metadata on the sibling
  temporary file before publication. Preserve permission/executable bits and
  implement documented fail-closed handling for supported ACLs, extended
  attributes, ownership-related metadata, and platform labels.
- [x] 3.5 Sync the temporary file before rename and the parent directory after
  rename where supported; retain cleanup on every failed path.
- [x] 3.6 Add tests for executable-bit preservation, metadata-copy failure,
  user edits during an approval delay, equal-mtime changed contents, target
  replacement, delete races, and crash-durability ordering.

## 4. Scoped Auto-Approval

- [x] 4.1 Add a versioned `AutoApprovalRule` configuration model containing a
  module-qualified tool ID, allowed operations, permission ceiling, risk
  ceiling, workspace mount/path patterns, optional expiry, and optional maximum
  uses.
- [x] 4.2 Resolve rules with source provenance and preserve the existing rule
  that repository-controlled layers cannot grant approval authority.
- [x] 4.3 Match rules against the immutable prepared call, never raw arguments.
  Require tool/operation/resource matches and a permission/risk subset; consume
  use bounds atomically.
- [x] 4.4 Make host-root resources, arbitrary process execution, network,
  credential use, data egress, unclassified resources, and outside-workspace
  deletion categorically ineligible for scoped auto-approval.
- [x] 4.5 Add migration guidance and examples for workspace-scoped `edit`
  `replace`/`create` rules. Do not provide a scoped host-shell migration; point
  already-isolated automation to explicit `allow-all`.
- [x] 4.6 Test operation changes, path escape, symlink swaps, permission
  widening, risk increases, expiry, use exhaustion, schema revisions, and
  fallback approval behavior.

## 5. Verification

- [ ] 5.1 Run formatting, Clippy, workspace tests, all-features tests, and the
  Agent Runtime Smith consumer-conformance gate on macOS and Linux.
- [x] 5.2 Update security and configuration references and record focused live
  evidence for host-shell approval and executable-file editing.

Local macOS evidence (2026-08-20): the through-runtime host-shell boundary
test executed an approved host read, the capability-workspace integration test
proved `allow-all` cannot widen ordinary file reads, and the edit integration
suite preserved executable mode across atomic replacement. Linux CI remains
the outstanding part of 5.1.
