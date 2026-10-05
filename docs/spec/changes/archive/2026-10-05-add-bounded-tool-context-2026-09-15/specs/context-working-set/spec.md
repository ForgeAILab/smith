## ADDED Requirements

### Requirement: Text evidence has a separate inline threshold
Smith SHALL resolve `context.tool_output_inline_bytes` through the existing
configuration layers, defaulting to 8192 serialized bytes in the range
256..=1048576. A stricter existing model-output ceiling SHALL not be increased.

#### Scenario: A medium tool outcome is above the inline threshold
- GIVEN an authorized artifact store and a text-only result above the threshold
- WHEN the result is committed
- THEN the unchanged captured outcome is stored before substitution and the
  model receives a reference and bounded excerpt rather than the full body.

#### Scenario: A result is small or contains typed non-text content
- GIVEN a small result or inline content containing images or opaque reasoning
- WHEN the output processor runs
- THEN it passes the outcome through without text-only offloading.

### Requirement: Excerpts preserve evidence identity without inventing meaning
Offloaded results SHALL retain the runtime-owned session attribution, stable
idempotency, source digest and original `is_error`. Human-readable previews
SHALL label heuristic diagnostic excerpts and SHALL NOT infer command success
from matching words. Failed storage SHALL NOT produce a fabricated reference.

#### Scenario: Successful command output contains the word error
- GIVEN a successful command whose text includes an error example
- WHEN a heuristic diagnostic excerpt is selected
- THEN the result remains successful and the exact original stays retrievable.

### Requirement: Artifact retrieval is bounded and authorized
The effective raw page bound SHALL derive from the same resolved inline policy.
Missing limits SHALL use that bound; valid larger limits SHALL be clamped during
preparation. Source schemas SHALL permit the accepted input before normalization.
Invocation SHALL refuse a prepared limit outside the current bound and delegate
all session-ownership and source-integrity validation to the existing runtime.

#### Scenario: A saved artifact is read after reopening a session
- GIVEN a valid session-owned artifact retained on disk
- WHEN its owner requests a large page after restart
- THEN the tool returns a bounded page and accurate continuation offset; a
  different session cannot read it and no original tool effect is replayed.

### Requirement: Root and child policy cannot drift
Root and child runtimes SHALL receive the policy derived from their resolved
configuration. `/context` SHALL identify whether offloading is available and
SHALL distinguish last-request occupancy from cumulative provider usage.

#### Scenario: A child inherits a small inline allowance
- GIVEN an inline threshold smaller than Runtime's generic default
- WHEN a child produces a medium text result
- THEN it offloads using the resolved threshold and retains session ownership.

#### Scenario: Persistence and artifact storage are unavailable
- GIVEN a runtime without an artifact store
- WHEN the context inspector reports the output policy
- THEN it does not claim recoverable offloading and existing output limits apply.
