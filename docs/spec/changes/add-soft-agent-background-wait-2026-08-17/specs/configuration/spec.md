## MODIFIED Requirements

### Requirement: Source-explainable child-agent wait policy

The resolved child-agent wait policy SHALL expose
`profiles.<name>.child_agents.wait_default_timeout_ms` and
`profiles.<name>.child_agents.wait_max_timeout_ms`. The accepted ranges SHALL
be `0..=300_000` and `1..=300_000` milliseconds respectively, with built-in
defaults of 300,000 milliseconds for both values. The default SHALL NOT exceed
the maximum. A zero default is an immediate status check when explicitly
configured; omitted `agent.wait.timeout_ms` uses the resolved default
foreground boundary.

The five-minute boundary limits only how long the parent model/tool call waits
in the foreground. It MUST NOT grant a child lifetime, token, cancellation, or
provider limit, and a timeout MUST leave the child running in the background.

#### Scenario: Default wait policy is five minutes

- **GIVEN** a profile omits both child-agent wait settings
- **WHEN** Smith resolves the profile
- **THEN** the default and maximum are each 300,000 milliseconds
- **AND** the values retain built-in provenance

#### Scenario: Repository narrows the foreground wait

- **GIVEN** a profile sets `wait_max_timeout_ms = 10000` and a compatible
  default
- **WHEN** Smith resolves the profile
- **THEN** the configured value wins for the foreground wait only
- **AND** it cannot stop or expire a child when the wait ends
