## MODIFIED Requirements

### Requirement: Child management controls

Smith SHALL expose root-only operations to spawn, list, send or follow up, wait,
fetch result, resume, and stop direct children. Operations MUST be addressed by
stable child ID and return structured lifecycle or error results.

The `agent.wait` operation SHALL NOT hold the parent tool call open while a
child runs. It SHALL accept an optional `timeout_ms` whose resolved
configuration paths SHALL be
`profiles.<name>.child_agents.wait_default_timeout_ms` and
`profiles.<name>.child_agents.wait_max_timeout_ms`. The default and maximum
ranges SHALL be `0..=300_000` and `1..=300_000` milliseconds respectively, and
the built-in values SHALL both be 300,000 milliseconds. The resolved default
MUST NOT exceed the resolved maximum. A requested timeout of zero SHALL be an
immediate status check that arms nothing; a requested timeout above the
resolved maximum SHALL be clamped to that maximum and the result SHALL report
that it was clamped. Positive timeouts SHALL NOT be raised to a minimum.

When the addressed child is terminal at call time, `agent.wait` SHALL return
its terminal status and acknowledge the outcome as `agent.result` does. When
the child is running and the timeout is non-zero, `agent.wait` SHALL return
immediately with a structured `running` result that reports the armed wake
duration and deadline, and SHALL arm a wait timer for that child as "Timed
wait wakes an idle parent" requires. The child's lifecycle, limits, and exact
state MUST be unchanged, and its terminal outcome MUST remain must-deliver for
automatic delivery. The model-facing description SHALL state that `wait`
returns at once, that the parent should end its turn, and that it will be
woken by the child's terminal outcome or by the timer, whichever comes first.

#### Scenario: Waiting on a running child returns at once

- **GIVEN** the root calls `agent.wait` for a child that remains active
- **WHEN** the call is executed
- **THEN** `agent.wait` returns a successful `running` result reporting the
  armed wake duration and deadline without waiting for the child
- **AND** the parent tool call is released for normal completion or new work
- **AND** the child continues running with its original lifecycle and limits

#### Scenario: Waiting on a child that already finished

- **GIVEN** the root calls `agent.wait` for a child whose task is terminal
- **WHEN** the call is executed
- **THEN** `agent.wait` returns the terminal/idle child status
- **AND** the outcome is acknowledged so no child-completion continuation
  delivers it again
- **AND** no wait timer is armed

#### Scenario: Zero timeout is a status check

- **GIVEN** the root requests `timeout_ms = 0` for a running child
- **WHEN** the call is executed
- **THEN** Smith returns the child's current status
- **AND** arms no wait timer

#### Scenario: Requested timeout above the maximum

- **GIVEN** the resolved maximum is 300,000 milliseconds
- **WHEN** the root requests `timeout_ms = 900000` for a running child
- **THEN** Smith arms a 300,000 millisecond timer
- **AND** the result reports the effective duration and that it was clamped

#### Scenario: Short timer is not raised to a floor

- **GIVEN** the root requests `timeout_ms = 1` for a running child
- **WHEN** the call is executed
- **THEN** Smith arms a one millisecond timer and returns immediately
- **AND** guidance tells the parent to end its turn and await the automatic wake

### Requirement: Parked-state shutdown and headless behavior

A parked parent SHALL remain cancellable and SHALL shut down in bounded order.
On shutdown, Smith SHALL stop cache-maintenance scheduling, cancel every armed
wait timer, cancel in-flight synthetic requests, freeze child-completion and
wait-timeout admission, apply existing child stop/wait/durability policy,
persist the latest compatible resume capsule when enabled, release
provider/cache resources according to adapter policy, and emit one terminal
shutdown sequence.

A headless host MAY remain process-alive while required child work is pending
under ordinary child policy, but MUST NOT remain alive solely to preserve a
provider cache or to honor an armed wait timer after required work is
complete.

#### Scenario: User exits while parent is parked

- **GIVEN** a parked parent with a running child and an armed wait timer
- **WHEN** the host begins shutdown
- **THEN** no new cache maintenance, child-completion, or wait-timeout turn
  starts
- **AND** the configured child shutdown policy applies
- **AND** shutdown remains bounded

#### Scenario: Required child work completes in headless mode

