# extension-system Specification

## Purpose
The versioned extension protocol, its authoring host, contribution points, and the trust and least-privilege rules extensions run under.
## Requirements
### Requirement: Pi-like initial contribution points

The extension API SHALL support tools, harness pipeline components,
event observers, commands, declarative status-line items, and MCP
registration. A sandboxed WASM module SHALL be limited to the contribution
kinds of the interface version it was built against. Provider registration,
permission-gate replacement, path protection, compaction policy, keyboard
shortcuts, and replacement of a built-in tool MUST be available only to
trusted compiled-in modules and MUST NOT be offered to a sandboxed module.
Executable contributions MUST adapt to Agent Runtime's shared provider,
tool, ability, registry, context, and event contracts rather than a
Smith-local parallel contract.

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

#### Scenario: Sandboxed module names a built-in tool

- **GIVEN** a WASM module contributes a tool with the name of a built-in
- **WHEN** the module is mounted
- **THEN** that contribution is refused and reported
- **AND** the built-in tool is unchanged

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
configured as sandboxed user plugin execution. A module crate that a build
adds to the compiled-in module list SHALL be trusted native, and when it is
not part of Smith's own source it MUST be recorded and listed with a
third-party provenance naming its crate. User-installed executable
extensions that are not compiled in MUST run in the sandboxed WASM component
host; Smith MUST NOT load user-installed native code.

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
- **AND** directs the extension to the WASM component host

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

### Requirement: Connected MCP servers contribute registry abilities

Smith SHALL register a trusted, connected server's advertised tools through the
shared ability and tool registry, applying the same approval, attribution, and
display rules as built-in tools. Remote tools MUST be namespaced by their
server so two servers advertising the same tool name coexist, and a remote tool
MUST NOT replace or shadow a built-in.

#### Scenario: Two servers advertise the same tool name

- **GIVEN** two trusted servers each advertising a tool named `search`
- **WHEN** their tools are registered
- **THEN** both are addressable under distinct server-namespaced names
- **AND** neither registration is rejected as a conflict

#### Scenario: Server advertises a built-in's name

- **GIVEN** a trusted server advertising a tool named `shell`
- **WHEN** its tools are registered
- **THEN** the built-in `shell` tool remains the one addressed by that name
- **AND** the remote tool is addressable only under its namespaced name

#### Scenario: Remote tool requires approval

- **GIVEN** an approval policy requiring confirmation for a remote tool's
  permission set
- **WHEN** the agent calls it
- **THEN** Smith requests approval before invocation
- **AND** the request attributes the call to its server

### Requirement: Conservative authority for unreviewed MCP tools

Smith SHALL treat every unreviewed MCP tool as an external service read,
possible external service write, endpoint-scoped network action, and data
egress regardless of missing or false server annotations. These effects MUST
NOT be represented as workspace filesystem access. A narrower effect set MAY be
used only through a host-owned policy bound to the exact server identity, tool
name, and schema revision; server or repository content MUST NOT create or
select such a narrowing.

#### Scenario: Server omits annotations on a mutating tool

- **GIVEN** a server advertises `delete_repository` without tool annotations
- **WHEN** Smith binds and prepares the unreviewed tool
- **THEN** the call requests possible external write, network, and data-egress
  authority before invocation
- **AND** it does not request workspace `FsRead` as a substitute

#### Scenario: Server claims a tool is read only

- **GIVEN** an unreviewed server advertises `send_email` with
  `readOnlyHint = true` and `destructiveHint = false`
- **WHEN** Smith derives the prepared authority
- **THEN** the host's possible-write and egress floor remains unchanged
- **AND** the server's hints do not suppress approval eligibility

#### Scenario: Reviewed schema changes

- **GIVEN** a host-owned narrow policy matches one server identity, tool name,
  and schema revision
- **WHEN** the server changes the tool schema or its identity changes
- **THEN** the narrow policy no longer matches
- **AND** Smith falls back to the conservative unreviewed authority floor

### Requirement: Server connection does not gate session start

