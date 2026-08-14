## Context

Harbor evaluates an installed agent inside each task sandbox and stores the
verifier result, timing, logs, and optional ATIF trajectory. Smith already has
the required non-interactive surface: `smith -p`, schema-v3 `stream-json`,
provider-attributed usage, context-plan events, tool lifecycle events,
reasoning selection, cache observations, child lifecycle, and an explicit
`allow-all` automation policy.

The selected provider is Smith's `chatgpt-responses` path. It uses a renewable
OAuth bundle stored as one opaque value in `~/.smith/auth.json`. This is a
supported Smith credential flow but it is not an OpenAI Platform API-key run,
does not report per-request USD cost, and must be labelled accordingly.

## Goals / Non-Goals

- Goals:
  - Run Smith reproducibly on Harbor Index 1.0 with Luna Max.
  - Keep benchmark mechanism separate from Smith product policy.
  - Preserve verifier reward, provider usage, timing, lifecycle, and
    trajectory evidence without exposing secrets.
  - Support cheap infrastructure checks, a frozen development subset, and a
    complete three-rollout evaluation.
  - Make compatible jobs statistically comparable without overstating noisy
    point estimates.
- Non-Goals:
  - Add a new provider adapter or credential format.
  - Produce an official cross-harness leaderboard in this change.
  - Treat subscription usage as OpenAI Platform spend.
  - Make OAuth refresh-token synchronization safe at high concurrency.

## Decisions

### 1. A custom Harbor installed agent owns the bridge

`benchmarks/harbor/smith_agent.py` will extend Harbor's installed-agent base.
Harbor continues to own datasets, environments, verifiers, rollouts, and job
storage. The bridge owns only Smith installation, private configuration,
invocation, and result conversion. Harbor is pinned in a nested `uv` project so
its Python dependencies do not enter Smith's Cargo workspace.

### 2. Build once and upload a static binary

Compiling Rust inside every task would make setup time dominate the benchmark.
A helper builds static Linux binaries for the supported Harbor architectures,
records the Smith Git revision, target, binary SHA-256, build timestamp, and
dirty-state refusal, and emits a manifest. The installed agent probes the task
architecture, uploads the matching artifact, verifies its digest, and records
the manifest in agent metadata.

The initial targets are `x86_64-unknown-linux-musl` and
`aarch64-unknown-linux-musl`. Unsupported operating systems or architectures
fail before a model request.

### 3. Copy only one OAuth entry

The host-side bridge reads the configured auth file (default
`~/.smith/auth.json`), validates schema version 1, selects one configured entry
(default `chatgpt`), and creates a temporary minimal document containing only
that entry. Harbor uploads it to the agent user's `$HOME/.smith/auth.json`; the
directory and file are set to modes `0700` and `0600` respectively.

The source file, temporary value, and uploaded value are never logged. Smith's
user-state directory is outside `/logs`, and artifact collection must reject
any path under it. After Smith exits, the bridge downloads the private auth
document into an owner-only host temporary directory, validates it again, and
extracts only the selected entry. Under an adjacent owner-only file lock, it
compares the current host entry with the originally copied value, replaces only
that entry with the refreshed value, preserves all other document fields, and
publishes the result with an atomic same-directory rename. If the host entry
changed concurrently, the handoff fails closed instead of overwriting it.

This handoff is required because ChatGPT OAuth refresh tokens can rotate: a
discarded refreshed bundle can invalidate the source used by the next trial.
Default job concurrency is therefore one. Increasing concurrency requires an
explicit override and is labelled an unsupported-risk experiment for a single
OAuth entry; compare-and-swap conflicts remain errors.

### 4. Pin the serving identity and conservative limits

Every benchmark invocation selects:

```text
provider: chatgpt
model: gpt-5.6-luna
reasoning effort: max
approval: allow-all
persistence: disabled
```

Smith's trusted ChatGPT table currently includes Terra, not Luna. The generated
trial config therefore supplies the same conservative ChatGPT product limits
used by the reviewed Terra binding: 272,000 context tokens, 255,616 input
tokens, and a 16,384 output/request reserve. It advertises the Luna effort
ladder through `max` and uses the existing `--effort max` invocation flag. A
live canary must prove that the endpoint accepts the model and reports `max` as
the effective selection before any Harbor Index job begins.