- **GIVEN** a headless host stayed alive to receive a required child outcome
- **WHEN** the outcome is integrated and no other required work remains
- **THEN** the host completes its ordinary terminal sequence
- **AND** it does not extend process life for cache retention or a wait timer

### Requirement: Delegation parking tests

Smith SHALL provide deterministic delegation tests covering at least:

1. a parent turn completing while a child remains running;
2. entry into parked state without provider I/O;
3. `agent.wait` returning `running` immediately and arming a timer;
4. terminal child result retention through the must-deliver channel;
5. one idle-parent child-completion continuation;
6. real user input winning concurrent admission;
7. deterministic ordering of multiple terminal outcomes;
8. child progress not refreshing parent cache or inactivity clocks;
9. the maximum child hold stopping maintenance while the child remains alive;
10. bounded parked-state shutdown;
11. one idle-parent wait-timeout continuation on timer expiry;
12. a terminal outcome cancelling an armed timer and winning a race with
    expiry; and
13. a timer expiring while the parent serves being delivered at the next safe
    boundary without a competing turn.

The fixtures MUST assert coordinator, admission, provider-call, cache-clock,
usage-purpose, timer, and persistence effects rather than only TUI
presentation.

#### Scenario: Delegation parking matrix runs

- **GIVEN** a scripted coordinator, admission boundary, clock, and provider
- **WHEN** each parking, timer, and race fixture runs
- **THEN** outcome delivery and provider turn counts match the listed contract
- **AND** no wait timer, progress event, or admission race loses a terminal
  outcome or delivers it twice

### Requirement: Profile-controlled root delegation availability

Smith SHALL allow an effective main-agent profile to narrow root delegation
availability. When delegation is disabled, the runtime MUST omit both the
delegation instruction fragment and the `agent` tool from the sealed
capability registry; child surfaces MUST remain non-delegating regardless of
profile configuration.

#### Scenario: Disable delegation for a main profile

- **GIVEN** the effective main profile has delegation disabled
- **WHEN** Smith composes the runtime and prompt
- **THEN** no delegation prompt fragment is contributed
- **AND** no `agent` tool schema or callable delegation route is registered
- **AND** every unrelated eligible tool remains unchanged

#### Scenario: Preserve existing main-profile behavior

- **GIVEN** an existing main profile omits delegation availability
- **WHEN** Smith composes the runtime and prompt
- **THEN** root delegation remains enabled under the existing host policy
- **AND** the prompt and tool registry continue to agree

#### Scenario: Child profile attempts to enable delegation

- **GIVEN** a direct child resolves a profile whose delegation field is true or
  omitted
- **WHEN** Smith composes the child runtime
- **THEN** the child receives no delegation prompt or tool
- **AND** the one-level hierarchy remains enforced

#### Scenario: Waiting for a running child does not block

- **GIVEN** a root invokes `agent.wait` for a child that remains running
- **WHEN** the tool executes
- **THEN** the tool returns a structured still-running outcome to the root
  without waiting for the child
- **AND** the child remains running and unchanged
- **AND** the enclosing invocation cancellation and deadline remain
  authoritative

### Requirement: Delegation guidance favors completed outcome delivery

Smith's root delegation guidance SHALL tell the model to continue independent
work or end its turn and park while a child runs, receive completed outcomes
through automatic delivery, use `agent.wait` only to set a deadline at which it
wants to be woken, and use transcript inspection on demand for understanding or
diagnosis. It SHALL discourage repeated status/transcript polling and
repeated re-arming as a way to obtain completion and SHALL distinguish child
control from shell-task control.

The guidance and the `agent` results that start or re-arm child work SHALL
tell the model that ending its turn means ending the response with no further
tool calls, that a child's outcome reaches it only as a host message starting a
later turn, and that it must never write that message, announce an unreceived
completion, or guess a child's findings. The guidance SHALL require artifact
IDs to be copied exactly from a tool result or host message and SHALL say that
an `artifact not found` error means the ID is wrong, not that another guess
should be tried. The guidance SHALL NOT quote the host's delivery sentinels
(the child-completion prefix or internal-turn source kinds), so that those
strings keep identifying genuine host messages.

#### Scenario: The parent has no independent work remaining

- **GIVEN** a child is still running and no independent parent work remains
- **WHEN** Smith presents delegation guidance
- **THEN** the guidance directs the parent to end its turn and await automatic
  delivery, optionally after arming a wait deadline