Smith SHALL start a session without waiting for MCP servers to connect. A
server that is slow, unreachable, untrusted, or failing MUST NOT delay the first
prompt or fail the session, and its tools SHALL become available at a safe
activation boundary once it connects.

#### Scenario: Server is slow to start

- **GIVEN** a trusted server whose command takes a long time to become ready
- **WHEN** the user opens a session
- **THEN** the prompt is available before the server finishes connecting
- **AND** the server's tools appear at a later safe boundary

#### Scenario: Server never connects

- **GIVEN** a trusted server whose command exits immediately
- **WHEN** the user opens a session
- **THEN** the session is fully usable with the remaining capabilities
- **AND** the failure is retained and reportable rather than silently discarded

### Requirement: Untrusted servers are inert in non-interactive runs

In a non-interactive run Smith MUST NOT prompt for MCP execution trust and MUST
NOT spawn an untrusted server. The server SHALL contribute no tools and the run
MUST report that trust is required, consistent with fail-closed headless
approval.

#### Scenario: Headless run encounters an untrusted server

- **GIVEN** a declared server with no trust record
- **WHEN** Smith runs non-interactively
- **THEN** the server is not spawned and no prompt is shown
- **AND** the run reports that the server requires interactive trust

### Requirement: MCP image results reach the model

Smith SHALL deliver an image block in a connected MCP server's tool result to
the model as an image part of that tool result. It SHALL NOT replace the
image with a text description. The image part SHALL be bounded separately
from text output: at most 5 MiB of decoded bytes per image, and at most 8
images per result. An image over either bound, or in a media type other than
PNG, JPEG, GIF, or WebP, SHALL be replaced by a text line that gives its
media type, its size, and the reason it was dropped. Whether the image is
then included in a given request is decided by the request image projection
(see `context-working-set`).

#### Scenario: Screenshot from a computer-use server

- **GIVEN** a trusted MCP server whose `get_window_state` result holds a
  text block and a 900 KB PNG image block
- **AND** the main model accepts image input
- **WHEN** the agent calls that tool
- **THEN** the tool result holds the text and the PNG as an image part
- **AND** the next provider request includes that image in the tool result

#### Scenario: Vision model on a Chat Completions endpoint

- **GIVEN** a custom OpenAI-compatible provider whose model is marked
  `input_modalities = ["text", "image"]`
- **WHEN** an MCP tool result with a PNG is sent to it
- **THEN** the `tool` message carries the result's text
- **AND** a `user` message right after it carries the PNG, labelled with the
  tool call id
- **AND** the request is not rejected

#### Scenario: Image over the size bound

- **GIVEN** a server result with a 12 MiB PNG
- **WHEN** the result is translated
- **THEN** the tool result holds a text line naming `image/png`, the size,
  and that the image exceeded the per-image bound
- **AND** no image part is present

#### Scenario: Text bound does not apply to images

- **GIVEN** a tool output limit of 32 KiB
- **AND** a server result with a 2 MB PNG and 1 KB of text
- **WHEN** the result is bounded
- **THEN** the image part is kept unchanged

#### Scenario: Harness turn receives a bridged image

- **GIVEN** a `cli/claude-code/...` main session with the same server
  declared in Smith
- **WHEN** the CLI calls the server's tool through the Smith bridge
- **THEN** the bridge returns the image to the CLI as an MCP image block

### Requirement: Smith-declared MCP servers reach a harness only through Smith

Smith SHALL offer a harness session the tools of each trusted, connected MCP
server declared in Smith's configuration or contributed by an active plugin
through the bridge, and SHALL make each call itself on the CLI's behalf
under the server's existing trust, credential, approval, attribution, and
conservative-authority rules. Tools contributed by an active module SHALL be
offered the same way under that module's grants.

Smith MUST NOT write a Smith-declared server, its command, its URL, or its
credentials into the CLI's own MCP configuration or argument list. A server
that is untrusted, disconnected, or withheld MUST contribute nothing to the
bridge.

#### Scenario: A remote tool called from a harness

- **GIVEN** a trusted, connected server `docs` declared in Smith's
  configuration
- **WHEN** the CLI calls its `lookup` tool through the bridge
- **THEN** Smith makes the call with the server's configured credential
- **AND** the approval request attributes the call to `docs`

