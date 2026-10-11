## ADDED Requirements

### Requirement: WASM modules implement the module contract

Smith SHALL load a WASM component as a module through the same contract as
a compiled-in module: the component describes itself, its mount step returns
a list of contributions, and Smith invokes a contribution by its id. The
interface SHALL be one versioned package with one world; a new contribution
kind or host function MUST be added as a new package version and MUST NOT
add a world. Smith SHALL load modules built against the current and the
previous package version and MUST refuse any other version with an error
naming both versions. Contribution kinds in the first version are tools,
context contributors, tool-output processors, and status items.

#### Scenario: Module contributes two tools and a status item

- **GIVEN** a WASM module whose mount step returns two tools and one status
  item
- **WHEN** the module is mounted
- **THEN** both tools are registered through the shared ability registry
- **AND** the status item is offered to clients
- **AND** the module is recorded with the sandboxed trust tier and its
  plugin as provenance

#### Scenario: Module contributes only a context contributor

- **GIVEN** a WASM module whose mount step returns one context contributor
  and no tools
- **WHEN** the module is mounted
- **THEN** the contributor joins the harness pipeline
- **AND** no tool is registered for the module

#### Scenario: Unsupported interface version

- **GIVEN** a WASM module built against an interface version Smith does not
  support
- **WHEN** Smith loads it
- **THEN** the module is reported as failed with both versions named
- **AND** the session starts without it

### Requirement: WASM modules have no ambient authority

A WASM module MUST run with no filesystem, network, environment, or
wall-clock access of its own. Smith SHALL offer host functions only for
capabilities the module's manifest requested and the user granted: file
access under named roots, outbound HTTP to named hosts through Smith's
transport, a key-value store in the plugin's data directory, and read access
to recent session history. Calling a host function for an ungranted
capability MUST fail that call. A tool contributed by a WASM module MUST
remain subject to the ordinary approval path and to the conservative
authority rule applied to unreviewed tools.

#### Scenario: Ungranted network access

- **GIVEN** a WASM module granted no network capability
- **WHEN** its tool calls the HTTP host function
- **THEN** the host function returns an error naming the missing capability
- **AND** no request leaves the machine

#### Scenario: Granted host only

- **GIVEN** a WASM module granted network access to `api.example.com`
- **WHEN** its tool requests `https://other.example.net/`
- **THEN** the request is refused

#### Scenario: Mutating WASM tool needs approval

- **GIVEN** a WASM module contributes a tool that writes files
- **WHEN** the model calls it under `approval.mode = ask`
- **THEN** the user is asked before the tool runs

### Requirement: WASM module calls are bounded and isolated

Every call into a WASM module MUST be bounded by a deadline, a memory cap,
and payload size limits. A trap, timeout, memory exhaustion, oversized
payload, or malformed result MUST fail only that call, MUST NOT end the
session or corrupt canonical session state, and MUST be recorded as a
diagnostic. After repeated consecutive failures Smith SHALL disable the
module for the rest of the session and report it as failed. A failing
pipeline contribution MUST contribute nothing for that phase.

#### Scenario: Tool call times out

- **GIVEN** a WASM tool that never returns
- **WHEN** the model calls it
- **THEN** the call ends at its deadline with a tool error
- **AND** the turn continues

#### Scenario: Module traps repeatedly

- **GIVEN** a WASM module whose calls trap three times in a row
- **WHEN** the third failure is recorded
- **THEN** the module is disabled for the session and listed as failed
- **AND** its tools are no longer offered from the next safe boundary

#### Scenario: Context contributor fails

- **GIVEN** a WASM context contributor that returns a malformed patch
- **WHEN** the provider request is assembled
- **THEN** the request is assembled without that contribution

### Requirement: WASM modules ship inside plugin bundles

A WASM module SHALL be declared in a plugin manifest with the component
file and the capabilities it requests, and SHALL be installed, trusted,
enabled, updated, and removed as part of that plugin. The plugin's digest
MUST cover the component bytes and the capability request, and the install
confirmation MUST show each requested capability. An update that requests a
capability the user has not granted MUST invalidate the plugin's trust and
MUST display the new capability before confirmation.

#### Scenario: Install shows capabilities

- **GIVEN** a plugin declaring a WASM module that requests network access
  to `api.example.com`
- **WHEN** the user installs the plugin
- **THEN** the confirmation lists the module and that host
- **AND** nothing is installed until the user confirms

#### Scenario: Update adds a capability

- **GIVEN** an installed plugin whose WASM module had no file access
- **WHEN** an update requests workspace write access
- **THEN** the plugin is withheld until the user confirms the new capability

## MODIFIED Requirements

### Requirement: Multiple extension tiers without unstable native loading

Smith SHALL support trusted compile-time Rust registration, MCP, and a
sandboxed WASM component host. It MUST NOT load arbitrary Rust dynamic
libraries as a stable extension ABI. The WASM host SHALL implement the same
versioned contribution contract as compiled-in modules without changing
agent-loop semantics.

#### Scenario: Register an MCP tool server

- **GIVEN** a trusted extension manifest declares an MCP server
- **WHEN** its process and tool schemas initialize successfully
- **THEN** Smith registers the MCP tools through the shared ability/tool
  registry
- **AND** applies the same approval and attribution rules as built-ins

#### Scenario: Same contract across tiers

- **GIVEN** a compiled-in module and a WASM module that each contribute one
  tool
- **WHEN** both are mounted
- **THEN** both tools are registered through the same registry path
- **AND** both modules appear in the same module listing with their tier

## REMOVED Requirements

### Requirement: Versioned subprocess extension protocol

**Reason**: Never built. The owner chose a sandboxed WASM host as the tier
for third-party code; out-of-process tools remain available through MCP.

**Migration**: None. No extension uses this protocol.

### Requirement: Optional TypeScript authoring host

**Reason**: Never built, and it depended on the subprocess protocol removed
above.

**Migration**: None. Authors target the WASM interface or ship an MCP
server.
