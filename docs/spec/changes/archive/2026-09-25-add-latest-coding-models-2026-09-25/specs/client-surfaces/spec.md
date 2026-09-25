## ADDED Requirements

### Requirement: Current installed-agent model choices

Smith SHALL list GPT-6 Astra, GPT-6 Sol, and GPT-6 Luna as selectable models
for an installed Codex CLI. Smith SHALL use Claude Code's stable rolling model
aliases rather than pinning dated Claude CLI model identifiers.

#### Scenario: Codex exposes current GPT-6 choices

- **GIVEN** `codex` is available on `PATH`
- **WHEN** the user opens Smith's model picker
- **THEN** `cli/codex/gpt-6-astra`, `cli/codex/gpt-6-sol`, and
  `cli/codex/gpt-6-luna` are selectable

#### Scenario: Claude tracks the latest version through an alias

- **GIVEN** an updated `claude` executable is available on `PATH`
- **WHEN** the user selects `cli/claude-code/opus`
- **THEN** Smith passes `opus` to Claude Code
- **AND** Claude Code resolves that alias to its current Opus release
