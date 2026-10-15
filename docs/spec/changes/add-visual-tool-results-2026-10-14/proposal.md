---
created_at: 2026-10-14T23:48:06Z
updated_at: 2026-10-15T04:09:16Z
---

## Why

Smith can already connect a computer-use MCP server such as cmux-cua, and its
click, type, and accessibility-tree tools work. The model is blind, though.
Agent Runtime's MCP adapter turns every image block into the line
`[image omitted: image/png, N base64 bytes]` (`agent-runtime-mcp`
`tool.rs` `render_blocks`), so a `get_window_state` screenshot, a `zoom`
crop, or a `get_desktop_state` capture never reaches the model. A loop that
needs pixels cannot ground itself.

Most of the rest of the path exists:

- `ContentPart::Image` already travels in tool results (`generate_image`
  returns one).
- The Anthropic, Responses (and so ChatGPT and xAI), and Gemini adapters
  serialize images inside tool results.
- The text offloader never offloads multimodal content.
- The CLI bridge already forwards inline images to a harness.

The exception is the OpenAI Chat Completions adapter, which custom and
OpenAI-compatible endpoints use. It rejects any tool result that is not pure
text.

Two gaps open once images flow. A text-only main model, such as GLM or any
command-JSONL provider, cannot accept an image: the command provider fails
the whole request today, and other endpoints return a 400. Owner direction
on 2026-10-14: let the user name a vision model that the main agent can use
for images. Second, a computer-use loop takes a screenshot each step, so a
session collects images until it reaches a provider's per-request image
limit.

## What Changes

- **MCP image results reach the model.** An MCP `image` block becomes an
  image part in the tool result instead of a text line. Images have their
  own bounds: a maximum size per image and a maximum count per result,
  separate from the text output limit. An image over a bound becomes a text
  placeholder that gives its media type and size.
- **Chat Completions carries tool-result images.** That adapter moves a tool
  result's images into a user message placed right after the tool messages,
  instead of rejecting the request.
- **Every request sees images only where the model can take them.** Agent
  Runtime gains a request-time image projection. Canonical history keeps
  every image. The planned request replaces an image with a deterministic
  placeholder (`[image 3f9a2c1b: image/png 1440×900 not included]`)
  - always, when the active model does not accept image input; and
  - beyond a window of the newest images when it does. The window advances
    in steps, so the request prefix stays byte-stable between advances and
    prompt caching keeps working.

  This covers tool-result images, user-attached images, and history carried
  across a `/model` switch to a text-only model.
- **Image input is known from metadata, not guessed.** Smith uses the
  catalog's `input_modalities`. A new `[models."<provider>/<model>"]
  input_modalities` key overrides it. A model with no image-input metadata
  is treated as text-only. A command-JSONL provider is always text-only.
- **A `vision` model for text-only agents.** A new `vision` key, mirroring
  `advisor`, takes a profile name, a `provider/model` reference, or `false`.
  It is set at the top level and per profile, and is checked at load time
  whatever the main model is. When the active model is text-only, including
  after a `/model` switch, the agent gets a `view_image` tool.
  The tool sends one image (a placeholder id from history, or a workspace
  image path) and a question to the vision model, and returns its text
  answer. The answer reports positions in that image's pixel space. The
  `vision` model must accept image input, or configuration fails to load.
  It ships as the compiled-in module `vision`.
- **`[context] inline_images`** sets the image window, default 10, range
  1..=100. Images in the newest user message are never dropped.
- **Display.** A tool result that carries images shows each image's media
  type and pixel dimensions in its preview. A `view_image` row shows the
  image id and question, then the answer.

## Impact

- Affected specs: `extension-system`, `configuration`,
  `context-working-set`, `tool-execution`, `tool-call-display`
- Affected code:
  - Agent Runtime (new branch from the pinned `db8dff2`, with its own spec
    change): `agent-runtime-mcp/src/tool.rs` (image passthrough and
    bounds), `agent-runtime-provider/src/openai.rs` (move tool-result
    images into a user message), and the request planner in
    `agent-runtime-context` (image projection before sizing, which keeps
    cache identity exact). Then the
    six pins in Smith's `Cargo.toml` are bumped.
  - Smith: `smith-config` (`vision`, `input_modalities`, `inline_images`,
    `config explain`), `smith-runtime/src/factory` (resolve the vision
    binding, pass the image policy to the runtime), a new
    `crates/modules/vision` crate (`view_image`), `smith-tui` (image
    previews, `view_image` rows), `docs/configuration.md`.
- Compatibility: a session on a model with no image-input metadata (an
  uncatalogued custom endpoint) no longer sends pasted images. It sends a
  placeholder instead, and Smith states once why and which key to set.
  Catalogued vision models are unaffected except for the image window.
  A command-JSONL session that received an image used to fail. It now gets
  a placeholder.
- Security: `view_image` sends image bytes to the vision model's provider
  endpoint. It declares that endpoint's network and data-egress effects, so
  approval applies. A path argument resolves only inside the workspace or
  Smith's generated-image directory, as `generate_image` references do.
- Not in scope: provider-native computer-use tools (Anthropic `computer`,
  OpenAI computer-use); resizing images; images in advisor transcripts;
  `read` returning image files; storing images outside the session journal
  (see Risks in `design.md`); context planning for harness (`cli/*`) turns,
  whose MCP images already pass through the bridge unchanged.