#### Scenario: The CLI's invocation is inspected

- **GIVEN** a Smith-declared server that authenticates with a stored secret
- **WHEN** the harness is invoked
- **THEN** the invocation names only the `smith` server
- **AND** neither the server's address nor its secret appears in it

#### Scenario: An untrusted server

- **GIVEN** a declared server whose trust is not recorded
- **WHEN** a harness turn runs
- **THEN** none of its tools are bridged

#### Scenario: A module tool

- **GIVEN** an active module contributing `generate_image`
- **WHEN** the CLI calls it through the bridge
- **THEN** the call runs under that module's granted capabilities

### Requirement: Interactive sessions name untrusted servers

In an interactive session Smith SHALL tell the user, once when the session
opens, which declared MCP servers are untrusted and how to trust each one.
The notice MUST NOT spawn the server, prompt for trust, or delay the first
prompt. Found by the 2026-10-14 live check: a harness turn searched the
registry, found nothing, and could only tell the user to check the server.

#### Scenario: Session opens with an untrusted project server

- **GIVEN** a project declares an MCP server `docs` with no trust record
- **WHEN** the user opens an interactive session
- **THEN** the transcript shows one notice naming `docs` and `/mcp trust docs`
- **AND** `docs` is not spawned until the user trusts it

#### Scenario: Every declared server is trusted

- **GIVEN** every declared MCP server has a trust record
- **WHEN** the user opens an interactive session
- **THEN** no untrusted-server notice is shown

### Requirement: Stdio servers do not outlive Smith

Every stdio MCP server Smith starts SHALL be stopped when the Smith process
that started it ends, however it ends, including SIGKILL. A server that is
busy and not reading its input MUST NOT keep running after Smith is gone.
Smith SHALL start each server under a watchdog that never loads
configuration; if no watchdog can be started, Smith starts the server
directly and warns once.

#### Scenario: Smith is SIGKILLed during an MCP call

- **GIVEN** a declared stdio MCP server is executing a long tool call
- **WHEN** the Smith process is killed with SIGKILL
- **THEN** the server process is gone within 5 seconds

#### Scenario: Normal shutdown still goes through Smith

- **GIVEN** a connected stdio MCP server
- **WHEN** Smith exits normally or closes the connection on reconfigure
- **THEN** the server stops under Smith's existing grace and kill bounds
- **AND** the watchdog exits with it

### Requirement: File commands are content-only contributions

Smith SHALL classify a file command as a content-only contribution that
requires no extension process and no protocol. A file command MUST contribute
prompt text only, MUST NOT receive or imply any filesystem, process, network,
credential, provider, or approval capability, and MUST be recorded with its
source layer in the same contribution records other modules use, so a later
executable extension that contributes a command appears in the same listing.

#### Scenario: File command is recorded without grants

- **GIVEN** a runnable user command
- **WHEN** the session's contributions are resolved
- **THEN** the command is recorded as a command contribution from the user
  layer
- **AND** the record carries no requested or granted capability

#### Scenario: No extension host is needed

- **GIVEN** Node is absent and no executable extension is configured
- **WHEN** the user invokes a file command
- **THEN** the command expands and runs normally

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

### Requirement: Installable plugin bundles in the Claude Code layout

Smith SHALL treat a directory in the Claude Code plugin layout as an
installable plugin: an optional `.claude-plugin/plugin.json` manifest plus
components in their standard locations. A `.smith-plugin/plugin.json`, when
present, SHALL overlay the Claude Code manifest: its keys replace the same
keys there, and Smith-specific keys MUST appear only in that file. Unknown
manifest keys MUST be ignored. A manifest path that is not `./`-relative, resolves
outside the plugin root, or does not exist MUST make that component fail to
load without failing the plugin.

#### Scenario: Existing Claude Code plugin is read unchanged

- **GIVEN** a directory with `.claude-plugin/plugin.json`, `skills/deploy/SKILL.md`,
  `commands/status.md`, and `.mcp.json`
- **WHEN** Smith inspects it as a plugin
- **THEN** its inventory lists one skill, one command, and the declared MCP
  servers under the manifest's name

