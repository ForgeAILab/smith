---
created_at: 2026-08-02T01:10:27Z
updated_at: 2026-08-02T01:10:27Z
---

## Why

Smith exposes explicit unattended execution through `--approval allow-all`,
but the common fast-path spelling `--yolo` is absent. Users want the concise
alias while preserving the stronger invariant that approval can authorize an
available prepared action but can never add capabilities removed by an agent
profile.

## What Changes

- Add `--yolo` as a valueless command-line alias for
  `--approval allow-all`.
- Reject duplicate or conflicting uses of `--yolo` and `--approval` rather
  than selecting one by argument order.
- Document the alias as dangerous outside an already isolated, trusted
  automation boundary.
- Preserve profile capability narrowing: plan/review profiles remain
  read-only under `--yolo`.

## Impact

- Affected spec: `client-surfaces`
- Affected code: `smith-cli` parser/help and CLI contract tests
- Affected docs: README, configuration reference, and security model
- Security: this adds no new approval mode and no configuration authority;
  the alias is an explicit invocation-level spelling of existing `allow-all`

## Approval Boundary

Approval authorizes the exact `--yolo` alias for invocation-level
`allow-all`. It does not authorize a default-on configuration value, implicit
activation, profile/tool authority widening, removal of central authorization,
or write access for `plan`, `review`, or child-agent read-only ceilings.

Approved by the user for implementation on 2026-08-01.