- **AND** it does not direct repeated reads of partial child output

#### Scenario: The parent is woken by a wait timer

- **GIVEN** a wait-timeout continuation reports a child still running
- **WHEN** Smith presents delegation guidance
- **THEN** the guidance lets the parent re-arm, inspect, stop, or continue
- **AND** it does not suggest re-arming as the default

#### Scenario: The parent needs to stop a child

- **GIVEN** the parent is diagnosing a child rather than a background shell task
- **WHEN** Smith describes the available controls
- **THEN** it names `agent` action `stop` with the child ID
- **AND** it identifies `task_stop` as a shell-task operation

#### Scenario: The parent announces that it is waiting

- **GIVEN** the parent has spawned or followed up a child and has no
  independent work
- **WHEN** Smith presents delegation guidance and the `agent` result
- **THEN** both direct the parent to end its response with no further tool
  calls
- **AND** both forbid writing, predicting, or announcing the child's outcome
  before a host message delivers it

#### Scenario: The parent reads a child's artifact

- **GIVEN** a child-completion delivery named an artifact ID
- **WHEN** Smith presents delegation guidance
- **THEN** the guidance requires that ID to be copied exactly
- **AND** treats `artifact not found` as a wrong ID rather than a cue to retry
  with a constructed one

#### Scenario: Delivery sentinels stay unique

- **GIVEN** the root delegation guidance is in the parent's context
- **WHEN** a provider request is inspected for a child-completion or
  wait-timeout delivery
- **THEN** neither the child-completion prefix nor either internal-turn source
  kind appears in the guidance text

## ADDED Requirements

### Requirement: Timed wait wakes an idle parent

Smith SHALL keep at most one armed wait timer per direct child; arming a child
again SHALL replace its earlier deadline, and a timer SHALL never re-arm
itself. Injecting a terminal outcome for a child SHALL cancel its timer. When a
timer expires and the child is still nonterminal, Smith SHALL queue a bounded
host-authored status notice naming the child ID, profile, `running` state,
elapsed time, and armed duration, without partial child output, and SHALL
attempt one idle-only internal continuation attributed
`delegation.wait-timeout`. When the child is terminal at expiry, the timer
SHALL be dropped and completion delivery SHALL take precedence.

The continuation SHALL follow the admission rules of child-completion
continuations: it MUST NOT queue behind or run concurrently with real user
work or another parent turn, MUST NOT fabricate a user-role message, and
SHALL batch every expired, still-running child ready at admission in
deterministic child-ID order. A notice that cannot be admitted because the
parent is serving SHALL be supplied at the next safe boundary. Armed timers
SHALL NOT be persisted; restart reconciliation supersedes them.

#### Scenario: Timer expires while the parent is parked

- **GIVEN** a parked parent armed a five-minute wait on a running child
- **WHEN** five minutes pass and the child is still running
- **THEN** Smith admits one `delegation.wait-timeout` continuation
- **AND** it carries the child's bounded status without partial output
- **AND** the child keeps running

#### Scenario: Child finishes before the timer

- **GIVEN** a parked parent armed a wait on a running child
- **WHEN** the child reaches a terminal outcome before the deadline
- **THEN** the timer is cancelled
- **AND** only the `delegation.child-completion` continuation runs

#### Scenario: Expiry races completion

- **GIVEN** a timer expires at the same boundary at which the child's terminal
  outcome is injected
- **WHEN** admission is serialized
- **THEN** the completion is delivered
- **AND** no wait-timeout continuation runs for that child

#### Scenario: Timer expires while the parent is serving

- **GIVEN** the parent is processing a user turn
- **WHEN** an armed timer expires for a running child
- **THEN** no competing internal turn starts
- **AND** the status notice is supplied at the next safe boundary

#### Scenario: Re-arming replaces the deadline

- **GIVEN** a child has an armed timer
- **WHEN** the parent calls `agent.wait` on it again with a new timeout
- **THEN** only the new deadline remains armed

#### Scenario: Restart drops armed timers

- **GIVEN** a parked parent has an armed timer for a running child
- **WHEN** the process exits and the session resumes
- **THEN** no wait-timeout continuation is admitted for the old timer
- **AND** the child is reconciled under the existing restart rules
