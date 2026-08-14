---
created_at: 2026-08-08T20:29:08Z
updated_at: 2026-08-08T23:40:01Z
completed_at: 2026-08-08T23:40:01Z
---

## 0. Approval and pinning

- [x] 0.1 Approve this proposal before implementation.
- [x] 0.2 Resolve and pin the current stable Harbor release in a nested
  `benchmarks/harbor/pyproject.toml` and `uv.lock`.
- [x] 0.3 Record Harbor Index 1.0's package identity and 82-task manifest.

## 1. Static Smith artifacts

- [x] 1.1 Add a release helper that builds
  `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` Smith binaries
  without modifying the user's Rust toolchain configuration.
- [x] 1.2 Emit a versioned manifest containing Smith revision, target, SHA-256,
  build timestamp, and build command; refuse a dirty source tree by default.
- [x] 1.3 Add a verifier that checks the manifest, digest, ELF architecture,
  executable bit, and `smith --version` before a trial can use an artifact.

## 2. Harbor installed agent

- [x] 2.1 Implement `SmithAgent` as a custom Harbor installed agent with
  explicit Linux/architecture support and a stable agent name/version.
- [x] 2.2 Probe the sandbox architecture, upload the matching verified Smith
  binary, and install only bounded runtime dependencies through Harbor's
  installed-agent helpers.
- [x] 2.3 Generate a private Smith config selecting `chatgpt`,
  `gpt-5.6-luna`, and conservative ChatGPT limits, then invoke `smith -p` with
  `--effort max`, `--approval allow-all`, persistence disabled, and
  schema-v3 `stream-json`.
- [x] 2.4 Save Smith's JSONL stream under the Harbor agent log directory and
  keep ordinary command stdout/stderr free of credentials and machine-protocol
  noise.
- [x] 2.5 Fail with a bounded diagnostic on unsupported OS/architecture,
  missing binary, digest mismatch, missing model support, or a non-success
  Smith result.
- [x] 2.6 Launch Smith with the task image's Bash login environment when
  available, with a POSIX fallback, so task-provided interpreters and test
  runners remain available to shell tool calls; bump the bridge version so
  pre-fix jobs cannot be treated as compatible.

## 3. OAuth isolation

- [x] 3.1 Validate the host auth document's regular-file type, owner-only mode,
  size bound, schema version, and selected entry without logging credential
  values.
- [x] 3.2 Create a temporary minimal auth document containing only the selected
  `chatgpt` entry, upload it into the sandbox, set directory/file modes to
  `0700`/`0600`, and remove the host temporary immediately.
- [x] 3.3 Prove other auth entries are absent from the uploaded document and
  that neither source nor uploaded auth paths are collected as Harbor
  artifacts.
- [x] 3.4 Keep the default concurrency at one and require an explicit labelled
  override for concurrent copies of one OAuth entry.
- [x] 3.5 Return only the selected refreshed entry through an owner-only lock,
  atomic replace, and compare-and-swap while preserving unrelated host state.

## 4. Trajectory and metric conversion

- [x] 4.1 Parse Smith schema-v3 stream/result envelopes with strict version,
  sequence, terminal, model, and effort checks.
- [x] 4.2 Convert instruction, text/reasoning, tool lifecycle, and terminal
  output into ATIF v1.7 without inventing withheld arguments or observations.
- [x] 4.3 Map Harbor input/cache/output counters exactly as specified and keep
  Smith's uncached/cache-write/cache-read/visible-output/reasoning counters in
  ATIF final metadata.
- [x] 4.4 Populate Harbor `AgentContext` with tokens and metadata; leave
  `cost_usd` unset for the OAuth/subscription run.
- [x] 4.5 Record requests, committed/discarded attempts, tool calls/errors,
  activations, children, compactions, reported success, model/effort, Smith
  revision, and artifact digest.
- [x] 4.6 Persist `trajectory.json` and converter diagnostics under the Harbor
  agent log directory even when Smith fails or times out.

## 5. Base-footprint probe

- [x] 5.1 Add a fixed minimal-input probe that starts a fresh Smith session and
  captures its first `context_planned` and provider-usage events.
- [x] 5.2 Report planned segment-kind totals and provider-observed first-request
  totals as separately labelled measurements.
- [x] 5.3 Add a regression fixture proving the probe never relabels an estimated
  component subtraction as provider-reported base tokens.

## 6. Frozen Harbor profiles

- [x] 6.1 Commit a tiny serial smoke profile from Harbor Index 1.0 with one
  rollout and fixed task identities.
- [x] 6.2 Commit a representative frozen 20–30 task development manifest with
  one rollout by default and coverage across the Index task families.
- [x] 6.3 Commit the full 82-task, three-rollout profile with the same model,
  effort, timeout, resource, network, approval, and credential policy.
- [x] 6.4 Record all run invariants in generated job provenance and ignore raw
  job/trajectory output in Git.
- [x] 6.5 Add runbook commands for installing Harbor, building Smith artifacts,
  checking OAuth readiness, running each profile, resuming a job, and opening
  the Harbor viewer.

## 7. Statistical analysis

- [x] 7.1 Add a job loader that validates task/model/effort/provider/run-policy
  compatibility and retains verifier failures rather than dropping them.
- [x] 7.2 Compute task-level rollout means and deterministic task-paired
  bootstrap intervals with at least 10,000 resamples.
- [x] 7.3 Report reward difference, token and latency percentage changes,
  reported-success/verifier-success cross-tabs, and intervals in JSON and
  Markdown.
- [x] 7.4 Refuse improvement/reduction language when an interval crosses zero
  and refuse an unlabelled paired comparison when run invariants differ.

## 8. Verification

- [x] 8.1 Unit-test auth minimization, mode checks, manifest/digest checks,
  stream parsing, ATIF conversion, metric mapping, failure preservation, and
  bootstrap determinism without reading the developer's real home directory.
- [x] 8.2 Run the nested Python formatter, linter, type checker, and tests plus
  Smith's existing relevant headless contract tests.
- [x] 8.3 Run one live Luna Max canary and verify the result names provider
  `chatgpt`, model `gpt-5.6-luna`, effort `max`, and provider-reported usage.
- [x] 8.4 Run the smoke profile, inspect Harbor rewards/trajectories/artifacts,
  and confirm no OAuth material appears in the job directory.
- [x] 8.5 Validate this change with the spec toolkit after every proposal/task
  update.
- [x] 8.6 Regress the task-login wrapper, command quoting, and POSIX fallback,
  then run the nested formatter, linter, type checker, and tests.
