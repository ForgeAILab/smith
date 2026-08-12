## ADDED Requirements

### Requirement: Command-line reasoning effort selection

Smith SHALL accept `--effort <NAME>` anywhere the shared selection parser
accepts selection flags, including `smith`, `smith -p`, `smith config explain`,
and `smith sessions list`. The flag MUST support both spaced and inline forms,
and the client MUST reject a missing value or more than one supplied value.

#### Scenario: Selection surfaces accept both flag forms

- **GIVEN** the invocation uses one provider-advertised effort name
- **WHEN** the user supplies `--effort high` or `--effort=high` to `smith`,
  `smith -p`, `smith config explain`, or `smith sessions list`
- **THEN** the shared selection parser accepts the invocation flag
- **AND** the selected effort remains available to the corresponding client
  surface

#### Scenario: Missing effort value is rejected

- **GIVEN** the invocation contains `--effort` without a value
- **WHEN** Smith parses the command line
- **THEN** Smith rejects the invocation with a non-success usage outcome
- **AND** it does not start the requested client surface

#### Scenario: Repeated effort value is rejected

- **GIVEN** the invocation supplies `--effort` twice, in either supported form
- **WHEN** Smith parses the command line
- **THEN** Smith rejects the invocation with a non-success usage outcome
- **AND** it does not silently choose one value by argument order

### Requirement: Discoverable invocation effort option

Smith SHALL list `--effort <NAME>` in the `RUN OPTIONS` section of `--help` as
selecting a provider-advertised reasoning effort.

#### Scenario: Run help describes effort selection

- **GIVEN** the user requests Smith command-line help
- **WHEN** Smith renders `--help`
- **THEN** `RUN OPTIONS` includes `--effort <NAME>`
- **AND** its description identifies the value as a provider-advertised
  reasoning effort

### Requirement: Local failure for an unadvertised invocation effort

Smith SHALL fail locally, with a non-success exit status, when the requested
invocation effort is not advertised by the resolved provider/model binding.
The diagnostic MUST name the requested value and list the supported
alternatives, without performing credential lookup or issuing a provider
request.

#### Scenario: Unsupported effort names the available alternatives

- **GIVEN** the user supplies an effort absent from the selected binding's
  advertised ladder
- **WHEN** Smith processes the invocation
- **THEN** Smith exits with a non-success status and names the requested effort
- **AND** the diagnostic lists the supported alternatives before any credential
  lookup or provider request

### Requirement: Explicit effort survives interactive startup boundaries

Interactive startup recovery MUST preserve the meaning of an explicitly typed
`--effort`. If the current binding cannot honor that flag, the recovery path
MUST fail with the reasoning diagnostic instead of clearing the typed value and
starting with a notice. When Smith composes a child-profile runtime, it MUST
omit the invocation flag so an uncontrollable child binding does not abort the
parent startup.

#### Scenario: Recovery refuses an unhonorable explicit effort

- **GIVEN** an interactive invocation explicitly supplies `--effort`
- **AND** startup recovery reaches a binding that cannot honor the requested
  reasoning selection
- **WHEN** the recovery path evaluates the binding
- **THEN** startup fails with the reasoning diagnostic
- **AND** Smith does not silently clear the typed flag or start with a notice

#### Scenario: Child profile does not inherit invocation effort

- **GIVEN** the parent invocation explicitly supplies `--effort`
- **AND** Smith composes a runtime for a child profile whose binding cannot
  control reasoning
- **WHEN** the child runtime is started
- **THEN** the child does not receive the parent invocation flag
- **AND** the uncontrollable child binding does not abort parent startup

### Requirement: Invocation effort provenance is user-facing

Smith MUST identify an invocation-supplied effort in `smith config explain
reasoning.effort` output with the source "command-line flag `--effort`". It
MUST NOT render the mechanical `--reasoning-effort` spelling for that source.

#### Scenario: Config explanation uses the typed flag spelling

- **GIVEN** the user supplies an invocation effort with `--effort`
- **WHEN** the user runs `smith config explain reasoning.effort`
- **THEN** the effective entry identifies its source as "command-line flag
  `--effort`"
- **AND** the output does not identify the source as `--reasoning-effort`

### Requirement: Invocation effort remains distinct from reasoning state

The `--effort` flag SHALL select an advertised effort only. It MUST NOT turn
reasoning on or off, and an explicit in-session `/effort` selection MUST remain
the higher-precedence control for the active session's subsequent turn.

#### Scenario: Invocation effort does not toggle thinking

- **GIVEN** the user starts Smith with an advertised `--effort` value
- **WHEN** Smith applies the invocation selection
- **THEN** it selects the requested effort without changing reasoning enabled
  state
- **AND** it does not interpret the flag as a thinking on/off switch

#### Scenario: In-session effort outranks invocation effort

- **GIVEN** the invocation supplies one advertised effort
- **AND** the user selects a different effort through `/effort` in the session
- **WHEN** Smith prepares the next complete turn
- **THEN** the in-session effort is effective for that turn
- **AND** the invocation flag does not override the explicit `/effort`
