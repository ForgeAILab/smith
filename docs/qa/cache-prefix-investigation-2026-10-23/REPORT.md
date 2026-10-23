# ChatGPT cache prefix investigation

Smith's native ChatGPT serializer was rewriting otherwise reusable conversation prefixes. Two regression tests reproduced the problems before the fix. A controlled, plan-posture six-read Astra probe increased from **19.01% to 99.08% cached input** with the final patched binary. Build-posture probes still show misses, as detailed below.

## Changes

- `chatgpt/provider.rs`: only the first leading system message becomes the request's `instructions`. Later host context remains at its planned position as a developer input message. Previously every system message was collected into `instructions`, so an update late in the conversation changed the beginning of the request.
- `chatgpt/encoding.rs`: serialize assistant text, signed reasoning and tool calls in their original order. Previously all assistant text was emitted first, ahead of reasoning and calls that originally preceded it. Adjacent text parts remain grouped.
- `chatgpt.rs` and `factory/provider/adapter.rs`: use cache contract revision `smith-chatgpt-prefix-2` for the changed native wire projection. Keep existing unsupported and credential-pool handling.

The changes preserve context privilege and existing cache routing. They do not upgrade Agent Runtime, change image preprocessing, replace the installed Smith binary, or change the original Blender session. [cache-prefix-fix.patch](cache-prefix-fix.patch) isolates these changes from the existing dirty workspace. Its reverse application was checked without modifying files.

## Live probe

All cases in this table used `gpt-6-astra`, plan posture, low effort, the same prompt hash, six identical inert fixture files and exactly six sequential `read` calls. Each produced seven provider attempts and 136,680 total input tokens. Tool argument fingerprints match across cases. Logs and binary hashes are retained beside this report.

| Build | Cached input | Uncached input | Cached share | Elapsed |
|---|---:|---:|---:|---:|
| Original serializers | 25,984 | 110,696 | 19.01% | 25.11s |
| Host-context fix only | 135,424 | 1,256 | 99.08% | 24.71s |
| Both serializer fixes, prior cache revision | 133,248 | 3,432 | 97.49% | 31.23s |
| Both fixes, final cache revision | 135,424 | 1,256 | 99.08% | 25.39s |

This probe primarily demonstrates the host-context fix. The independent assistant-order bug is demonstrated by the failing regression fixture, not by attributing a separate live cache gain to it. Later cases may benefit from shared provider warmth; these are empirical observations, not guaranteed hit rates or a latency benchmark. The baseline kept receiving exactly 3,712 cached tokens while its uncached suffix grew from 2,568 to 29,068 tokens. The patched run reused successively longer prefixes.

An additional four-turn no-tool Astra comparison is recorded in `astra-before-result.json` and `astra-after-result.json`; its weighted cache share was 77.75% and 96.45%. Initial warmth differed, so the matched tool probe is the stronger result.

### Remaining miss pattern

The larger three-stage webpage task still showed low Smith cache reuse after these fixes. A second **no-image** diagnostic activates `write_todos`, creates one todo, reads the same six fixtures without changing the todo between reads, then completes it. That run reported **18.61%** cached input (37,120 cached / 162,296 uncached).

The temporary diagnostic build saved hashes, types, roles and lengths of request items, plus fingerprints of instructions/tools and the opaque cache key. Requests 4–9 retain every prior input item exactly, and keep instructions, tools and cache key unchanged, while each still reports only 3,712 cached tokens. See `todos-wire-audit.json` and `todos-wire-result.json`. This shows that image encoding is **not necessary** to reproduce the remaining cache miss pattern. It does not establish whether the remaining issue lies in native conversation replay, cache population/routing or another provider behavior.

A further control uses **build posture with no todos or registry activation**, and the exact original six-read prompt/fixtures. It reports **18.73%** cached input (26,240 cached / 113,842 uncached), versus 99.08% in the earlier plan-posture final-build probe. The build-only requests also keep their instructions, tools, key and prior input items unchanged; see `build-read-wire-audit.json`. Changing todo memory is therefore not required either. This narrows the remaining investigation to a difference associated with the build prompt/tool configuration or its native replay, rather than establishing a specific cause.

The diagnostic build only modified the isolated temporary source copy; instrumentation is absent from production files and benchmark binaries. Its reproducible changes are in `wire-audit-instrumentation.patch`, and the temporary source copy was restored afterward. An initial stderr-based diagnostic was excluded because the headless runner holds stderr's lock; valid diagnostic output uses a separate file. Excluded logs have the `invalid-stderr-audit-` prefix.

## Follow-up: request routing

The remaining misses come from request routing, not from serialization. Smith sent `session-id: <request id>`, which is `req-1`, `req-2`, … in every session. Every concurrent Smith session on an account therefore shared one id per request number, and no session kept its own id from one request to the next. Smith also discarded the `x-codex-turn-state` sticky-routing token that the endpoint returns on a turn's first response. Codex echoes that token on every later request in the turn.

This explains both earlier observations. The 18.61% and 18.73% probes ran at 18:08 and 18:27, while the benchmark's Smith sessions were running. The 99% probes ran with no other Smith session active. A rerun of the unchanged final binary, with nothing else running, reached 87.54% and 99.05% in build posture. In the benchmark, nearly every Smith request reused only a 3,712-, 3,968- or 7,296-token base prefix and never its own previous request.

