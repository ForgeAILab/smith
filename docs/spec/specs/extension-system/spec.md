# extension-system Specification

## Purpose
The versioned extension protocol, its authoring host, contribution points, and the trust and least-privilege rules extensions run under.
## Requirements
### Requirement: Versioned subprocess extension protocol

Smith SHALL define a language-neutral, framed-JSON protocol over stdio for
extension initialization, version negotiation, permission declaration,
registration, events, requests, responses, cancellation, and shutdown.
Protocol payload size, response time, and in-flight request count MUST be
bounded.

#### Scenario: Compatible extension initializes

- **GIVEN** an extension and Smith share a compatible protocol version
- **WHEN** the extension completes initialization and declares capabilities
- **THEN** Smith applies approved grants
- **AND** activates its registered contributions in deterministic order

#### Scenario: Protocol versions are incompatible

- **GIVEN** an extension requires an unsupported protocol version
- **WHEN** initialization is negotiated
- **THEN** Smith rejects the extension with an actionable compatibility error
- **AND** continues without crashing the core runtime

### Requirement: Optional TypeScript authoring host

Smith SHALL provide a first-party TypeScript SDK and a separate optional Node
host. The host MUST support asynchronous extension factories, while the Rust
core MUST start and run normally when Node is absent and no TypeScript extension
is enabled.

#### Scenario: Async extension factory

- **GIVEN** a TypeScript extension performs asynchronous model discovery before
  registering a provider
- **WHEN** Smith starts the extension
- **THEN** startup awaits the factory within its configured deadline
- **AND** exposes the provider only after successful registration

#### Scenario: Node is absent

- **GIVEN** Node is not installed and no TypeScript extension is enabled
- **WHEN** Smith starts
- **THEN** all Rust-native features remain functional

#### Scenario: Enabled TypeScript extension lacks Node

- **GIVEN** a TypeScript extension is enabled but the configured host cannot
  start
- **WHEN** Smith loads extensions
- **THEN** it reports a clear dependency error naming that extension
- **AND** follows its configured required-or-optional failure policy

### Requirement: Pi-like initial contribution points

The initial extension API SHALL support tools and trusted tool replacement,
commands, keyboard shortcuts, lifecycle/provider/tool/session events,
permission gates, path protection, compaction/summarization policy, provider
registration, declarative status-line items/widgets, and MCP registration.
Executable contributions MUST adapt to Agent Runtime's shared provider, tool,
ability, registry, context, and event contracts rather than a Smith-local
parallel contract.

#### Scenario: Extension adds a tool and status item

- **GIVEN** a trusted extension registers `deploy` and a declarative deployment
  status item
- **WHEN** initialization completes
- **THEN** the agent may call the namespaced tool according to its permissions
- **AND** the TUI renders the status contribution without giving the extension
  direct renderer memory access

#### Scenario: Extension replaces a built-in

- **GIVEN** an extension requests replacement of a built-in tool
- **WHEN** replacement was not explicitly trusted and configured
- **THEN** Smith rejects the conflicting registration

### Requirement: Extension trust and least privilege

Smith MUST NOT start executable project extensions before hash-bound project
trust is confirmed. Each extension SHALL receive only approved declared
capabilities, and access to secrets, shell, filesystem writes, network, provider
registration, or permission-gate replacement MUST be separately representable.

#### Scenario: Extension requests new capability after update

- **GIVEN** a trusted extension update adds network access
- **WHEN** Smith computes the changed manifest/content hash
- **THEN** the prior trust grant is invalid
- **AND** the new capability is displayed before confirmation

### Requirement: Failure isolation and deterministic hooks

An extension crash, timeout, malformed message, or oversized payload MUST NOT
crash Smith or corrupt the canonical session. Hook order and timeout/failure
policy MUST be deterministic and visible.

#### Scenario: Event hook times out

- **GIVEN** an optional hook exceeds its configured deadline
- **WHEN** Smith processes the event
- **THEN** Smith terminates or disables the faulty extension according to
  policy
- **AND** records a diagnostic
- **AND** continues the session without partial hook mutation

### Requirement: Multiple extension tiers without unstable native loading

Smith SHALL support trusted compile-time Rust registration and MCP alongside
the subprocess protocol. It MUST NOT load arbitrary Rust dynamic libraries as a
stable extension ABI. A future WASM Component/WASI host MAY implement the same
versioned contribution contract without changing agent-loop semantics.

#### Scenario: Register an MCP tool server

- **GIVEN** a trusted extension manifest declares an MCP server
- **WHEN** its process and tool schemas initialize successfully
- **THEN** Smith registers the MCP tools through the shared ability/tool
  registry
- **AND** applies the same approval and attribution rules as built-ins

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
