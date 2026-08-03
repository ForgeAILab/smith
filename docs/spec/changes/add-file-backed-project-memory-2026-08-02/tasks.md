---
created_at: 2026-08-02T15:43:22Z
updated_at: 2026-08-02T15:54:07Z
completed_at:
---

## 1. Configuration and public contracts

- [ ] 1.1 Add provenance-carrying Smith memory configuration with built-in
  defaults enabled=true and auto_capture=false.
- [ ] 1.2 Reject repository-controlled memory roots, automatic-capture
  enablement, sensitivity downgrades, and other persistence authority during
  resolved-config preflight.
- [ ] 1.3 Define versioned topic, index, store-policy, capture-policy, safe
  status, and canonical event types without exposing memory content in Debug
  or errors.
- [ ] 1.4 Add configuration explanation and user-edit support for enabling or
  disabling memory and opting into automatic capture.

## 2. Private file-backed store

- [ ] 2.1 Resolve ~/.smith/memory/<project-id>/ from the standard host's
  canonical ProjectId without allowing checkout-relative fallback.
- [ ] 2.2 Implement schema-1 Markdown topic parsing and rendering for bounded
  ids, descriptions, keywords, timestamps, types, sensitivity, and bodies.
- [ ] 2.3 Enforce regular-file and symlink containment, UTF-8 and size bounds,
  private directory/file modes, safe error rendering, and registered-secret
  rejection.
- [ ] 2.4 Implement a project-scoped cross-process writer lease,
  same-directory temporary writes, durable atomic topic publication, exact-id
  forget, and conflict-safe failure.
- [ ] 2.5 Generate bounded MEMORY.md deterministically from validated topic
  metadata, including stable ordering and an omission marker when the complete
  catalog does not fit.
- [ ] 2.6 Implement open-time and post-mutation deterministic maintenance for
  stale Smith temporary files, invalid-file reporting, duplicate ids, quotas,
  and index repair without semantic rewriting or deletion.
- [ ] 2.7 Add unit and concurrency tests for create, update, no-op, forget,
  crash repair, lock contention, manual edits, malformed files, symlink
  escapes, quotas, permissions, and secret refusal.

## 3. Recall and host composition

- [ ] 3.1 Evolve SmithMemorySource or add a compatible file-backed source that
  snapshots validated memory at turn boundaries and produces content-derived
  record revisions.
- [ ] 3.2 Implement deterministic index and topic selection from keyword
  matches, description overlap, Smith-owned type priority, update time, and id
  tie-breaking under existing generic and Smith bounds.
- [ ] 3.3 Install the file-backed source in standard terminal and headless host
  construction when memory is enabled while preserving explicit direct
  embedder injection.
- [ ] 3.4 Keep recall in optional Host memory fragments with sensitivity,
  cache, provenance, revision, and token accounting; prove no body is copied
  into canonical history or authority state.
- [ ] 3.5 Make valid external edits visible only at the next turn boundary and
  make resumed sessions use current memory for new turns without rewriting old
  manifests.
- [ ] 3.6 Inherit bounded recall into child runtimes while withholding memory
  mutation abilities and automatic capture.
- [ ] 3.7 Update Smith prompt guidance for memory taxonomy, explicit remember
  and forget requests, non-derivable content, current-state verification, and
  the distinction from plans, goals, tasks, sessions, and AGENTS.md.

## 4. Dedicated memory abilities

- [ ] 4.1 Implement memory.remember as a prepared, bounded create-or-update
  operation over one validated id using the shared store mutation service.
- [ ] 4.2 Implement memory.forget as an exact-id destructive operation with no
  path or query input and route it through normal activation and approval.
- [ ] 4.3 Implement memory.search as bounded deterministic current-project
  search with sensitivity-marked metadata and snippets.
- [ ] 4.4 Register the three abilities through Smith's standard registry and
  root tool view without adding ~/.smith exceptions to workspace tools.
- [ ] 4.5 Project tool calls, results, approvals, failures, and canonical
  events consistently in the TUI and headless surfaces without logging bodies,
  descriptions, keywords, or raw paths.
- [ ] 4.6 Keep search and forget available for explicit management while
  memory is disabled; refuse remember safely and explain how the user may
  re-enable it.

## 5. Opt-in automatic capture

- [ ] 5.1 Add a separately attributed memory.capture model adapter and
  structured upsert-only response contract with output, usage, record-count,
  and wall-clock bounds.
- [ ] 5.2 Run capture only after successfully committed persistent root turns
  when auto_capture is enabled; skip failed, cancelled, interrupted,
  needs-input, child, and review work.
- [ ] 5.3 Supply only the newly committed turn range and a bounded memory
  manifest, validate every proposal through the shared remember path, and
  prevent automatic deletion or sensitivity downgrade.
- [ ] 5.4 Skip a range that already used remember or forget, coalesce
  overlapping triggers, and preserve an exact cursor so retries do not
  duplicate completed work.
- [ ] 5.5 Isolate capture failure, timeout, quota refusal, invalid output, and
  secret rejection from the completed user turn while emitting a safe outcome
  and separate usage attribution.
- [ ] 5.6 Add bounded background completion for interactive sessions and
  bounded pending-capture drain after headless response flush.

## 6. Status, lifecycle, and integration coverage

- [ ] 6.1 Expose enabled state, auto-capture state, valid-record count, index
  revision, and latest safe maintenance/capture outcome through shared runtime
  status.
- [ ] 6.2 Add deterministic fake-model integration tests proving explicit save
  in one session is recalled in a different session for the same ProjectId and
  isolated from another project.
- [ ] 6.3 Prove the default-on empty store adds no fabricated memory, disabling
  suppresses recall and capture without deletion, and opt-in capture is the
  only hidden provider spend.
- [ ] 6.4 Prove automatic and explicit mutations cannot grant tool authority,
  escape the memory root, enter project instructions, expose secrets, or
  silently rewrite canonical history.
- [ ] 6.5 Cover multiple processes, provider/capture failures, shutdown,
  resumed sessions, children, TUI/headless parity, audit redaction, and context
  manifest accounting.

## 7. Documentation and release validation

- [ ] 7.1 Update DESIGN.md and configuration, security, memory-file, tool, and
  privacy documentation, including default enablement and automatic-capture
  opt-in.
- [ ] 7.2 Document the deferred roadmap for model-assisted recall, Dream-style
  semantic consolidation, retention deletion, global memory, team sync, and
  remote storage.
- [ ] 7.3 Run cargo fmt, workspace Clippy with warnings denied, workspace tests,
  targeted persistence/security/concurrency tests, and the coordinated Agent
  Runtime Smith consumer-conformance suite.