These are benchmark-run limits, not a new trusted Smith catalog record.

### 5. Preserve detailed usage while mapping Harbor aggregates

Smith's terminal result contains disjoint `input_uncached`, `input_cached`,
`cache_write`, `output`, and `reasoning` counters. The bridge maps them as:

```text
Harbor input  = input_uncached + input_cached + cache_write
Harbor cache  = input_cached
Harbor output = output + reasoning
Harbor cost   = unknown
```

The ATIF final metrics and metadata retain every Smith counter separately,
including cache writes and reasoning. The bridge also records provider request
count, committed/discarded attempts, tool calls/errors, activated
capabilities, child calls, compactions, Smith-reported terminal status, model,
effort, binary revision, and Harbor trial identity.

ATIF steps are reconstructed from the instruction, attempt-scoped text and
reasoning events, redaction-safe tool request/completion events, and the final
committed answer. Smith intentionally withholds raw tool arguments and output
from general event logs; the converter must preserve that boundary rather than
invent missing observations.

### 6. Separate base footprint from full trajectories

The base probe runs a fixed minimal input in a fresh session. From the first
`context_planned` event it reports planned tokens by segment kind:

```text
system_instruction
developer_instruction
ability_instruction
tool_schema
memory
retrieval
history
tool_result
user_input
continuation
summary
```

The first attempt's provider usage is reported separately as an observed total.
The report may show both values but must not subtract an estimated user segment
from provider usage and relabel the remainder as provider-reported base tokens.

Harbor task results use complete trajectory totals across every provider
attempt, retry, tool continuation, compaction, and child call.

### 7. Freeze three execution profiles

- `smoke`: a tiny fixed task list, one rollout, serial execution; proves
  installation, OAuth, model selection, tools, verifier, and conversion.
- `dev`: a committed 20–30 task manifest spanning the Harbor Index task
  families, one rollout by default; used during harness development.
- `full`: all 82 Harbor Index 1.0 tasks, three rollouts, serial execution by
  default; used for reportable results.

Dataset version, task identities/digests, Harbor version, Smith binary
manifest, model, effort, timeout, resource policy, network policy, approval
policy, and rollout count are retained with every job. Raw jobs are ignored by
Git; a sanitized summary may be committed deliberately.

### 8. Pair comparisons by task

The analyzer accepts two compatible Harbor job directories. For each task it
averages rollouts within each job, computes the paired difference, then
bootstraps tasks with replacement using a fixed seed and at least 10,000
replicates. It reports 2.5th/97.5th percentiles for reward difference and
percentage changes in tokens and latency. A metric is called improved or
reduced only when its interval excludes zero.

Jobs with different dataset/task sets, model, effort, provider path, timeout,
or rollout policy are rejected as unpaired unless the user explicitly requests
a labelled descriptive-only comparison.

## Risks / Trade-offs

- The ChatGPT backend is subscription/OAuth, not the public Platform API.
  Results must name the path and cannot be compared directly with API-key runs
  as if provider conditions were identical.
- A subscription run has no per-trajectory USD bill. Cost remains unknown,
  reducing parity with API-backed evaluations but avoiding fabricated numbers.
- Serial execution is slow. It is the safe default for one renewable
  credential whose rotations are handed from one trial to the next; higher
  concurrency remains an explicit experiment and can fail on a refresh
  compare-and-swap conflict.
- Model or subscription availability can change independently of the code. A
  live canary and job provenance make this visible but cannot eliminate it.
- Static musl binaries enlarge the setup artifact but avoid per-trial Rust
  compilation and libc variance.
- Smith's redaction-safe events do not contain full raw tool observations, so
  ATIF is operationally useful but not a byte-for-byte replay transcript.

## Migration Plan

1. Land the benchmark project without changing Smith production behavior.
2. Build and verify both static artifacts.
3. Run the local fake/converter tests.
4. Run one live Luna Max canary with the selected OAuth entry.
5. Run the frozen smoke profile and inspect every artifact boundary.
6. Run the development subset.
7. Run the complete three-rollout profile only after the previous gates pass.

## Open Questions

- None for the first serial OAuth-backed run. Cross-harness comparison and
  higher-concurrency credential handling require separate approval.
