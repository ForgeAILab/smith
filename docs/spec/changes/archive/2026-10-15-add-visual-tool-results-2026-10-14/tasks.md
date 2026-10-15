---
created_at: 2026-10-14T23:48:06Z
updated_at: 2026-10-15T04:09:16Z
completed_at: 2026-10-15T04:09:16Z
---

Order matters: the runtime branch lands and is pushed before Smith bumps its
pins. R1 never ships without R2 (design D1).

## 1. Agent Runtime (new branch from `db8dff2`)

- [x] 1.1 Write the runtime spec change for MCP image content and request
      image projection in the runtime repo's `docs/spec/changes/`.
- [x] 1.2 R1: in `agent-runtime-mcp/src/tool.rs`, map `ContentBlock::Image`
      to `ContentPart::Image` data URLs. Add `max_image_bytes` and
      `max_images` bounds and an allowed-media-type check. Drop over-bound
      images to a text line. Unit tests: in bounds, over size, over count,
      unsupported mime, mixed text and image, and the text bound not
      counting image bytes.
- [x] 1.2a R1 takes a host-supplied default `detail`.
- [x] 1.2b R3: in `agent-runtime-provider/src/openai.rs`, allow images in
      `Role::Tool` results. Send each run's images in one following `user`
      message, labelled by `tool_call_id`. Tests: one result, several
      results in one run, text-only results unchanged (byte-identical wire
      output), and a stable serialization for cache identity.
- [x] 1.3 R2: request image projection in the `agent-runtime-context`
      planner, before sizing and cache identity. Text-only replacement,
      the window with hysteresis, the newest-user-message exemption,
      digest ids (memoized), and dimensions from PNG, JPEG, GIF, and WebP
      headers.
- [x] 1.4 R2 tests: deterministic output for the same history and `K`;
      byte-identical prefix between advances; advance at `T = 2K`; a
      text-only profile never sees an image part; ids stable across
      compaction of earlier messages.
- [x] 1.5 Expose the projection policy (`ImageWindow`) on the runtime
      builder and the per-request input-modality source. Report each plan's
      count of images replaced because the model is text-only.
- [x] 1.6 Runtime gate: `cargo fmt --check`, `cargo clippy --workspace
      --all-targets --all-features -D warnings`, and `cargo test --workspace
      --all-features`. Push the branch.

## 2. Smith pin and configuration

- [x] 2.1 Bump the six `agent-runtime*` pins and run `cargo update -p
      agent-runtime --precise <rev>`.
- [x] 2.2 `ModelSection.input_modalities` with field-level precedence over
      the catalog. Command providers stay forced to text-only. Record
      whether image-input metadata existed, and its source, before runtime
      layering turns missing metadata into `[Text]`. Add it to
      `config explain`.
- [x] 2.3 `vision` key at the top level and per profile, sharing the advisor
      selection parser. Reject a selection without image input at load
      time, whatever the main model is. Add `smith config explain vision`.
- [x] 2.4 `context.inline_images`: default 10, range 1..=100, with
      provenance.
- [x] 2.5 Configuration tests for every scenario in
      `specs/configuration/spec.md`.

## 3. Smith factory wiring

- [x] 3.1 Pass `ImageWindow { inline: context.inline_images }` and the
      resolved input modalities to the root and child runtimes.
- [x] 3.2 Configure the MCP image bounds (5 MiB, 8) where the supervisor
      builds `McpTool`s.
- [x] 3.3 Resolve the vision binding (provider, credential, model) the same
      way the advisor binding is resolved, and hand it to the module
      context.
- [x] 3.4 Emit the one-time "no image-input metadata" notice the first time
      a plan reports a replaced image on a model recorded as having no
      metadata (task 2.2).
- [x] 3.5 Decide the `vision` module's activation at each composition from
      the active model, so a `/model` switch mounts or unmounts
      `view_image`.

## 4. `vision` module (`crates/modules/vision`)

- [x] 4.1 Add the crate and cargo feature, register it in the compiled-in
      module list, and mount `Inactive` with a reason when it cannot
      serve.
- [x] 4.2 `view_image`: schema; a description that maps `[image <id>: …]`
      placeholders to the `image` argument; `prepare` validation (`image` xor `path`,
      question length, path roots shared with `generate_image`), effects
      for network and data egress to the vision endpoint, and a host read
      for `path`.
- [x] 4.3 Resolve an image by digest from canonical history (reuse the
      `RecentImageSource` approach). Return an error for a compacted id.
- [x] 4.4 Vision request: fixed instruction, image, dimensions, source tool
      name, and question. Bounded output. Record usage like the advisor.
- [x] 4.5 Fake-provider tests: locate-control round trip, workspace path,
      compacted id, path outside the roots, and approval required. Script
      `registry.activate` before calls.

- [x] 4.6 Gate fixes: composition futures overflow the 2 MiB test/worker
      stack after the vision request was added; vision test fixtures; module
      expectations; architecture allowlist; docs section opening sentence.
- [x] 4.7 Price vision usage at the vision model's catalog rate in status
      accounting, as advisor usage is.

## 5. Display

- [x] 5.1 Show image lines in tool-result previews, live and resumed.
- [x] 5.2 `view_image` rows: in progress, completed with an answer excerpt,
      and failed.
- [x] 5.3 Render tests for both.

## 6. Cross-path checks

- [x] 6.1 Command-JSONL main plus an MCP image: the request succeeds with a
      placeholder.
- [x] 6.2 `/model` switch from vision to text-only and back.
- [x] 6.3 CLI bridge regression: a bridged MCP image reaches the harness as
      an image block.
- [x] 6.4 `docs/configuration.md`: `vision`, `input_modalities`, and
      `inline_images`.

## 7. Gate and live check

- [x] 7.1 Smith gate: `--no-fail-fast`, Rust 1.88 clippy, and CI's
      `--no-default-features` steps. Push a branch for CI before main.
- [x] 7.2 Live run against cmux-cua with a vision main model: screenshot,
      locate, click, and confirm the window advance in `SMITH_LOG`.
- [x] 7.3 Live run against cmux-cua with GLM as main and a `vision` model,
      using `view_image` to ground a click.
