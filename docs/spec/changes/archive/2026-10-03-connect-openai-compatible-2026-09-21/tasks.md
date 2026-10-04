# Tasks: Connect any OpenAI-compatible endpoint from `/connect`

## 1. Picker and routing (smith-cli)

- [x] 1.1 Add a deduplicated `openai-compatible` connection entry to
      `runtime_resources` in `crates/smith-cli/src/resources.rs`, following the
      existing OpenRouter/ChatGPT/xAI/Google pattern.
- [x] 1.2 Route the `openai-compatible` connect id to `SetupMode::AddProvider`
      in `connection::connect`, and extend the unconfigured-provider error with
      the in-session guidance.
- [x] 1.3 Update the `/connect`/`/disconnect` CLI help line to mention custom
      endpoints.

## 2. Tests

- [x] 2.1 Extend the resource-metadata unit test to assert the
      `openai-compatible` connection entry exists with the expected shape and
      is not marked active.

## 3. Validation

- [x] 3.1 `cargo fmt`, `cargo clippy` on the touched crates, and focused
      `smith-cli` test runs.