#### Scenario: Plugin without a manifest

- **GIVEN** a directory with `skills/` and no manifest
- **WHEN** Smith inspects it as a plugin
- **THEN** it is accepted and named after its directory or marketplace entry

#### Scenario: Component path escapes the plugin root

- **GIVEN** a manifest whose `skills` entry is `./../other`
- **WHEN** Smith inspects the plugin
- **THEN** that entry is reported as escaping the plugin root and is not loaded
- **AND** the plugin's other components are still listed

### Requirement: Plugin components load through existing catalogs

An enabled, trusted plugin SHALL contribute its skills, file commands, and
MCP servers through the same resolvers that load user and project ones, and
Smith MUST NOT add a second loader for them. Plugin skills and commands MUST
be named `<plugin>:<name>` and plugin MCP servers `<plugin>:<server>`, so a
plugin entry never shadows and is never shadowed by an unprefixed name.
`${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}`, and `${CLAUDE_PROJECT_DIR}`
MUST resolve in MCP server command, arguments, environment, URL, and
headers, and in skill and command bodies.

#### Scenario: Plugin command runs

- **GIVEN** an enabled plugin `deploy-tools` with `commands/status.md`
- **WHEN** the user runs `/deploy-tools:status`
- **THEN** the expanded template is submitted as an ordinary user prompt

#### Scenario: Plugin skill does not shadow a user skill

- **GIVEN** a user skill `deploy` and a plugin skill `deploy-tools:deploy`
- **WHEN** the skill catalog resolves
- **THEN** both are present under their own names

#### Scenario: Plugin MCP server finds its own files

- **GIVEN** a plugin MCP server declared with
  `args = ["${CLAUDE_PLUGIN_ROOT}/server.js"]`
- **WHEN** Smith starts the server
- **THEN** the argument is the absolute path inside the installed plugin
- **AND** the process environment carries `CLAUDE_PLUGIN_ROOT` and
  `CLAUDE_PLUGIN_DATA`

### Requirement: Unsupported plugin components are listed, never run

Smith SHALL recognise plugin components it does not load (hooks, agents,
LSP servers, output styles, workflows, themes, monitors, channels, a `bin/`
directory, plugin settings, and MCP bundle files) and MUST list each with
the reason it is not loaded. Such a component MUST NOT prevent installation
and MUST NOT be executed. A component that references `${user_config.*}`
MUST be listed as needing configuration and MUST NOT be loaded.

#### Scenario: Plugin carries hooks

- **GIVEN** a plugin with `hooks/hooks.json`
- **WHEN** the user installs it
- **THEN** the confirmation and the plugin listing state that its hooks will
  not run
- **AND** no hook command is executed in any session

#### Scenario: MCP server needs user configuration

- **GIVEN** a plugin MCP server whose environment references
  `${user_config.api_token}`
- **WHEN** the plugin is enabled
- **THEN** that server is listed as needing configuration and is not started
- **AND** the plugin's skills and commands still load

### Requirement: Plugins are recorded as modules without authority

Each enabled plugin SHALL be recorded in composition evidence with
user-manifest provenance: its skills and commands as content-only
contributions and its MCP servers under the trusted-MCP tier. Installing or
enabling a plugin MUST NOT grant any capability; its MCP tools remain
subject to the existing conservative authority and approval rules.

#### Scenario: Disabled plugin leaves no record

- **GIVEN** an installed plugin that is switched off
- **WHEN** the runtime composition resolves
- **THEN** no module is recorded for it
- **AND** none of its skills, commands, or MCP servers are present

#### Scenario: Plugin MCP tool still needs approval

- **GIVEN** an enabled plugin whose MCP server advertises a mutating tool
- **WHEN** the model calls that tool
- **THEN** the call is subject to the same approval as any other MCP tool

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
pipeline contribution MUST contribute nothing for that phase. A context
contributor SHALL run without the session-history capability, and it then
receives an empty history. A view that Smith itself cannot fit within the
payload limits MUST skip that contribution and MUST NOT count as a failure
of the module.

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

#### Scenario: Contributor without history access

