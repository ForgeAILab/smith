## ADDED Requirements

### Requirement: Harness turns are labelled as externally executed

TUI and headless surfaces SHALL make it visible that a turn ran on an installed
CLI rather than through Smith's own loop, naming which harness ran it.

Tool activity the CLI performed itself MUST be rendered distinctly from Smith's
own tool calls, because Smith did not dispatch it, did not approve it, and
cannot vouch for it. Presenting the two identically would imply a guarantee
Smith cannot make.

#### Scenario: Harness turn renders in the TUI

- **GIVEN** a turn running on a harness profile
- **WHEN** the CLI streams text and its own tool activity
- **THEN** the transcript shows the text as it arrives
- **AND** the turn carries a harness label
- **AND** CLI-run tools are visibly distinct from Smith-dispatched tools

#### Scenario: Headless output distinguishes harness activity

- **GIVEN** a headless run on a harness profile
- **WHEN** machine output is requested
- **THEN** external events appear under their own event types
- **AND** a consumer can tell harness activity from dispatched tool calls
