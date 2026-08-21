## ADDED Requirements

### Requirement: Module contributions do not imply capability grants

Every resolved module SHALL record contributions separately from requested and
host-granted capabilities. Registering a tool, command, observer, skill, or UI
panel MUST NOT itself grant filesystem, process, network, credential, provider,
approval, or renderer authority, and executable adapters MUST receive only
broker handles covered by the resolved grant.

#### Scenario: Tool contribution requests undeclared network access

- **GIVEN** a module contributes a tool but requests no network capability
- **WHEN** the tool contribution is resolved
- **THEN** it receives no network broker handle
- **AND** its contribution declaration cannot widen the grant

#### Scenario: Content-only skill is activated

- **GIVEN** a module contributes a trusted skill body
- **WHEN** the skill activates
- **THEN** it contributes bounded instructions only
- **AND** it receives no executable module capability

### Requirement: Native registration is trusted embedding only

In-process Rust provider, tool, and host-service registration SHALL be
classified as a trusted native embedding tier and MUST NOT be described or
configured as sandboxed user plugin execution. User-installed executable
extensions MUST use the versioned capability-brokered subprocess protocol;
Smith MUST NOT expose a public user-installed plugin SDK until that mediation
path is implemented and covered by conformance tests.

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
- **AND** directs the extension to the subprocess protocol

#### Scenario: Declarative panel is rendered

- **GIVEN** an external extension contributes bounded declarative panel data
- **WHEN** a client renders it
- **THEN** the extension receives no renderer memory or runtime handle
- **AND** presentation failure cannot mutate canonical session state
