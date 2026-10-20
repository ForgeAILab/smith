## ADDED Requirements

### Requirement: Inactive modules may carry a setup hint

A module that reports itself inactive SHALL be able to attach a setup hint.
A hint is a short, safe phrase naming what is missing, the name of a
built-in skill that explains the fix, up to 5 candidate values taken from
the user's configuration (profile names or `provider/model` references), and
optionally the configuration key those candidates are for. With a key and
candidates, the fragment line SHALL tell the agent to offer exactly those
values for that key and no other.
A hint MUST NOT contain credentials, setting values other than those
candidates, or file contents. When at least one inactive module has a hint,
the composition SHALL add one developer fragment, `smith.capability-hints`,
with one line per hint, revisioned by its content. The fragment SHALL tell
the agent to raise the gap with the user only when the current request needs
the missing capability. Without hints, the fragment SHALL be absent. `/modules`
and `smith config modules` SHALL show a short user-facing form of the hint
next to the inactive reason: the guide and the candidates, without the
agent-directed wording.
A hint grants no tool, permission, or approval.

#### Scenario: Hint reaches the agent
- **GIVEN** an inactive module with a setup hint naming `smith.guide.vision`
  and candidate `visual`
- **WHEN** the runtime composition resolves
- **THEN** the planned request carries the `smith.capability-hints` fragment
  naming the gap, `smith.guide.vision`, and `visual`

#### Scenario: Fragment is stable between rebuilds
- **GIVEN** a session with an unchanged mounted set and configuration
- **WHEN** consecutive turns are planned
- **THEN** the `smith.capability-hints` fragment is byte-identical in each

#### Scenario: No hints, no fragment
- **GIVEN** every inactive module reports no setup hint
- **WHEN** the runtime composition resolves
- **THEN** no `smith.capability-hints` fragment is present

#### Scenario: Hint shown in module status
- **GIVEN** an inactive module with a setup hint
- **WHEN** the user runs `/modules`
- **THEN** the module row shows its inactive reason and its hint