`chatgpt/provider.rs` now sends `session-id` equal to `prompt_cache_key` (as Codex does) and `thread-id` equal to the session. It keeps the first turn-state token each turn returns and replays it while the newest non-system message is a tool result. The token is dropped when a new turn starts or the credential account changes. Advisor, vision and module one-shots reuse the caller's session mid-turn, so only ordinary requests that offer tools read or record the token. No body or cache-key bytes changed, so the cache revision stays the same.

All probes used the session fallback cache key. With a runtime `CacheIdentity`, `session-id` follows its digest, which changes when registry or activation revisions change. The prompt-cache partition moves at the same point, so routing and partition stay aligned. The header is not stable across such a change.

Controlled comparison: build posture, six reads, fixtures salted per run so earlier runs cannot warm the cache, three concurrent sessions per row. About 78% is this probe's ceiling, since each read's new output cannot be cached.

| Build | Cached share (3 sessions) |
|---|---|
| Diagnostic, original header | 28.35%, 30.27%, 30.31% |
| Diagnostic, stable `session-id` | 63.49%, 78.31%, 78.23% |
| Diagnostic, stable `session-id` + turn-state | 78.10%, 78.07%, 79.98% |
| Final binary before this fix (`736cec7d…`) | 28.26%, 32.17%, 32.37% |
| Fixed binary (`5f2e2bf9…`) | 72.87%, 78.26%, 78.09% |
| Fixed binary with one-shot gate (`ed9665f0…`) | 64.50%, 80.31%, 79.90% |

The fourth and fifth rows ran back to back. With the original header, follow-up requests reused only the 6,400-token prompt prefix the three sessions share. With the fix, 17 of 18 follow-up requests reused their own previous request (16 of 18 with the final binary). Some scatter remains. Codex runs a turn over one Responses WebSocket and sends incremental input, which Smith's HTTP adapter does not; this may account for the rest of the gap to Codex's 91–93%, but that was not tested. Results, the salted probe, the concurrent runner and the diagnostic provider are in [routing/](routing/). `turn-state-diag.jsonl` records only whether a token was present and its length, never its value.

The multistage image benchmark was not rerun with the fixed binary.

## Source comparison

- Smith workspace HEAD: `7baa1c7c`; pre-existing local changes preserved.
- Smith pins Agent Runtime v0.7.0, commit `a5f1b1788e982c39e7c3a394c715a7d92cc7852b`.
- Sibling Agent Runtime HEAD: `b791341`, v0.9.0. Its generic native Responses adapter already keeps system messages in conversation order.
- Fetched Codex `origin/main`: `970111f6cb62cc02fe36a82ea7d71b37a2889c97`. Inspected in detached worktree `/private/tmp/codex-cache-source-20261023`; the user's dirty Codex checkout at `32329b289d` was left intact.
- Current Codex `codex-rs/core/src/client.rs` prepends base instructions as stable developer items with deterministic UUIDv5 IDs and preserves ordered response items. `core/tests/suite/responses_lite.rs` checks the leading developer instruction item. Smith's narrower fix keeps its existing API shape and restores the same ordering property.

Two broader Runtime opportunities remain unmodified: TodoComponent's changing memory can precede history, and provider routing identity includes registry/activation/harness revisions that may partition reuse when capabilities change. Those require separate, controlled evidence before changing Runtime policy. The no-image diagnostic above used a stable session fallback cache key, so routing-key churn does not explain that particular run. These are partial fixes, not a claim that all caching behavior is resolved.

## Verification

Routing follow-up:

- `routing_headers_follow_the_session_and_replay_the_turn_token` failed on the original header (`left: Some("req-1")`, [routing-regression-before.log](routing-regression-before.log)). With the gate removed, it also failed: the parent replayed a one-shot's token.
- `cargo test -p smith-runtime --lib`: **699 passed, 2 ignored** in the shared workspace, and **689 passed, 2 ignored** on the committed tree, which excludes other uncommitted work.
- `cargo clippy -p smith-runtime --lib --tests -- -D warnings`: passed on both.
- Final binary: `/private/tmp/smith-routing-fix-final`, SHA-256 `ed9665f0826627aaf5863769658edf5a259090fab76379ea2e67d87899d384f2`.

Serializer fixes (earlier):

- Both added regression tests failed before the fixes: `tail-regression-before.log`, `assistant-order-before.log`.
- ChatGPT adapter regression suite: 28 passed, 1 ignored.
- `cargo test -p smith-runtime --lib`: **697 passed, 2 ignored**.
- `cargo clippy -p smith-runtime --lib -- -D warnings`: passed.
- Scoped formatting and diff whitespace checks: passed.
- Final binary: `/private/tmp/smith-cache-prefix-versioned`, SHA-256 `736cec7de694187f53ad67d2318c6b4eaa9198c3d24688c539edf8b944ea2e0d`.

The [three-stage image benchmark](../web-reconstruction-multistage-2026-10-23/) uses that immutable final binary. It tests multiple images and resumed editing separately from this controlled cache probe.
