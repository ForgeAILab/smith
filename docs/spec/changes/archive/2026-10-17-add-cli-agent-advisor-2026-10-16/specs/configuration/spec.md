## MODIFIED Requirements

### Requirement: Advisor selection

Smith SHALL select the advisor for a main profile from, in increasing
precedence, the top-level `advisor` key and the profile's own `advisor` key
(inherited through `extends`). The value SHALL be a profile name, a
`provider/model` reference, an installed agent `cli/<kind>/<model>`, or
`false`. `false` SHALL disable the advisor for that profile. A value starting
with `cli/` whose `<kind>` is in the installed-agent catalog SHALL be an
installed agent, never a provider named `cli`. Otherwise, because profile
names never contain `/`, the first `/` SHALL separate the provider from a
model id that may itself contain `/`.

A profile advisor SHALL resolve with that profile's provider, model, settings,
and instructions, whatever its `use` placements. A model advisor SHALL resolve
with the top-level settings and its `[models."<provider>/<model>"]` entry and
no profile's settings or instructions. An installed-agent advisor SHALL
resolve as a model advisor whose model is the agent id. Its provider SHALL be
the main session's resolved provider, which supplies identity and limits and
is never called. Its `[harness.<kind>]` settings SHALL apply. `use` SHALL NOT
accept an `advisor` placement.

Nothing SHALL advise itself. A top-level default naming the active profile
SHALL be skipped for that profile. A profile-level key naming the profile
itself SHALL be rejected. A model or installed-agent advisor naming the
session's resolved model SHALL be skipped. A profile's own advisor selection
SHALL be ignored while it serves as an advisor, because an advisor request
carries no tools. Before credential or provider construction, Smith SHALL
reject an advisor that names an unknown profile, an undeclared provider, an
unknown installed-agent kind, or a malformed value. `smith config explain
advisor` SHALL report the winner and every overridden source.

A `vision` target SHALL NOT be an installed agent; Smith SHALL reject one,
saying an installed agent cannot serve image requests.

#### Scenario: Top-level profile default

- **GIVEN** top-level `advisor = "sol"` and profile `sol` with
  `use = ["main", "child"]`
- **WHEN** profile `code` starts without its own `advisor` key
- **THEN** `code` resolves advisor `sol`

#### Scenario: Model reference

- **GIVEN** top-level `advisor = "chatgpt/gpt-6.1-sol"` and a declared
  `chatgpt` provider
- **AND** profile `code` sets `max_output_tokens = 8192`
- **WHEN** profile `code` starts
- **THEN** the advisor resolves provider `chatgpt` and model `gpt-6.1-sol`
- **AND** the advisor request is not capped by `code`'s `max_output_tokens`
- **AND** carries no profile instructions

#### Scenario: Installed-agent reference

- **GIVEN** top-level `advisor = "cli/claude-code/opus"` and no provider
  named `cli`
- **AND** profile `code` resolves provider `chatgpt`
- **WHEN** profile `code` starts
- **THEN** configuration loads
- **AND** the advisor resolves model `cli/claude-code/opus` with provider
  `chatgpt`
- **AND** the advisor's harness is `claude-code` with model `opus`

#### Scenario: Unknown installed-agent kind

- **GIVEN** `advisor = "cli/emacs/sonnet"` and no provider named `cli`
- **WHEN** configuration loads
- **THEN** Smith reports an unknown provider `cli` at the `advisor` key

#### Scenario: Installed agent as vision target

- **GIVEN** `vision = "cli/claude-code/opus"`
- **WHEN** configuration loads
- **THEN** Smith rejects the `vision` key, saying an installed agent cannot
  serve image requests

#### Scenario: Profile disables the advisor

- **GIVEN** top-level `advisor = "sol"`
- **AND** profile `glm` sets `advisor = false`
- **WHEN** profile `glm` starts
- **THEN** no advisor is resolved

#### Scenario: Advisor profile used as the main profile

- **GIVEN** top-level `advisor = "sol"`
- **WHEN** profile `sol` starts as the main profile
- **THEN** configuration loads and `sol` has no advisor
- **AND** profile `code` still resolves advisor `sol`

#### Scenario: Model advisor names the main binding

- **GIVEN** top-level `advisor = "chatgpt/gpt-6.1-sol"`
- **WHEN** a profile whose provider and model are `chatgpt` and
  `gpt-6.1-sol` starts as the main profile
- **THEN** configuration loads and that session has no advisor

#### Scenario: Installed-agent advisor names the main binding

- **GIVEN** top-level `advisor = "cli/claude-code/opus"`
- **WHEN** a profile whose model is `cli/claude-code/opus` starts as the
  main profile
- **THEN** configuration loads and that session has no advisor

#### Scenario: Unknown profile

- **GIVEN** `advisor = "sool"` and a profile named `sol`
- **WHEN** configuration loads
- **THEN** Smith reports an unknown profile at the `advisor` key, suggesting
  `sol`
- **AND** no credential is read

#### Scenario: Undeclared provider

- **GIVEN** `advisor = "missing/model"` and no provider named `missing`
- **WHEN** configuration loads
- **THEN** Smith reports an unknown provider at the `advisor` key
