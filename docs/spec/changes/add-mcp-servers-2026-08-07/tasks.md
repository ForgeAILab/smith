---
created_at: 2026-08-07T20:26:54Z
updated_at: 2026-08-08T04:00:00Z
completed_at:
---

## 0. Coordination

- [x] 0.1 Approve this proposal and the coordinated agent-runtime proposal
  (`../agent-runtime/docs/spec/changes/add-mcp-capability-source-2026-08-07/`).
- [x] 0.2 Do not begin Section 3 until agent-runtime slices 1 through 4 pass.
  They do: 50 tests in `agent-runtime-mcp`, 0 failures.
- [x] 0.3 Record the `agent-runtime-mcp` version this change depends on.
  `=0.1.0`, with the `stdio` and `http` transports. See "Dependency pin" below.

## 1. Configuration Surface

- [x] 1.1 Add the `[mcp.servers.<name>]` declaration type to `smith-config`
  covering command, args, env, url, and enabled.
- [x] 1.2 Resolve declarations through the existing layered chain with source
  attribution.
- [x] 1.3 Resolve credential-referencing environment values through the existing
  secret path.
- [x] 1.4 Reject literal secret values in repository-controlled layers without
  reproducing the value in the diagnostic.
- [x] 1.5 Add `server_declaration_reports_its_winning_layer`.
- [x] 1.6 Add `literal_secret_in_project_config_is_rejected`.
- [x] 1.7 Add validation tests: unknown transport, both `command` and `url`,
  neither, and a malformed server name.

## 2. Execution Trust

- [x] 2.1 Add `ExecutableKind::McpServer` and its stable string.
- [x] 2.2 Digest the resolved invocation — command, args, and environment
  variable *names*, excluding values.
- [x] 2.3 Wire the confirmation prompt to show server name, resolved command and
  arguments, and content identity.
- [x] 2.4 Add `changed_args_invalidate_the_trust_record`.
- [x] 2.5 Add `rotated_credential_value_does_not_re_prompt`.
- [x] 2.6 Add `new_environment_variable_name_invalidates_trust`.
- [x] 2.7 Add `allow_all_approval_does_not_spawn_an_untrusted_server`.
- [x] 2.8 Extend the self-authorization preflight to MCP tool auto-approval; add
  `project_auto_approving_a_remote_tool_fails_preflight`.
- [x] 2.9 Add `headless_run_does_not_prompt_or_spawn_an_untrusted_server`.

## 3. Composition and Registration

- [x] 3.1 Add the `agent-runtime-mcp` dependency. *(Started at default features;
  the `http` transport was added in Section 6.)*
- [x] 3.2 Connect trusted servers concurrently with session start; do not block
  the first prompt.
- [x] 3.3 Register connected servers' tools through the shared registry into the
  existing composition tool list.
- [x] 3.4 Namespace remote tools by server; ensure a remote tool never shadows a
  built-in.
- [x] 3.5 Retain failures as reportable state rather than logging and discarding.
- [x] 3.6 Add `two_servers_with_the_same_tool_name_both_register`.
- [x] 3.7 Add `remote_tool_does_not_shadow_a_built_in`.
- [x] 3.8 Add `slow_server_does_not_delay_the_first_prompt`.
- [x] 3.9 Add `failing_server_leaves_the_session_usable`.
- [x] 3.10 Add `remote_tool_call_requests_approval_and_attributes_its_server`.
- [x] 3.11 Make a remote tool retrievable by its server and tool name, so a
  registered tool is actually reachable under live ability routing.
  *(Added during implementation — see "Behavior added beyond the task text".)*

## 4. Surfaces

- [x] 4.1 Add the `/mcp` command listing servers with state, tool count, source,
  and bounded failure reason.
- [x] 4.2 Add trust granting from the command without restarting the session.
- [x] 4.3 Never display credential values; name them instead.
- [x] 4.4 Reflect connecting/failed servers in operational status; resolve to
  quiet once settled.
- [x] 4.5 Render rows as `mcp__<server>__<tool>` with arguments hidden and the
  withholding stated.
- [x] 4.6 Update the existing `mcp.some_third_party_tool` fixture in
  `smith-tui/src/render/tests/composer.rs` to the namespaced form.
- [x] 4.7 Add `two_servers_same_tool_name_render_distinguishably`.
- [x] 4.8 Add `resumed_remote_tool_row_matches_the_live_row`.
- [x] 4.9 Add `mcp_command_never_renders_a_credential_value`.

## 5. Release Gate

- [x] 5.1 `cargo test --workspace` and `cargo test --workspace --all-features`
  pass. 30 suites, 0 failures.
- [x] 5.2 `cargo clippy --all-targets -- -D warnings` passes.
- [x] 5.3 Verify a project with no `[mcp.servers]` spawns nothing and behaves
  identically to before —
  `a_project_declaring_no_server_gets_no_supervisor_and_no_trust_file`.
