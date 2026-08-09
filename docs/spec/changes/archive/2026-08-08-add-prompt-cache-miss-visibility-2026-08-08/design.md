## Context

Smith currently has three partial cache views:

- `Status::record_cache` accumulates positive reads and `render_cache` renders
  either a token count or `?`.
- restored snapshots recover positive `InputCached` totals but cannot recover
  explicit zero evidence;
- headless `CacheOutput` reports the final turn's last `CachePlanChanged`
  expectation but no provider observation.

The coordinated Agent Runtime change supplies one canonical per-attempt event
with request, attempt, cache-plan, state, expected read, observed read/write,
missed tokens, and confidence. Smith should reduce that event once and project
the same facts into each host surface.

## Goals / Non-Goals

- Goals:
  - distinguish zero, omission, first eligibility, hit, and miss;
  - show a useful latest-turn cache-read percentage without guessing;
  - surface significant re-billing without making transcript noise mandatory;
  - keep TUI live/replay, status, exit, and headless projections equivalent;
  - preserve derived/provided provenance and avoid double-counting usage;
  - state idle correlation without claiming expiry.
- Non-Goals:
  - move Smith presentation policy into Agent Runtime;
  - infer a universal provider TTL;
  - add a provider diagnostic beta or extra request;
  - add a settings UI or a new session command;
  - design child cache panels in this change.

## Decisions

### 1. One reducer owns Smith's cache projection

Smith adds a cache projection keyed by turn, logical request, provider attempt,
and cache-plan fingerprint. The reducer consumes:

- `ProviderAttemptStarted` for request timing;
- `Usage` for disjoint attempt-attributed input counters;
- `CacheStateChanged` for expectation and observed state;
- `ProviderAttemptFinished` for failed/retried attempt disposition;
- `TurnCompleted` for one stable turn rollup and optional notice.

The same reducer code is used for live TUI events and journal replay. Headless
uses the same projection type or a contract-tested equivalent, not a separate
cache-miss formula.

### 2. `CH` is provider-reported cache-read share

For the latest completed root turn:

`prompt_input = input_uncached + input_cached + cache_write`

`CH = input_cached / prompt_input`

All provider attempts in the turn contribute because failed retries are still
billed. The footer renders:

- a rounded percentage, including `0%`, when prompt input and explicit
  cache-read presence are known;
- `?` when cache-read evidence is absent or prompt input is unavailable;
- no prior provider/model's value after an identity switch.

This is deliberately distinct from expectation fulfilment. `/status` and
machine output also expose expected, observed, and missed tokens from the
canonical event.

### 3. Re-billed tokens are a derived diagnostic, not usage

For each attributed event, Smith uses Agent Runtime's
`missed_tokens = expected - observed` projection and retains its confidence.
Turn and session rollups sum those values and count miss-bearing attempts.
They do not add a new `CounterKind`, inflate total tokens, alter routing, enter
budgets, or reach the model.

When the serving binding has a compatible versioned price:

`paid_tokens = input_uncached + cache_write`

`paid_rate = (priced uncached input + priced cache writes) / paid_tokens`

`extra_cost = missed_tokens * max(0, paid_rate - cache_read_rate)`

The result is labelled derived/estimated according to the cache expectation
and price inputs. If a required rate or attribution is missing, extra cost is
unknown and the token threshold remains the only significance test.

### 4. Notices are opt-in and turn-scoped

`cache.miss_notices` defaults to `false`. When enabled, Smith appends at most
one local notice after a completed root turn when either:

- total missed tokens are at least 20,000; or
- known derived extra cost is at least $0.10.

The notice reports expected/observed or re-billed tokens and includes cost only
when known. Multiple miss-bearing attempts are summarized rather than producing
retry or tool-loop spam. The block is local presentation: it is not a user or
assistant message, never enters provider context, and is deterministically
reconstructed from journaled events on replay.

### 5. Idle time is context, not cause

Idle duration is measured between the starts of consecutive logical provider
requests, not between retry attempts. For a single miss-bearing request, an
idle duration of at least one minute may be rounded to minutes and rendered as
`Cache miss after 9m idle`. Shorter or unavailable durations use `Cache miss`.

Elapsed time alone MUST NOT produce `expired` or `likely expired`. That wording
may be introduced only by a future canonical provider diagnostic that
establishes matching requests and an unavailable cache entry. A provider/model
identity change has an expected read of zero and therefore produces no miss;
Smith's existing model-switch notice remains the correct surface.

### 6. Human and machine outputs share one vocabulary

`/status` and the exit/session usage summary show:

- latest canonical state;
- latest-turn `CH`;
- expected, observed, and missed tokens when known;
- cumulative miss count and re-billed tokens;
- extra cost only with its derived provenance.

Final JSON retains the existing last-plan fields and adds the final turn's
aggregate state and metrics. `stream-json` includes the canonical runtime
events unchanged. Human headless mode keeps stdout reserved for the answer;
when notices are enabled, a significant cache notice is progress/diagnostic
text on stderr.

### 7. Persistence is additive and backward-compatible

Canonical cache events are already journal records. Live and replay reducers
must produce equal cache projections. Smith's bounded usage-log/session-summary
schema gains optional/defaulted cache miss count and re-billed-token fields;
older records deserialize as “no cache-miss evidence,” not a verified zero.

Legacy unattributed `CacheObservation` entries may restore positive cache-read
totals but cannot create a miss, idle hint, or expected-read metric.

## Risks / Trade-offs

- A cache read can exceed the planner's stable-prefix expectation. `CH` keeps
  the provider count; missed tokens remain saturating and never negative.
- Turn aggregation includes failed attempts, so `CH` describes billed work
  rather than only the committed answer. Status text must make that clear.
- Fixed significance thresholds are product policy. Keeping only the on/off
  switch configurable avoids a wider configuration surface in this change.
- Concurrent and recently archived changes touch `Status` and client surfaces.
  Implementation needs a careful rebase and focused contract tests.

## Migration Plan

1. Land and pin the coordinated Agent Runtime event contract.
2. Update Smith's experimental ChatGPT adapter and all exhaustive event
   matches.
3. Add the shared cache projection and live/replay tests.
4. Add footer, `/status`, exit, transcript, and headless projections.
5. Add configuration, persistence migration, and documentation.
6. Run deterministic cross-surface fixtures and one opt-in live-provider check
   without asserting a provider TTL.

## References

- [OpenAI prompt caching](https://developers.openai.com/api/docs/guides/prompt-caching)
  reports reads/writes explicitly and documents model-specific prefix and
  retention behavior, which is why Smith has no universal five-minute rule.
- [Anthropic Cache Diagnostics](https://platform.claude.com/docs/en/build-with-claude/cache-diagnostics)
  confirms that first-turn zero is normal and that request matching and cache
  availability are separate evidence.
- [Pi cache-miss tracking PR](https://github.com/earendil-works/pi/pull/6427)
  informs the notice thresholds and product vocabulary, while Smith uses the
  stronger shared-runtime expectation.

## Open Questions

None for this approval. Custom thresholds, delegated cache aggregation, and
provider diagnostics remain follow-up work.
