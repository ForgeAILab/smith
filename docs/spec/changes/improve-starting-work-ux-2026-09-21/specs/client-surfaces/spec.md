## ADDED Requirements

### Requirement: Starting-work orientation
Smith SHALL show a compact local guide in an empty transcript and SHALL make
help readable from the beginning of the newly requested result. The guide
SHALL use existing commands and SHALL not become canonical history.

#### Scenario: New configured session
- **WHEN** a configured session has no transcript content and is idle
- **THEN** Smith shows how to submit a task, choose a model, connect a provider,
  and discover commands using the existing terminal design tokens
- **AND** the guide disappears when transcript content or active work exists

#### Scenario: Open help after existing conversation
- **WHEN** the user invokes `/help` or the empty-composer `?` shortcut
- **THEN** the viewport begins at the new help result when it exceeds the
  available height, retaining the complete registry and keyboard reference
- **AND** scrolling and subsequent ordinary conversation remain functional

### Requirement: Predictable command-menu activation
Smith SHALL resolve the highlighted completion when the input names no exact
command and SHALL retain exact command arguments and host safeguards.

#### Scenario: Choose from the unfiltered menu
- **WHEN** the user types `/`, highlights `/status`, and presses Enter
- **THEN** Smith runs the local status command without requiring its name
- **AND** the menu shows at most five command rows, scrolling its selected
  window without covering the conversation

#### Scenario: Search by intent
- **WHEN** the user searches `switch` and no command name starts with that term
- **THEN** matching registered command descriptions are offered
- **AND** Tab completes without execution and Enter activates the selection

#### Scenario: Explicit arguments remain authoritative
- **WHEN** input is `/model local/example-model` or a recognized command with
  invalid arguments
- **THEN** the original command parser processes those exact arguments
- **AND** invalid arguments are not silently discarded to activate another row

### Requirement: Actionable picker state
Smith SHALL distinguish empty inventory from unmatched search and SHALL show
selection availability before optional descriptive metadata.

#### Scenario: Filter has no matches
- **GIVEN** a resource inventory contains entries
- **WHEN** its filter matches none
- **THEN** Smith says there are no matches and offers a filter-clearing action
- **AND** Ctrl+U clears the query without selecting or applying a resource

#### Scenario: State survives long metadata
- **WHEN** a current or unavailable resource has long capability metadata
- **THEN** its state label precedes that metadata at normal and narrow widths
- **AND** unavailable resources remain non-selectable

#### Scenario: Narrow picker controls
- **WHEN** a resource picker is open at 44 columns
- **THEN** Enter/choose and Escape/cancel remain visible using shortened hints
- **AND** optional filter guidance yields before those essential actions

### Requirement: Setup correction retains non-secret values
Smith SHALL restore previously entered provider names, endpoints, and model IDs
when the user navigates backward to those fields. Changing a reviewed value
SHALL still invalidate any pending collision approval and require review again.

#### Scenario: Correct an endpoint after choosing authentication
- **WHEN** the user uses Shift+Tab to return to the endpoint or provider field
- **THEN** the existing non-secret value is available for editing
- **AND** no configuration is written until the updated review is confirmed
