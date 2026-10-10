## ADDED Requirements

### Requirement: Compiled-in module contract

Smith SHALL define one module contract for features compiled into the
binary. A module MUST declare a stable id, a revision, and the module ids it
requires, and MUST supply its contributions as values returned from a mount
step rather than by mutating shared runtime state. Contributions SHALL be
limited to tools, harness pipeline components, event observers, slash
commands, and declarative status items, and each executable contribution
MUST adapt to the corresponding Agent Runtime contract. Smith MUST NOT add a
second event bus for modules.

#### Scenario: Module contributes a tool

- **GIVEN** a compiled-in module whose mount step returns one tool
- **WHEN** the module is mounted and the runtime composition resolves
- **THEN** the tool is registered through the shared ability registry
- **AND** it is subject to the same approval and attribution rules as any
  built-in tool

#### Scenario: Module contributes a pipeline component

- **GIVEN** a compiled-in module whose mount step returns a context
  contributor
- **WHEN** the runtime composition resolves
- **THEN** the component joins the harness pipeline in its declared phase
- **AND** its order is decided by the pipeline's own ordering constraints

#### Scenario: Mount step fails

- **GIVEN** a module whose mount step returns an error
- **WHEN** the runtime composition resolves
- **THEN** none of that module's contributions are present
- **AND** Smith reports the module and the reason
- **AND** the session starts with the remaining modules

### Requirement: Mounting is reversible

The set of mounted modules SHALL fully determine the module contributions in
a runtime composition. Unmounting a module MUST leave no tool, pipeline
component, observer, command, or status item of that module in the next
composition. A change to the mounted set MUST take effect only at a safe
boundary and MUST NOT interrupt a turn in progress.

#### Scenario: Module is switched off during a session

- **GIVEN** `image-generation` is mounted and a turn is running
- **WHEN** the user switches the module off
- **THEN** the running turn completes with its existing tool set
- **AND** from the next safe boundary `generate_image` is not registered

#### Scenario: Module is switched back on

- **GIVEN** `image-generation` was switched off earlier in the session
- **WHEN** the user switches it on
- **THEN** from the next safe boundary its contributions are present again
- **AND** the composition equals one in which it was never switched off

### Requirement: Module requirements gate mounting

Smith SHALL mount modules in an order that satisfies their declared
requirements. A module whose required module is not compiled in, is switched
off, or failed to mount MUST NOT mount, and Smith MUST report it with the
unmet requirement. A requirement cycle MUST be rejected at composition time.

#### Scenario: Required module is switched off

- **GIVEN** module `b` requires module `a`
- **AND** `a` is switched off
- **WHEN** the runtime composition resolves
- **THEN** `b` is not mounted
- **AND** the module listing shows `b` as blocked by `a`

#### Scenario: Requirement cycle

- **GIVEN** modules `a` and `b` each require the other
- **WHEN** the runtime composition resolves
- **THEN** neither is mounted
- **AND** Smith reports the cycle naming both modules

### Requirement: Mounted modules are recorded as modules

Each mounted module SHALL be recorded in composition evidence as a module
with its provenance, trust tier, and contributions, the same record shape
used for MCP servers. A module that is not mounted MUST contribute no
record. A module record MUST NOT grant any capability its contributions did
not already declare and receive.

#### Scenario: Unmounted module leaves no record

- **GIVEN** `image-generation` is switched off
- **WHEN** the runtime composition resolves
- **THEN** no `smith/image-generation` module is recorded
- **AND** `generate_image` is not registered

#### Scenario: First-party module is recorded

- **GIVEN** `budget-notice` is mounted
- **WHEN** the runtime composition resolves
- **THEN** a `smith/budget-notice` module is recorded with built-in
  provenance
- **AND** its record lists its pipeline components and its status item

## MODIFIED Requirements

### Requirement: Native registration is trusted embedding only

In-process Rust provider, tool, and host-service registration SHALL be
classified as a trusted native embedding tier and MUST NOT be described or
configured as sandboxed user plugin execution. A module crate that a build
adds to the compiled-in module list SHALL be trusted native, and when it is
not part of Smith's own source it MUST be recorded and listed with a
third-party provenance naming its crate. User-installed executable
extensions that are not compiled in MUST use a mediated execution tier (the
versioned capability-brokered subprocess protocol or a WASM component host);
Smith MUST NOT expose a public SDK for runtime-installed executable plugins
until such a mediation path is implemented and covered by conformance tests.

#### Scenario: Embedder supplies an in-process tool

- **GIVEN** a trusted host supplies an `Arc<dyn Tool>` during harness resolution
- **WHEN** Smith composes the runtime
- **THEN** composition evidence labels the module trusted native
- **AND** Smith makes no claim that syscalls from that code are mediated

#### Scenario: User manifest requests native loading

- **GIVEN** a user-installed extension manifest requests an in-process native
  library
- **WHEN** Smith resolves the module
- **THEN** Smith rejects the unsupported execution tier
- **AND** directs the extension to a mediated execution tier

#### Scenario: Declarative panel is rendered

- **GIVEN** an external extension contributes bounded declarative panel data
- **WHEN** a client renders it
- **THEN** the extension receives no renderer memory or runtime handle
- **AND** presentation failure cannot mutate canonical session state

#### Scenario: Third-party crate is compiled in

- **GIVEN** a build adds a module crate that is not part of Smith's source
  to the compiled-in module list
- **WHEN** Smith composes the runtime with that module mounted
- **THEN** composition evidence labels it trusted native with third-party
  provenance naming the crate
- **AND** every module listing marks it as third-party
- **AND** Smith makes no claim that its code is mediated
