## Context

A computer-use MCP server (cmux-cua) is already usable through Smith's MCP
supervisor, but its screenshots are discarded by Agent Runtime's MCP adapter
(`render_blocks`). Its documentation describes them as a context-budget
hazard, with an artifact-store home as a later step. This change makes images
first-class across the MCP path and covers the two problems that follow:
models that cannot take images, and an image count that grows without bound.

What exists at pin `db8dff2`:

- `ContentPart::Image { url, detail }` in tool outcomes, user messages, and
  assistant messages. `generate_image` produces one in its tool result.
- Anthropic (nested `tool_result` image), Responses (`input_image` in
  `function_result_output`, which covers ChatGPT and xAI), and Gemini
  serialize images in tool results.
- OpenAI Chat Completions does not. `validate_openai_messages` allows only
  text inside a `Role::Tool` result and fails the request with
  `BadRequest`, because the wire `tool` message has no image form.
- `CatalogModel.input_modalities` with `CatalogModality::Image`, carried into
  `ModelRecord.input_modalities` and the resolved model profile.
  `factory/provider/resolution.rs` pins command providers to `[Text]`.
- `ContextOffloader` passes multimodal outcomes through unchanged.
- `cli_bridge/server.rs` forwards inline base64 images to a harness CLI.
- `generate_image`'s `RecentImageSource` reads images back out of canonical
  session history.
- The module contract offers a `HistoryProjector`. It can only replace a whole
  old prefix with summaries, so it cannot express per-image substitution.

## Goals / Non-Goals

- Goals:
  - The model sees MCP images, within bounds.
  - A text-only model never receives an image part and never fails a request
    because of one.
  - A text-only main model can ask a configured vision model about an image.
  - Long visual sessions stay under provider image limits without a cache miss
    on every request.
- Non-Goals:
  - Provider-native computer-use tool types.
  - Smith-side resizing or re-encoding of images.
  - Moving image bytes out of the session journal.
  - Images in advisor transcripts.
  - `read` returning images.

## Decisions

### D1. Three runtime changes, one runtime branch

- **R1, MCP passthrough.** `translate` maps `ContentBlock::Image` to
  `ContentPart::Image { url: "data:<mime>;base64,<data>", detail }`.
  `detail` is a host-supplied default; Smith passes `high`, as
  `generate_image` does, because coordinate accuracy depends on resolution.
  New `McpTool` bounds: `max_image_bytes` (decoded bytes per image) and
  `max_images` (per result). Over-bound images, and media types other than
  png, jpeg, gif, and webp, become `[image omitted: <mime>, <n> bytes, <reason>]`.
  Audio and resources keep today's text lines. Smith passes 5 MiB and 8,
  which stays within Anthropic's 5 MB per-image API limit. These are not user
  keys in v1.
- **R3, Chat Completions moves tool-result images.** For each run of
  consecutive tool messages, the adapter sends each result's text as the
  `tool` message. It then sends the run's images in one following `user`
  message, each labelled with its `tool_call_id`, in order. This is the usual
  workaround for this wire format, and it is deterministic, so cache
  identity holds. Without R3, a vision model on a custom endpoint (the
  override scenario) would fail on its first screenshot. It also covers
  `generate_image` on a Chat Completions binding.
- **R2, request image projection.** It runs in the context planner, before
  sizing and before cache identity is computed. Inputs are the canonical
  history, the resolved profile's `input_modalities`, and an
  `ImageWindow { inline: K }`. Output is the planned message list, with each
  dropped image part replaced by a text part:
  `[image <id>: <mime>[ <w>×<h>] not included in this request]`.
  `<id>` is the first 8 hex digits of SHA-256 over the data URL, so it stays
  stable across compaction and resume and needs no stored counter.

R2 also reports, per plan, how many images it replaced because the model is
text-only. Smith uses that count to trigger the one-time notice for a model
with no image-input metadata.

One runtime branch, one Smith pin bump. R1 alone would be unsafe to ship,
because it would deliver images to text-only models that fail on them.
The same goes for R1 without R3 on a Chat Completions binding.

### D2. Projection rather than a tool-output processor

Alternative considered: a Smith `ToolOutputProcessor` that archives images to
the artifact store and substitutes a reference when the main model is
text-only. It needs no runtime change for the text-only case. It was rejected
because it covers only tool results. User attachments, history that survives
a `/model` switch, and the image window would each still need their own
mechanism. The projection covers all four at one point and leaves canonical
history untouched.

