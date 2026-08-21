## Context

At commit `19e1696`, `ShellTool::prepare` requests host-wide filesystem,
process, network, and egress permissions but attaches a workspace filesystem
resource; invocation then runs inherited `sh -c` with only `current_dir`
changed. `AutoApprove` compares only `prepared.tool()`. `write_atomically`
publishes a new inode without copying metadata, while destructive read-state
validation occurs during preparation and uses modification time only. The
workspace resolves path strings before later independent syscalls, and
`read_bounded` performs metadata and read through separate pathname lookups.

## Goals / Non-Goals

- Goals:
  - Make declared authority match executable authority.
  - Make automatic approval a subset check over an immutable prepared action.
  - Prevent pathname replacement and concurrent edits from changing the object
    a validated built-in operation acts upon.
  - Preserve the security-relevant metadata contract of an existing file.
- Non-Goals:
  - No host-shell sandbox in this change.
  - No arbitrary out-of-workspace filesystem API for built-in tools.
  - No compatibility mode that silently retains tool-name-only auto-approval.

## Decisions

### Decision: declare the current shell as trusted host execution

The prepared resource is `other("host-shell", action_revision)`, and the
approval display states the ambient same-user authority. The action revision
binds canonical prepared fields while the existing preparation fingerprint
continues to protect the entire call.

An OS sandbox was considered. It remains desirable, but Linux and macOS need
different enforceable helpers, mount policies, environment rules, and network
controls. Relabeling the current process is the only immediate fix that does
not overstate containment.

### Decision: auto-approval is a deny-by-default prepared-call matcher

Rules are host policy, not tool configuration. A rule matches only if every
prepared permission and effect is inside its ceiling and the concrete resource
is covered. Unknown fields, resource kinds, permissions, operations, and risk
levels fail closed. An ordinary approval policy remains the fallback.

Tool-name lists were considered as migration aliases. They cannot express the
difference between `edit` replacement, overwrite, delete, workspace escape, or
future widened authority, so non-empty legacy values are rejected rather than
guessed into grants.

### Decision: use directory-relative capabilities as the enforcement primitive

`ProjectWorkspace` owns a root directory handle. Built-in tools traverse and
act relative to it with no-follow/beneath semantics, then retain handles for
metadata, reading, and commit. Canonical string paths remain useful for display
and attribution but are not the final authority check.

### Decision: versions are content- and identity-bound

The read result carries a version token produced from the same handle as the
returned bytes. On supported Unix targets the identity includes device and
inode; the token also includes size, modification time, and a cryptographic
content hash. Commit compares a freshly opened handle under the same directory
capability before publishing. Equal timestamps cannot authorize changed bytes.

### Decision: replacement preserves an explicit metadata set

The temporary file receives required metadata before publication. Permission
and executable bits are mandatory. Platform ACLs, extended attributes,
ownership-related metadata, and labels use a platform adapter with a documented
support matrix; required metadata that cannot be preserved causes refusal
instead of silent loss. The temp file is synced before rename and the parent is
synced after rename where supported.

## Risks / Trade-offs

- Existing unattended configurations using `approval.auto_approve` stop at
  preflight until migrated. This is intentional because preserving them would
  preserve ambiguous authority.
- Handle-relative filesystem APIs add platform-specific code and dependencies;
  the workspace forbids unsafe code, so wrappers must provide safe interfaces.
- Hashing full-read content adds bounded CPU cost. The tool already reads the
  bytes and enforces a size limit, so hashing is incremental and bounded.
- ACL/xattr preservation differs by filesystem. The support matrix and refusal
  behavior must be visible rather than inferred.

## Migration Plan

1. Release truthful shell labeling and legacy-list refusal behind no feature
   flag; both are security corrections.
2. Introduce the capability workspace internally and migrate tools without
   changing user-facing paths.
3. Replace read observations and edit publication atomically in one release so
   no mixed timestamp/version protocol exists.
4. Enable the typed `[[approval.auto]]` schema after its matcher is covered by
   adversarial tests. Update examples and `config explain` output.

## Open Questions

- Select the safe, Rust-1.88-compatible directory-capability implementation
  after a short macOS/Linux spike. The acceptance criteria, not a particular
  crate, are normative.
- Define which extended metadata is mandatory on each supported filesystem and
  whether unsupported metadata makes an edit fail or is reported as a bounded
  warning. Permission/executable-bit preservation is not optional.