- [x] 5.4 Manually exercise one real stdio server end to end and record the
  result. See "Live server exercise" below.

## 6. Remote Servers and Startup Timing

Added after the first four slices, at the user's direction: the stdio slice
exercised the credential path a remote server needs, so the deferral no longer
bought anything.

- [x] 6.1 Enable the `http` transport; confirm no licence change is needed.
- [x] 6.2 Add `credential` and `headers` to the declaration, resolved through
  the existing secret path.
- [x] 6.3 Refuse an authorization-bearing header written as a literal, and any
  option the chosen transport cannot use.
- [x] 6.4 Bind a remote server's trust to its endpoint and header names,
  including the `Authorization` name a declared credential is sent under.
- [x] 6.5 Show a remote server's endpoint and header credentials in `/mcp` and
  in the confirmation, by name only.
- [x] 6.6 Wait a bounded grace for declared servers before an interactive
  start, so the common case never crosses the rebuild boundary.
- [x] 6.7 Add `a_remote_server_resolves_its_endpoint_credential_and_headers`,
  `an_authorization_header_written_in_plain_text_is_refused`,
  `an_option_the_chosen_transport_cannot_use_is_refused`,
  `a_remote_server_is_trusted_by_its_endpoint_and_header_names`,
  `a_remote_confirmation_names_the_endpoint_and_its_bearer_header`,
  `the_startup_grace_is_bounded_and_leaves_a_slow_server_connecting`, and
  `a_remote_server_this_build_can_reach_fails_loudly_rather_than_silently`.
- [ ] 6.8 Exercise a successful remote round trip against a real streamable
  HTTP server. Blocked on the coordinated change's own 6.3 — see "Remaining
  work" below.
- [x] 6.9 Move Smith's own transport to `reqwest 0.13` so the `http` MCP
  transport and the provider transport share one client. See "HTTP client
  unification" below.

## Dependency pin

`agent-runtime-mcp = "=0.1.0"`, with the `stdio` and `http` transports. The
manifest entry carries the same git URL and `rev` as the other `agent-runtime`
crates, but that revision does not yet contain the package: it resolves today
through the git-ignored `.cargo/config.toml` patch table the workspace already
uses for sibling development. The entry becomes self-sufficient when
agent-runtime publishes a revision containing `crates/agent-runtime-mcp` and the
workspace `rev` moves to it.

`cargo deny check` passes unchanged and the local `deny.toml` needed no edit:
every crate `http` adds is already covered by the licence allow list.

## HTTP client unification

`rmcp` builds against `reqwest 0.13`. Smith's transport was on `reqwest 0.12`,
so enabling `http` linked two client stacks into the binary — two `reqwest`s,
two `hyper`s, two `rustls` trees. The workspace `reqwest` requirement is now
`0.13`, and `cargo tree -i aws-lc-rs` shows a single `reqwest v0.13.4` with
`rmcp` and `smith-runtime` both above it.

Two things moved with the version:

- `rustls-tls` is spelled `rustls` in 0.13, and `form` became its own feature
  (`RequestBuilder::form`, which the xAI token exchange posts through, is no
  longer unconditional). The workspace feature list carries both.
- 0.13's `rustls` selects `rustls-platform-verifier` and `aws-lc-rs` where
  0.12's `rustls-tls` selected bundled `webpki-roots` and `ring`. **Root
  certificates now come from the operating system's trust store rather than a
  copy compiled into the binary.** On macOS that is the Keychain; on glibc Linux
  it is the system CA bundle, which the release targets all ship. The practical
  effect is that a host with a corporate root installed now works without a
  rebuild, and a host with no CA bundle at all — a scratch container — no longer
  does. `aws-lc-rs` was already in the graph by way of `rmcp`, so it is not new
  weight; `ring` and `webpki-roots` left it.

No source change was needed beyond the manifest: the 27 call sites the earlier
note projected turned out to be source-compatible, and only the two `.form(…)`
sites in `xai.rs` failed to compile, against a missing feature rather than a
changed API.

## Live server exercise

`crates/smith-runtime/tests/mcp_live.rs` dials a real stdio server named by
`SMITH_MCP_COMMAND`, and is `#[ignore]`d because the server is
developer-specific. Run against a locally installed CodeGraph server:

```text
SMITH_MCP_COMMAND=codegraph SMITH_MCP_ARGS='serve --mcp' \
  cargo test -p smith-runtime --test mcp_live -- --ignored --nocapture

state: Connected { tools: 10 }
tools: ["mcp__live__codegraph_search", "mcp__live__codegraph_context",
        "mcp__live__codegraph_callers", "mcp__live__codegraph_callees",
        "mcp__live__codegraph_impact", "mcp__live__codegraph_node",
        "mcp__live__codegraph_explore", "mcp__live__codegraph_status",
        "mcp__live__codegraph_files", "mcp__live__codegraph_trace"]
refused: []
```