### D3. Window advance with hysteresis

Number the image parts in planned history `0..T`. Images in the newest user
message are exempt and always inline. Of the rest, an image at index `i` is
inline when `i >= C`, where `C = K * floor(max(T - K, 0) / K)`. The inline
count stays between `K` and `2K - 1`. `C` changes once every `K` new images,
so between advances the projected prefix is byte-identical, and an advance
costs one cache miss. A second advance source: when a new user message
arrives, the previous message's exempt images join the window count. That is
one extra possible miss per user turn that carried images, which is
acceptable. Default `K = 10` keeps a request at or under 19 window
images, below Anthropic's 20-image threshold for per-image size reduction and
well below its 100-image hard limit.

### D4. Image-input metadata fails closed

The catalog decides. `[models."p/m"] input_modalities` overrides it with
field-level precedence, per the existing catalog rule. With no metadata, the
model is treated as text-only. The runtime's layered profile turns missing
modalities into `[Text]` (`catalog/layered.rs`), so Smith records whether
image-input metadata existed, and where it came from, before layering. That
record feeds both `config explain` and the notice. A false "text-only" gives a placeholder and a
one-time notice naming the override key. A false "vision" gives a provider
400 partway through a loop. The first failure is recoverable and explains
itself; the second is not.

### D5. `vision` mirrors `advisor`

Same value grammar, layering, `extends` inheritance, early rejection, and
`config explain`. Differences:

- A selection whose model lacks image input is a load error. This check
  always runs, whatever the main model is.
- Whether the selection is used is decided at composition, from the active
  model. A vision main leaves the `vision` module inactive. A `/model`
  switch to a text-only model mounts `view_image` at the rebuild boundary
  `/model` already crosses.
- A child profile resolves its own `vision`, as it does `advisor`.

Usage is recorded the same way as advisor usage.

### D6. `view_image` as the `vision` module

The tool is a feature, so it follows the module-kernel direction. It is
compiled-in module `vision`, default on, and mounts `Inactive` with a reason
when no vision model resolves or the main model accepts images. The
projection is correctness and lives in the runtime, so turning the module off
never sends images to a text-only model.

Tool input:

- `image`: a placeholder id from history.
- `path`: a workspace or generated-image path. Mutually exclusive with
  `image`.
- `question`: required, up to 2,000 chars.

The vision request carries:

- A fixed instruction: answer for another agent; give positions as pixel
  coordinates in the image's own W×H space; say when something is not
  visible.
- The image, with its dimensions stated in text.
- The name of the tool that produced the image, when known.
- The question.

It carries no conversation history. History lookup reuses the
`RecentImageSource` approach and resolves an id by digest. An id that has been
compacted away returns a tool error saying so.

### D7. Harness turns are out of scope

Smith does not plan context for `cli/*` turns. Their MCP images pass through
the bridge to a CLI that handles its own vision, and only a regression test
is added.

## Risks / Trade-offs

- **Journal growth.** Screenshots are stored inline in the session journal,
  as `generate_image` output already is. A 200-step loop at about 500 KB per
  image is about 100 MB. The R1 per-image bound limits each entry. An
  artifact-backed image part is the follow-on if resume time suffers.
- **Coordinate scale.** Providers may downscale large images. A model that
  reports coordinates in the downscaled space will click in the wrong place.
  v1 states the original pixel dimensions next to every image and in every
  `view_image` request. cmux-cua's `zoom` with `from_zoom` and its
  scaled-window screenshots reduce the problem. Smith-side scaling is a
  follow-on.
- **Projection cost.** Hashing every image on every plan. The digest is
  memoized per history entry.
- **Fail-closed regression.** Uncatalogued custom vision endpoints lose
  pasted images until the override key is set. The notice names the key.

## Migration Plan

1. Runtime: write the spec change (`add-mcp-image-content` plus the request
   image projection), then implement R1 and R2 on a branch from `db8dff2`
   and run the runtime gate. Push it.
2. Smith: bump the six pins. Add configuration, factory wiring, the `vision`
   module, and display, with tests through the fake provider.
3. Live check against cmux-cua with one vision main model, and with GLM as a
   text-only main model plus a `vision` model.

No persisted-format change: history already holds image parts. Checkpoint
revision constants are not touched.

## Open Questions

- None blocking. The defaults (`K = 10`, image bound) can be tuned after the
  live check.