- **GIVEN** a WASM context contributor whose module was not granted session
  history
- **WHEN** the provider request is assembled
- **THEN** the contributor runs with an empty history
- **AND** its tools stay available

#### Scenario: History too large to pass

- **GIVEN** a granted WASM context contributor and a session history beyond
  the payload limits
- **WHEN** the provider request is assembled
- **THEN** the request is assembled without that contribution
- **AND** the module is not disabled

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

### Requirement: Sandboxed modules can call the session's model

Smith SHALL offer a sandboxed module a host function that performs one
bounded, non-streaming, tool-free request on the session's current model
route, including an installed agent when selected, only when the module's manifest requested the model capability
and the user granted it. The module MUST NOT receive a credential or choose a provider. Smith MUST
enforce the per-turn call limit and returned-payload bound, and MUST pass the
module's per-call output-token ceiling to provider executors.
The function MUST be available only while the module is handling a tool
call. A refused request MUST return a typed error to the module and MUST NOT
count towards disabling it.

Smith SHALL use the shared one-shot executor. Provider routes SHALL receive
the requested output-token ceiling; installed-agent routes use the CLI's own
generation limits and SHALL receive the role-labelled conversation rendered
as one user input. Both routes remain subject to the host deadline, call-count
and ABI payload limits. A truncated successful answer SHALL return finish
reason `length`; filtered, empty, tool-calling or unfinished answers SHALL
return a recoverable error. Installed-agent usage SHALL remain unpriced.

#### Scenario: Granted module asks the model

- **GIVEN** a WASM module granted the model capability
- **WHEN** its tool calls the model function with a prompt
- **THEN** the request is sent on the session's current provider and model
- **AND** the module receives the response text and usage
- **AND** no credential is visible to the module

#### Scenario: Ungranted module asks the model

- **GIVEN** a WASM module without the model capability
- **WHEN** its tool calls the model function
- **THEN** the function returns an error naming the missing capability
- **AND** no provider request is made

#### Scenario: Per-turn limit reached

- **GIVEN** a WASM module that has made the permitted number of model calls
  in the current turn
- **WHEN** it calls the model function again
- **THEN** the call is refused with a limit error
- **AND** the module is not disabled

#### Scenario: Model call outside a tool call

- **GIVEN** a WASM module granted the model capability
- **WHEN** its context contributor calls the model function
- **THEN** the call is refused

### Requirement: Sandboxed modules can notify a session

Smith SHALL let a sandboxed module contribute a watcher that Smith polls at
a host-owned bounded interval while a root session is live, only when the
module was granted the session-notify capability. Notices returned by a
watcher SHALL be delivered through the session inbox under the same rules as
monitor notifications: shown to the user without a prompt, supplied to the
model at the next safe boundary, and never interrupting a stream or tool
result. Notices MUST be bounded in count and size, MUST be labelled with the
plugin that produced them, and MUST be presented to the model as untrusted
text.

#### Scenario: Notice reaches an idle session

- **GIVEN** an idle interactive session with a granted watcher
- **WHEN** a poll returns one notice
- **THEN** the notice is shown with its plugin name
- **AND** the session takes a turn with the notice supplied to the model

#### Scenario: Notice arrives during a stream

- **GIVEN** the agent is streaming a response
- **WHEN** a poll returns a notice
- **THEN** the stream is not interrupted
- **AND** the notice is supplied at the next safe boundary

#### Scenario: Watcher without the capability

- **GIVEN** a WASM module that contributes a watcher without the
  session-notify capability
- **WHEN** the module is mounted
- **THEN** the watcher is refused with a reason
- **AND** the module's other contributions remain available

#### Scenario: Headless run ends

- **GIVEN** a headless run with a granted watcher
- **WHEN** its turn completes
- **THEN** the process exits without waiting for further polls

### Requirement: Interface versions coexist

Smith SHALL load modules built against interface 0.1 and interface 0.2. A
0.1 module MUST behave as it did before and MUST NOT be able to reach
functions introduced in 0.2.

#### Scenario: Older module still loads

