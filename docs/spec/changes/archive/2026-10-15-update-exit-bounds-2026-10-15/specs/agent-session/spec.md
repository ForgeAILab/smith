## MODIFIED Requirements

### Requirement: Graceful process-owned shutdown

Smith MUST return an active-work error before a non-interactive exit when
monitors, children, or background shell tasks remain and the caller chose no
exit policy that covers them. In the TUI, `/quit` SHALL exit without asking,
whatever work is active. Once exit begins, Smith SHALL stop accepting new
work, cancel the running turn and children, terminate owned process groups,
append terminal events, flush canonical records, and exit. Exit begins when
the user quits the TUI, when a non-interactive run starts its shutdown, or
when Smith receives its first SIGINT, SIGTERM, or SIGHUP. Smith MUST exit
within 10 seconds of that moment; work that has not stopped by then SHALL be
killed, and the exit SHALL report that the deadline was reached.

#### Scenario: User quits the TUI with active work

- **GIVEN** a turn, one child, and one background shell task are running
- **WHEN** the user types `/quit`
- **THEN** Smith exits without a confirmation
- **AND** stops the turn, the child, and the task and records each terminal
  reason
- **AND** the exit output names the work it stopped

#### Scenario: Background task is stopped at exit

- **GIVEN** only a background shell task is running
- **WHEN** the user types `/quit`
- **THEN** Smith terminates the task's owned process group and records its
  terminal reason
- **AND** the exit output names the task

#### Scenario: Shutdown cannot finish in time

- **GIVEN** some part of shutdown has not finished 10 seconds after exit
  began
- **WHEN** the deadline passes
- **THEN** Smith kills its remaining owned processes and exits
- **AND** the TUI restores the terminal and a non-interactive run still writes
  its result, which reports the deadline

#### Scenario: Ordinary exit finishes well inside the deadline

- **GIVEN** a turn whose provider honors cancellation
- **WHEN** the user types `/quit` during it
- **THEN** Smith saves the session and exits without waiting for the
  deadline
