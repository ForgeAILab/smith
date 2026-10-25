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

### Requirement: Session artifacts can be listed

When Smith's own artifact store backs a root or child runtime, Smith SHALL
register an `artifact.list` tool beside `artifact.read`. It SHALL return only
artifacts whose protected owner is the invoking session, taken from the
runtime-owned invocation context and never from model arguments. Each row
SHALL carry metadata only: ID, purpose, media type, byte length, the
originating turn and call when recorded, the child that produced it, and the
source session of an artifact copied from a child. The child SHALL come from
the artifact's provenance when Runtime recorded it, otherwise from resolving a
copy's lineage session against the session's own children; it is absent when
neither names one. Rows SHALL be ordered newest first, with ties broken by ID,
and returned in pages of at most 100 rows (default 20) with a total count and a
continuation offset. Optional exact `purpose` and `child_id` filters SHALL
narrow the rows. Listing SHALL request the `artifact.read` permission over a
`session-artifact` resource and SHALL skip unreadable or malformed metadata
records instead of failing the page.

#### Scenario: A session lists its own artifacts

- **GIVEN** a store holding artifacts owned by the session and by another
  session
- **WHEN** the session calls `artifact.list`
- **THEN** only its own artifacts are listed, newest first, without content
- **AND** an `artifact.read` of any listed ID succeeds

#### Scenario: Listing narrows by purpose and pages

- **GIVEN** the session owns more `delegation.child-result` artifacts than one
  page holds and one `delegation.child-reply` artifact
- **WHEN** it lists with `purpose = "delegation.child-reply"`
- **THEN** exactly the reply artifact is returned with `total = 1`
- **AND** an unfiltered first page returns the default page size with a
  continuation offset

#### Scenario: Listing finds one child's reply

- **GIVEN** a child whose reply was stored as a parent-owned copy, and a copied
  artifact whose lineage names that child's session
- **WHEN** the parent lists with that `child_id`
- **THEN** both rows are returned, each naming the child
- **AND** the parent's own tool outputs are excluded and name no child

#### Scenario: No Smith store backs the runtime

- **GIVEN** a runtime whose artifact store was supplied without a listing
  catalog
- **WHEN** Smith registers its tools
- **THEN** `artifact.list` is not registered and no error names it

### Requirement: A missing artifact names how to find real ones

When `artifact.read` fails because the ID does not exist, Smith SHALL return an
error that says the ID does not exist in this session and that artifact IDs
are copied from tool results or host messages, never constructed. When
`artifact.list` is registered for the same runtime, the error SHALL name it.
Ownership and integrity failures SHALL keep Runtime's own errors.

#### Scenario: A model reads an invented ID

- **GIVEN** `artifact.list` is registered
- **WHEN** the model reads an ID that no artifact has
- **THEN** the tool error states the ID does not exist and names
  `artifact.list`
- **AND** no artifact content or other session's metadata is revealed

### Requirement: Provider- and model-aware image token estimation

Smith SHALL install a `RequestSizer` on root and child runtimes that estimates
inline `ContentPart::Image` tokens from the serving provider's post-fit
dimensions and model vision billing regime rather than a flat low-detail
constant. For `data:` image URLs with readable headers, Smith SHALL decode pixel
dimensions from the bounded header, apply the serving provider's `ImageLimits`
dimension fit, and calculate tokens according to the active provider and model
tier:
- Anthropic models SHALL use 28×28-pixel patches (`⌈w/28⌉ × ⌈h/28⌉`) after
  downsizing to the model tier's edge and token budget (2,576 px / 4,784 tokens
  for Claude 4.7+ and unknown Anthropic models; 1,568 px / 1,568 tokens for
  earlier Claude models).
- OpenAI and ChatGPT models SHALL use 32×32-pixel patches with a 2,500-patch
  budget and `⌈patches × multiplier⌉` tokens for patch models, with a
  multiplier of 1.2 for `gpt-5.2+`, `gpt-6*`, `gpt-5-mini`, and unknown models,
  1.5 for `gpt-5-nano`, 1.62 for `gpt-4.1-mini`, 2.46 for `gpt-4.1-nano`, and
  1.72 for `o4-mini`. Tile models SHALL use 512×512 tiles after a 768 px
  shortest-side fit and `base + tile × tiles` tokens: 85 + 170 for `gpt-4o`
  and `gpt-4.1`, 2,833 + 5,667 for `gpt-4o-mini`, 75 + 150 for `o1` and `o3`,
  and 70 + 140 for `gpt-5` and `gpt-5.1`.
- Gemini 1.x and 2.x models SHALL use 258 tokens when both fitted dimensions
  are at most 384 px, and `⌈w/768⌉ × ⌈h/768⌉ × 258` tokens otherwise. Gemini 3
  and 4 models SHALL use 1,120 tokens per image, the maximum at the default
  media resolution. Other Gemini models SHALL use the larger of the two.
- xAI and compatible providers SHALL use the OpenAI 32×32 patch formula on the
  fitted 2,048 px dimensions.

When an image URL is not a `data:` URL or its header dimensions cannot be
decoded, the sizer SHALL estimate 2,500 tokens. Images replaced by text
placeholders before sizing (by the text-only projection or the bounded image
window) SHALL be sized as their placeholder text.

#### Scenario: High-resolution screenshot sized for Claude 4.7+, GPT-5.6, and Gemini

- **GIVEN** a 1920×1080 PNG data URL in a message or tool result
- **WHEN** the request sizer estimates its image token contribution
- **THEN** it estimates 2,691 tokens for Claude 4.7+, 2,448 tokens for
  OpenAI `gpt-5.6`, 1,548 tokens for Gemini 2.5, and 1,120 tokens for
  Gemini 3 and 4

#### Scenario: 4K image is sized using post-fit provider dimensions

- **GIVEN** a 3840×2160 PNG data URL in session history that has not yet been
  re-fitted for the active provider
- **WHEN** the request sizer estimates its token contribution
- **THEN** it applies the active provider's `ImageLimits` fit first and
  estimates 4,784 tokens for Claude 4.7+, 2,765 tokens for OpenAI `gpt-5.6`, and
  3,096 tokens for Gemini 2.5

#### Scenario: Unreadable or remote image uses conservative fallback

- **GIVEN** a remote `https://` image URL or a `data:` image URL with a corrupt
  header
- **WHEN** the request sizer estimates its token contribution
- **THEN** it estimates 2,500 tokens without failing planning

#### Scenario: Older OpenAI models use their own image rates

- **GIVEN** a 1920×1080 PNG data URL
- **WHEN** the request sizer estimates it for `gpt-4.1-mini`, `gpt-4o-mini`,
  and `o3`
- **THEN** it estimates 3,305 tokens (2,040 patches × 1.62), 36,835 tokens
  (2,833 + 6 tiles × 5,667), and 975 tokens (75 + 6 tiles × 150)

#### Scenario: Child policy fingerprints ignore the request sizer

- **GIVEN** a durable child interrupted under a Smith build whose image sizer
  revision differs from the current one
- **WHEN** the parent resumes and the child's policy fingerprint is compared
- **THEN** the sizer revision does not contribute to the fingerprint, so the
  child is not reported as having a changed policy for that reason alone
