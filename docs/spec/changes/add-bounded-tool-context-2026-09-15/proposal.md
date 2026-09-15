# Bounded tool evidence in Smith

## Approval and scope

Implements PR A of the context audit/implementation sequence approved by the user
on 2026-09-15. Keep this change independent of the existing resume/status PR.

## Why

Root sessions coupled exact-output offloading to the 64 KiB model-output ceiling;
child sessions used Runtime's default threshold instead of the resolved host
policy. Repeated medium results therefore occupied the active request even though
Smith already had authorized artifact storage. Artifact expansion defaulted to
16 KiB and could immediately undo the savings.

## Change

Add one layered setting, `context.tool_output_inline_bytes` (default 8192), for
text-only outcome offloading. Keep the older model-rendering ceiling and the
separate tool/process capture limits intact. Root and child composition share one
resolved policy. Reuse Runtime's immutable archive-before-replace mechanism,
owner checks, digest checks and prepared tool authorization. Present bounded
head/tail and explicitly heuristic diagnostic excerpts, with exact reported status
metadata. Bound artifact expansion during preparation and preserve pagination.

The existing `/context` category/occupancy report remains authoritative and names
the active output policy without conflating cumulative usage with request size.

## Non-goals

This is not semantic compaction or an aggregate request-size guarantee. It does
not change high/low watermark wiring, summarize an active turn, truncate user
requirements, strip signed continuation, install LCM, enable background spending,
or add provider-native compaction or `/compact`. Those are the separate PR B/C
stages of the approved plan. The runtime dependency pin is unchanged.

## Compatibility

Existing configs inherit the new default. Users can raise it, subject to the
existing stricter rendering ceiling. No on-disk schema or artifact format changes.
New tool descriptors change activation identity; merge the separate safe-upgrade
resume fix before shipping this default change. No old approval is transferred.

## Validation

Unit/config tests cover evidence fidelity, identity/idempotency, Unicode, typed
content preservation, failures, bounded preparation and ownership after reopening
storage. Production host tests cover eighteen medium shell results in one task,
subsequent protected-session resume and real artifact-tool retrieval without
replaying effects. Child delegation tests verify inherited limits and ownership.
This measures a deterministic serialized-request fixture, not real token savings
or semantic continuation quality.
