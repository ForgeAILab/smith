## ADDED Requirements

### Requirement: Runtime-scoped background task services

Background shell, output, stop, exit-policy, and shutdown operations SHALL use
an explicitly injected task service owned by the composing Smith host. Tool
execution MUST NOT discover a first-install-wins process-global host or task
registry, and two hosts in one process MUST NOT observe or control each other's
tasks.

#### Scenario: Two embedded Smith hosts run tasks

- **GIVEN** two Smith hosts composed in one process with separate background
  services
- **WHEN** each starts a background task
- **THEN** each host lists, polls, stops, and shuts down only its own task
- **AND** installing or dropping one host does not change the other's service

#### Scenario: Direct embedder omits background service

- **GIVEN** a direct embedder supplies no background task service
- **WHEN** it resolves built-in capabilities
- **THEN** background-capable tools are omitted or fail preflight explicitly
- **AND** no ambient global service is consulted

### Requirement: Task stop returns acknowledged terminal state

A successful `task_stop` call for a running task SHALL wait within the bounded
cleanup period until the owned process group is terminated, one terminal state
is committed, and its terminal notification is enqueued. The first successful
result MUST report that terminal state and MUST NOT report `running`; an
already-terminal task remains idempotent and an unconfirmed cleanup returns an
error rather than false success.

#### Scenario: Stop a running task once

- **GIVEN** a task is running when `task_stop` is invoked
- **WHEN** the worker accepts and completes the stop
- **THEN** the first tool result reports `stopped`
- **AND** exactly one terminal journal record and notification exist

#### Scenario: Task exits while stop is requested

- **GIVEN** a task naturally exits while a stop request races it
- **WHEN** both terminal paths converge
- **THEN** `task_stop` returns the single winning terminal state
- **AND** no second terminal record or notification is emitted

#### Scenario: Process cleanup exceeds its bound

- **GIVEN** Smith cannot confirm process-group termination within the cleanup
  deadline
- **WHEN** `task_stop` reaches that deadline
- **THEN** it returns a bounded error or explicit unconfirmed outcome
- **AND** it does not claim that the task stopped
