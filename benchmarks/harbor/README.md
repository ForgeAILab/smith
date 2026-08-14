# Smith Harbor evaluation

This isolated project runs Smith as a custom Harbor installed agent against the
pinned 82-task Harbor Index 1.0 package. The serving path is Smith's supported
ChatGPT/Codex OAuth provider with `gpt-5.6-luna` and `max` reasoning.

The benchmark never writes credential values into a repository file, Harbor
configuration, command, log, trajectory, or artifact. Each trial receives a
private schema-v1 `auth.json` containing only the `chatgpt` entry. After the
trial, the bridge atomically returns only that entry's refreshed bundle to
Smith's supported host auth store; every unrelated credential and document
field is preserved. A private lock and compare-and-swap reject concurrent host
changes. Because refresh-token rotation is sequential, frozen jobs run
serially.

Smith is launched through the task image's Bash login environment when Bash is
available, with a POSIX-shell fallback for minimal images. This preserves
task-provided interpreter and test-runner activation for Smith and every shell
tool call it spawns.

## Install and preflight

From this directory:

```sh
uv sync --locked --dev
uv run smith-harbor auth-check
uv run smith-harbor validate-profile smoke
uv run smith-harbor validate-profile dev
uv run smith-harbor validate-profile full
```

`auth-check` validates regular-file type, owner, private mode, size, schema, and
the selected entry without printing the auth path or value. Set
`SMITH_HARBOR_AUTH_FILE` only when the supported Smith auth file is not at
`~/.smith/auth.json`.

If the entry is absent or stale, start Smith with any usable provider and renew
it through Smith's supported login surface before retrying:

```text
/connect chatgpt
```

## Build Smith artifacts

The builder uses a digest-pinned, native-arm64 Cargo Zigbuild container with
Rust 1.93 to produce both musl targets. Smith enables keyring's vendored D-Bus
feature so the Linux binaries do not depend on a sandbox's system D-Bus
library. No Rust targets or tools are added to the host. A clean source tree is
required for reportable artifacts:

```sh
uv run smith-harbor build
uv run smith-harbor verify-artifacts
```

During local harness development only, `--allow-dirty` emits a manifest with
`"dirty": true`; do not use that artifact for a reportable comparison.

The manifest records the full Smith Git revision, dirty state, build time,
target, platform, command, ELF machine, SHA-256, byte size, and verified
`smith --version` output. Each Harbor trial verifies the selected digest and
version again after upload.

## Live Luna Max gates

Run the isolated host canary before spending on Harbor tasks:

```sh
uv run smith-harbor canary --output reports/canary.json
uv run smith-harbor probe --output reports/base-footprint.json
```

The canary requires provider `chatgpt`, model `gpt-5.6-luna`, effective effort
`max`, a successful terminal result, and provider-reported usage. The base
report keeps Smith's estimated/planned segment counts separate from the
provider-observed first-attempt counters. OAuth subscription cost is always
`unknown`; OpenAI Platform prices and zero marginal cost are not substituted.

## Run Harbor Index

The gates are intentionally sequential:

```sh
uv run smith-harbor run smoke
uv run smith-harbor run dev
uv run smith-harbor run full
```

- `smoke`: 3 fixed tasks × 1 rollout.
- `dev`: 26 fixed tasks spanning the Index task families × 1 rollout.
- `full`: all 82 pinned tasks × 3 rollouts.

To make a stable job name for handoff or resume:

```sh
uv run smith-harbor run smoke --job-name smith-luna-max-smoke-001
uv run smith-harbor resume jobs/smith-luna-max-smoke-001
uv run harbor view jobs --jobs
uv run smith-harbor audit-job jobs/smith-luna-max-smoke-001
```

Raw jobs and trajectories are ignored by Git. Do not pass Harbor's `--upload`
option: public or hosted job upload is outside this evaluation's authorization.

On macOS, the CLI defaults job output to
`~/Library/Caches/smith-harbor/jobs` because Docker Desktop must be able to
bind-mount the job directory for live trajectory conversion. Linux defaults to
this package's ignored `jobs/` directory. Set `SMITH_HARBOR_JOBS_DIR` or pass
`--jobs-dir` to override it, but use a Docker-shared path. For long experiments,
prefer a durable non-cache volume: macOS may purge `~/Library/Caches` under disk
pressure, including completed Harbor trajectories.

Concurrency above one is not a supported OAuth configuration. Development
experiments must both pass `--unsafe-concurrency N` and include
`unsafe-oauth` in the job name; their provenance is labelled accordingly and
compare-and-swap conflicts fail closed.

