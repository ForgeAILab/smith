# context-working-set Specification

## Purpose
How much tool output stays inline in the model request, how larger output is preserved as session-owned artifacts, and how those artifacts are read back within bounds.
## Requirements
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

### Requirement: Images reach a text-only model as placeholders

Smith SHALL replace every image part in the planned request when the active
model does not accept image input. The replacement SHALL be a text placeholder of the form
`[image <id>: <mime>[ <w>×<h>] not included in this request]`. Pixel
dimensions SHALL appear when the image header can be decoded. This SHALL
apply to images in tool results, user messages, and assistant messages.
Canonical session history SHALL keep the original image. `<id>` SHALL be the
first 8 lowercase hex digits of the SHA-256 digest of the image's data URL,
so the same image keeps the same id across requests, compaction of other
messages, and resume.

#### Scenario: Command provider receives a screenshot result

- **GIVEN** a command-JSONL main model
- **AND** an MCP tool result carrying a PNG
- **WHEN** the next request is planned
- **THEN** the provider receives a placeholder naming the image id, `image/png`,
  and its dimensions
- **AND** the request does not fail

#### Scenario: Pasted image on a text-only model

- **GIVEN** a text-only main model
- **WHEN** the user submits a message with an attached image
- **THEN** the request carries the message text and a placeholder for the image
- **AND** the image remains in the session history

#### Scenario: Switching to a text-only model mid-session

- **GIVEN** a session whose history holds images from a vision model's turns
- **WHEN** the user switches with `/model` to a text-only model and sends a
  prompt
- **THEN** every earlier image is a placeholder in that request
- **AND** switching back to the vision model includes the window's images again

### Requirement: Bounded image window with a stable prefix

When the active model accepts image input, the planned request SHALL include
only a window of the newest images. Every image in the newest user message
SHALL be included and SHALL NOT count toward the window. The other images
SHALL be numbered `0..T` in history order. The image at index `i` SHALL be
included when `i >= K * floor(max(T - K, 0) / K)`, where `K` is
`context.inline_images`. Every other image SHALL be replaced by the
placeholder above. Whether an image is included SHALL depend only on history
and `K`. As a result, the planned prefix stays byte-identical between window
advances, and an advance happens once per `K` new images.

#### Scenario: Window holds steady between advances

- **GIVEN** `inline_images = 10`
- **AND** 14 earlier screenshots in history
- **WHEN** the agent takes a 15th screenshot and the next request is planned
- **THEN** all 15 images are included
- **AND** the request prefix up to the new tool result matches the previous
  request's prefix byte for byte

#### Scenario: Window advances

- **GIVEN** `inline_images = 10`
- **AND** 19 earlier screenshots in history
- **WHEN** a 20th screenshot is planned
- **THEN** images 10 through 19 are included and images 0 through 9 are
  placeholders

#### Scenario: Many images in one user message

- **GIVEN** `inline_images = 3`
- **WHEN** the user attaches 6 images to one message
- **THEN** all 6 are included in that request