- **GIVEN** an installed module built against interface 0.1
- **WHEN** a Smith that supports 0.2 starts a session
- **THEN** the module mounts and its tools work as before

### Requirement: MCP server stderr never reaches the terminal

Smith SHALL capture the stderr of every stdio MCP server it starts, including
the watchdog's own messages and anything a descendant of the server writes,
and MUST NOT let any of it reach Smith's terminal in an interactive or a
headless run. Smith SHALL drain that output continuously so a server never
blocks on a full pipe, and SHALL keep only a bounded tail per server with
terminal escape sequences removed and every environment value it passed to
the server redacted. Smith SHALL write that output to the session log. When
a server fails to start or exits on its own, Smith SHALL report the failure
with the last lines of that tail where the user already sees MCP status, and
normal output from a healthy server MUST NOT produce any notice. Found by the
owner on 2026-10-16: `npm notice` lines from `npx` were painted over the
composer and could not be cleared.

#### Scenario: A healthy server writes to stderr

- **GIVEN** a trusted stdio MCP server that writes `npm notice` lines to
  stderr and then connects
- **WHEN** an interactive session is open
- **THEN** nothing the server wrote appears on the terminal
- **AND** the server is listed as connected with no stderr detail
- **AND** the lines are written to the session log at `info` level

#### Scenario: A server fails to start

- **GIVEN** a trusted stdio MCP server that writes
  `cannot find module` to stderr and exits with status 1
- **WHEN** Smith tries to connect it
- **THEN** `/mcp` lists the server as failed
- **AND** the failure includes `cannot find module`

#### Scenario: A headless run with a failing server

- **GIVEN** the same failing server
- **WHEN** `smith -p` runs with `--output-format json`
- **THEN** Smith's stdout carries only the JSON envelope and its stderr does
  not contain the server's output
- **AND** the result's `diagnostics` name the server and include its last
  stderr line

#### Scenario: A connected server exits

- **GIVEN** a connected stdio MCP server
- **WHEN** the server exits on its own with status 3 after writing an error
  to stderr
- **THEN** the server is reported as failed with its exit status and that
  error
- **AND** its tools are withdrawn at the next safe boundary

#### Scenario: A server exits while a descendant holds its output

- **GIVEN** a connected stdio MCP server that started a background process
  sharing its stdout and stderr
- **WHEN** the server exits with status 4
- **THEN** the server is reported as failed with that status within about two
  seconds

#### Scenario: Output before the session log exists

- **GIVEN** `SMITH_LOG=info` and a server that writes to stderr while Smith is
  still starting the session
- **WHEN** the session's log file is created
- **THEN** those lines are in it

#### Scenario: Smith stops a server

- **GIVEN** a connected stdio MCP server
- **WHEN** Smith closes the connection on reconfigure or exit
- **THEN** the server is not reported as failed

#### Scenario: A server prints its own credential

- **GIVEN** a server started with `API_TOKEN` resolved from a credential
- **WHEN** the server writes that token's value to stderr
- **THEN** the session log and the reported failure show it redacted

### Requirement: Readable WASM module ids

A WASM module's id SHALL be its plugin name, `/`, and its component path
without the `.wasm` suffix, when that value fits the module id alphabet and
length. Otherwise it SHALL be `wasm.` followed by hashes of the plugin name
and component path. Both forms SHALL be stable across installs. The hashed
form SHALL remain an alias for a module that has a readable id, so install
records, capability grants, and configuration written before this change
keep applying without reinstalling. Listings and diagnostics SHALL show the
readable id.

#### Scenario: Readable id

- **GIVEN** plugin `relay` with component `modules/mailbox.wasm`
- **WHEN** the plugin is installed
- **THEN** the module id is `relay/modules/mailbox`

#### Scenario: Unrepresentable name

- **GIVEN** a plugin or component path with characters outside the module
  id alphabet
- **WHEN** the plugin is installed
- **THEN** the module id is the hashed form

#### Scenario: Install from before the change

- **GIVEN** a module installed and granted capabilities under its hashed id
- **WHEN** Smith starts after upgrading
- **THEN** the module loads with the same grants under its readable id
- **AND** a configuration entry naming the hashed id still applies to it