### Apple-Silicon Colima

Harbor Index task and verifier images are predominantly `linux/amd64`. A
dedicated Colima VM on Apple Silicon must use Virtualization.framework with
Rosetta rather than generic binfmt/QEMU emulation. Enable it while no Harbor
job is active:

```sh
export SMITH_HARBOR_COLIMA_HOME=/Volumes/Data/.smith-harbor-colima-home
COLIMA_HOME="$SMITH_HARBOR_COLIMA_HOME" colima stop --profile harbor
COLIMA_HOME="$SMITH_HARBOR_COLIMA_HOME" colima start --profile harbor --vz-rosetta
```

Verify both the persisted setting and an AMD64 container before a reportable
run:

```sh
rg '^rosetta: true' \
  "$SMITH_HARBOR_COLIMA_HOME/harbor/colima.yaml"
docker --context colima-harbor run --rm --platform linux/amd64 alpine:3.21 \
  sh -c "grep -m1 '^vendor_id' /proc/cpuinfo"
```

The AMD64 probe should report `VirtualApple`. A reportable run must not switch
emulation mode between paired variants.

## Compare compatible jobs

```sh
uv run smith-harbor analyze \
  jobs/baseline \
  jobs/candidate \
  --json reports/comparison.json \
  --markdown reports/comparison.md
```

The analyzer retains verifier failures, averages rollouts within each task,
and bootstraps paired task differences with a fixed seed and 10,000 resamples.
It compares reward difference plus token and latency percentage changes.
“Improved” and “reduced” appear only when the corresponding 95% interval
excludes zero. Missing token or timing evidence makes that metric unavailable
rather than silently dropping failed trials.

Provider, model, effort, task set, rollout policy, timeout, resource, network,
approval, persistence, and OAuth concurrency must match. Use
`--descriptive-only` for an explicitly unpaired summary when they do not.

## Completion-policy ablation

The approved Luna Max development ablation uses the frozen 26-task `dev`
profile, one rollout in each job, and exactly one active OAuth trial. Write the
schedule before starting, then use the same command to run or resume it:

```sh
SMITH_HARBOR_DOCKER_CONTEXT=colima-harbor \
SMITH_HARBOR_JOBS_DIR=/Volumes/Data/smith-harbor/jobs \
uv run smith-harbor experiment-plan \
  --manifest reports/dev-completion-policy-manifest.json
SMITH_HARBOR_DOCKER_CONTEXT=colima-harbor \
SMITH_HARBOR_JOBS_DIR=/Volumes/Data/smith-harbor/jobs \
uv run smith-harbor experiment-run \
  --manifest reports/dev-completion-policy-manifest.json
```

The launcher runs these nine jobs in order:

```text
smith-luna-max-dev-ablation-r1-current
smith-luna-max-dev-ablation-r1-artifact-first
smith-luna-max-dev-ablation-r1-artifact-first-no-delegation
smith-luna-max-dev-ablation-r2-artifact-first
smith-luna-max-dev-ablation-r2-artifact-first-no-delegation
smith-luna-max-dev-ablation-r2-current
smith-luna-max-dev-ablation-r3-artifact-first-no-delegation
smith-luna-max-dev-ablation-r3-current
smith-luna-max-dev-ablation-r3-artifact-first
```

Together they produce 234 expected trajectories. On a quota, network, Docker,
or OAuth interruption, renew through `/connect chatgpt` if needed and rerun the
same `experiment-run` command. Completed cells are skipped; an incomplete cell
is resumed only after its manifest, serving invariants, variant, and Smith
artifact match. Never add Harbor's `--upload` option: this experiment is local
only and upload is explicitly unauthorized.

Set `SMITH_HARBOR_DOCKER_CONTEXT` while both planning and running when the
experiment uses a dedicated Docker engine. The selected context is recorded as
a compatibility invariant and passed explicitly to Harbor; it is never inferred
again during resume.

After all nine cells and audits complete:

```sh
SMITH_HARBOR_JOBS_DIR=/Volumes/Data/smith-harbor/jobs \
uv run smith-harbor experiment-analyze \
  --manifest reports/dev-completion-policy-manifest.json \
  --json reports/dev-completion-policy.json \
  --markdown reports/dev-completion-policy.md
```

The report keeps the completion-policy contrast separate from the incremental
no-delegation contrast and uses deterministic task-paired bootstrap intervals
over the three rollout observations per task.