## Deviations and Remaining Work

**Credential references use the existing scheme spelling.** `design.md` writes
an environment value as `"${credential:github}"`. That form names nothing that
exists; the *existing secret path* is `CredentialRef`, whose schemes are
`keychain:`, `authfile:`, `env:`, and `file:`. An MCP environment value is
therefore written the same way a provider's `credential` is —
`GITHUB_TOKEN = "keychain:smith/github"` — and resolves through the same
resolver behind the same trust boundary. One spelling for one concept beat a
second syntax that would have to be taught, documented, and kept in step.

**A literal is refused by variable name, not by value shape.** The requirement
is that a raw secret in repository-controlled configuration be rejected. Smith
cannot classify a server-defined value, so the rule matches the *name*
(`*TOKEN*`, `*SECRET*`, `*KEY*`, `*PASSWORD*`, `*CREDENTIAL*`, `*AUTH*`) —
exactly the reasoning already applied to `AUTH_HEADERS`. A literal under such a
name is refused outside owner-only user configuration; every other literal is
allowed but kept redaction-safe in memory and never rendered, because a server
defines its own fields and Smith cannot tell its settings from its secrets.

**Behavior added beyond the task text (3.11).** Registering a remote tool is not
enough to make it usable: Smith routes tools through descriptor retrieval, and
`mcp__docs__search` matches no keyword a user or model would ever produce. The
first `remote_tool_call_requests_approval_and_attributes_its_server` run
registered the tool, advertised it to nobody, and failed the call — the exact
"it registered fine" failure that would have shipped unnoticed. Remote tools now
contribute their server and tool names as retrieval keywords. Found by writing
the test for 3.10.

**Startup waits a bounded grace, then stops waiting.** An interactive start
gives declared servers 1.5 s before opening the prompt without them, so a local
server — which answers in milliseconds — contributes its tools on turn one and
the rebuild boundary below is never crossed. A slower server still cannot hold
the prompt: the grace ends and its tools join later. A non-interactive run has
no later boundary, so it waits for the full startup timeout instead.

**Tools join at a session-rebuild boundary, not inside the live runtime.** Agent
Runtime seals its tool registry when a runtime is composed, so a server that
connects afterwards cannot push tools into the running one. The supervisor
therefore outlives the session: it holds the connections, and the interactive
loop rebuilds the session around the same identity at the next idle boundary
(`InteractiveExit::CapabilitiesChanged`) once a server has contributed
something new. Connections survive the rebuild, so the boundary costs a
recomposition and not a reconnection. A non-interactive run has no later
boundary, so it waits — bounded by the startup timeout — before composing.

**`Activated::McpConnection` is not produced.** The coordinated change deferred
its Section 5 to this one. Smith registers a remote tool through its own
`SmithToolAbility`, the same wrapper every built-in uses, rather than through
the binding's `AbilityKind::Mcp` descriptor. The shared descriptor declares a
dependency on `mcp:<server>`, which would require Smith to also register a
server-level ability whose activation yields `Activated::McpConnection` — a new
activation kind inside a sealed registry that live routing has invariants
about. The behavior the specs require (registration, approval, attribution,
namespacing, dependency on a live connection) holds either way, because a
server that is not connected contributes no tools at all. Reopening the shared
descriptor path is the natural next change, and the seam is unchanged.

**Remote HTTP servers connect, but no successful round trip is covered.** The
`http` transport is compiled in and a remote server resolves, authenticates,
takes its own trust decision, and reports a real dial failure
(`a_remote_server_this_build_can_reach_fails_loudly_rather_than_silently`
proves the transport is present rather than reported missing). What is still
untested anywhere is a *successful* streamable-HTTP exchange: the coordinated
package's conformance fixture speaks over an in-memory duplex, so exercising the
HTTP path needs a real local HTTP server, and that fixture belongs upstream
(their task 6.3, still open). Until it exists, remote support rests on the
transport's own upstream tests.

**The release build has not yet cross-compiled `aws-lc-sys`.** Enabling `http`
put it in the graph, and the unification left it as the only crypto provider
where `ring` used to be. It builds on the development host, but the
`aarch64-unknown-linux-gnu` release job has never compiled it. Reading its build
script, that job should already have what it needs: `aws-lc-sys` ships
pre-generated bindings for `aarch64-unknown-linux-gnu`, so it takes the `cc`
builder rather than the CMake one, and the `cc` path wants only the
`aarch64-linux-gnu-gcc` the workflow installs and exports. That is a reading of
the build script, not an observation — the first tagged release after this
change is what confirms it.
