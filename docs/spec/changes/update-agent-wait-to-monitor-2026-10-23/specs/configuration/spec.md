## MODIFIED Requirements

### Requirement: Source-explainable child-agent wait policy

The resolved child-agent wait policy SHALL expose
`profiles.<name>.child_agents.wait_default_timeout_ms` and
`profiles.<name>.child_agents.wait_max_timeout_ms`. The accepted ranges SHALL
be `0..=300_000` and `1..=300_000` milliseconds respectively, with built-in
defaults of 300,000 milliseconds for both values. The default SHALL NOT exceed
the maximum. A zero default is an immediate status check when explicitly
configured; omitted `agent.wait.timeout_ms` uses the resolved default wait
timer duration, and a requested value above the maximum is clamped to it.

These values bound only the wait timer after which an idle parent is woken.
They MUST NOT hold a parent tool call open, and MUST NOT grant a child
lifetime, token, cancellation, or provider limit; timer expiry MUST leave the
child running.

#### Scenario: Default wait policy is five minutes

- **GIVEN** a profile omits both child-agent wait settings
- **WHEN** Smith resolves the profile
- **THEN** the default and maximum are each 300,000 milliseconds
- **AND** the values retain built-in provenance

#### Scenario: Repository narrows the wait timer

- **GIVEN** a profile sets `wait_max_timeout_ms = 10000` and a compatible
  default
- **WHEN** Smith resolves the profile
- **THEN** the configured value bounds the wait timer only
- **AND** it cannot stop or expire a child when the timer fires
