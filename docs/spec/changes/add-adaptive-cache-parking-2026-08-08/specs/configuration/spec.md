## ADDED Requirements

### Requirement: Layered adaptive cache lifecycle configuration

Smith SHALL expose the following source-explainable cache lifecycle policy:

```toml
[profiles.<name>.context.cache]
maintenance = "adaptive"             # off | observe | adaptive
inactivity_limit_ms = 3600000
max_hold_while_child_ms = 3600000
max_maintenance_calls = 1
max_maintenance_input_tokens = 0      # 0 = exact resolved model/plan budget
max_maintenance_output_tokens = 256
maintenance_deadline_ms = 30000
keepalive_margin_ms = 120000
keepalive_jitter_percent = 10
handoff_checkpoint = true
idle_compaction = true
resume_capsule = true

[profiles.<name>.child_agents]
wait_default_timeout_ms = 5000
wait_max_timeout_ms = 30000
```

`off` SHALL disable synthetic cache maintenance while preserving ordinary
correctness and provider caching. `observe` SHALL record plans, leases, and
provider observations but send no synthetic request. `adaptive` SHALL permit
only actions that also satisfy provider/model capability, adapter conformance,
host authority, lifecycle, continuation-source, and budget policy.

Invalid types, enum values, ranges, or unsupported action combinations MUST
fail closed during configuration resolution. The accepted ranges are
`inactivity_limit_ms: 1_000..=86_400_000` (zero invalid),
`max_hold_while_child_ms: 0..=86_400_000` (zero disables child hold),
`max_maintenance_calls: 0..=8` (zero disables synthetic maintenance),
`max_maintenance_input_tokens: 0..=resolved_model_input_limit` (zero uses the
exact resolved plan/model budget), `max_maintenance_output_tokens: 1..=4096`,
`maintenance_deadline_ms: 1..=120_000`, `keepalive_margin_ms:
0..=inactivity_limit_ms` (zero means no early margin), and
`keepalive_jitter_percent: 0..=50` (zero is deterministic). The wait default
is `0..=30_000` milliseconds and the wait maximum is `1..=30_000`; their
defaults are 5,000 and 30,000 respectively, and the default MUST NOT exceed
the maximum. `agent.wait.timeout_ms = 0` is an immediate status check; a
requested timeout above the resolved maximum is rejected before waiting.
An unsupported provider SHALL remain usable for ordinary turns with cache
status `unsupported`; it MUST NOT be made invalid solely because the requested
maintenance mode was `adaptive`.

#### Scenario: Unsupported provider with adaptive configuration

- **GIVEN** resolved configuration requests `maintenance = "adaptive"`
- **AND** the selected model declares caching unsupported
- **WHEN** Smith resolves the runtime
- **THEN** the ordinary provider runtime remains valid
- **AND** cache status is `unsupported`
- **AND** no synthetic maintenance is scheduled

#### Scenario: Observe mode is selected

- **GIVEN** configuration resolves `maintenance = "observe"`
- **WHEN** a parked parent crosses a known retention boundary
- **THEN** Smith may emit an observation-only scheduling projection
- **AND** sends no keepalive or handoff checkpoint

#### Scenario: Invalid jitter is declared

- **GIVEN** a configuration layer declares a jitter percentage outside the
  accepted bounded range
- **WHEN** Smith resolves the cache policy
- **THEN** preflight fails with the exact key and source
- **AND** no provider or cache request is constructed

### Requirement: Existing idle-compaction setting has one migration path

For one transition release, Smith SHALL accept
`profiles.<name>.context.idle_compaction_ms` as a deprecated alias for
`profiles.<name>.context.cache.inactivity_limit_ms`. The alias SHALL retain
ordinary layered precedence and source provenance. If one layer declares both
keys with different values, resolution MUST fail as ambiguous rather than
choosing by parse or discovery order. Across layers, the normal layer
precedence selects one winner; equal-precedence declarations from different
sources fail as ambiguous and all losing declarations remain explainable.

After resolution, Smith SHALL use one meaningful-inactivity clock and one
effective limit for maintenance cutoff and idle compaction; the alias MUST NOT
create an independent timer. An effective `adaptive` request still requires
explicit host authority `synthetic_cache_spend = allow`; the default is deny
and repository/project configuration cannot grant it. Price or calculated cost
MUST remain presentation-only and MUST NOT participate in dispatch decisions.

#### Scenario: Legacy setting remains compatible

- **GIVEN** a compatible user configuration declares only
  `profiles.work.context.idle_compaction_ms = 900000`
- **WHEN** Smith resolves it during the migration window
- **THEN** the effective `profiles.work.context.cache.inactivity_limit_ms` is
  900000
- **AND** explain output identifies the deprecated source key

#### Scenario: Same layer declares conflicting keys

- **GIVEN** one configuration file declares different values for the legacy
  and replacement keys
- **WHEN** preflight resolves that layer
- **THEN** it reports an ambiguity error naming both keys
- **AND** no runtime or scheduler is constructed

#### Scenario: Equal-precedence alias declarations collide

- **GIVEN** two sources at the same layer declare the legacy and replacement
  keys with different values
- **WHEN** preflight resolves the cache policy
- **THEN** it reports both source locations as ambiguous
- **AND** no value is selected by discovery order

### Requirement: Synthetic maintenance authority fails closed

Smith SHALL resolve synthetic-maintenance authority fail-closed. Repository
configuration MAY narrow or disable synthetic cache maintenance but MUST NOT
grant provider-spend authority withheld by user or host policy. Smith SHALL
retain both requested configuration and effective authority narrowing in
provenance/explain output. Adapter cache support or an active child alone MUST
NOT imply spend authority. Effective synthetic dispatch requires the explicit
host value `synthetic_cache_spend = allow`; otherwise the effective mode is
`observe` or `off`.

The existing `cache.miss_notices` setting SHALL remain presentation-only and
MUST NOT grant, schedule, suppress, or modify cache maintenance.

#### Scenario: Project enables adaptive maintenance

- **GIVEN** project configuration requests adaptive maintenance
- **AND** host policy permits observation but not synthetic provider spend
- **WHEN** Smith resolves the effective run configuration
- **THEN** the effective mechanism is narrowed to `observe` or `off`
- **AND** explain output names the project request and host-policy narrowing

#### Scenario: User disables project-requested maintenance

- **GIVEN** trusted project configuration requests `adaptive`
- **AND** a higher-precedence user or invocation layer requests `off`
- **WHEN** Smith resolves the policy
- **THEN** no synthetic maintenance is possible
- **AND** ordinary cold continuation remains supported

#### Scenario: Miss notices differ but mechanism is identical

- **GIVEN** two runs differ only in `cache.miss_notices`
- **WHEN** both resolve cache lifecycle policy
- **THEN** their maintenance authority and provider request eligibility are
  equivalent
- **AND** only local notice presentation may differ
