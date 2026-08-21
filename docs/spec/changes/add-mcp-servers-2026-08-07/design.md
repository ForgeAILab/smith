---
created_at: 2026-08-07T20:26:54Z
updated_at: 2026-08-20T22:14:36Z
---

# Design: MCP servers in Smith

The shared package's design document
(`../agent-runtime/docs/spec/changes/add-mcp-capability-source-2026-08-07/design.md`)
owns transport, schema translation, and remote-tool authority. This document
covers only the Smith-side decisions: declaration, trust, and presentation.

## Configuration shape

```toml
[mcp.servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "keychain:smith/github" }
enabled = true

[mcp.servers.internal]
url = "https://mcp.example.com/v1"
credential = "keychain:smith/internal"     # sent as `Authorization: Bearer …`
headers = { X-Tenant = "acme" }
```

A value in `env` or `headers` is either a credential *reference* — the same
`keychain:`/`authfile:`/`env:`/`file:` forms a provider's `credential` takes —
or a literal. References resolve through the existing secret path behind the
trust boundary. Literals are kept redaction-safe and never rendered, and are
refused outright where the name says the value is a credential and the file is
repository-controlled.

An option the chosen transport cannot use is refused rather than ignored: a
`credential` on a local command, or `args` on a remote endpoint, would otherwise
leave a user believing a server is authenticated when nothing is ever sent.

Declarations resolve through the existing layered chain and carry source
attribution, so `/config` can answer "where did this server come from?" the same
way it does for a model or a provider. Environment values referencing a
credential resolve through the existing secret path; raw secrets in project
configuration are rejected, not merely redacted.

## Decision: trust binds the resolved command, not the config file

An MCP stdio server is a command Smith spawns. `smith-config`'s `TrustStore`
already exists for exactly this class of authority, with `Executable::from_setting`
digesting a shell-valued setting's text.

The digest covers the **fully resolved** invocation — command, arguments, and
environment variable *names* — not the file that declared it. This matters
because:

- Two projects declaring an identical server should not each require separate
  approval of the same content, and a project that renames its config file
  should not re-prompt.
- Changing `args` from `server-github` to `server-github-evil` **must**
  invalidate the decision, which a file-level digest of a large config would
  also do, but noisily — every unrelated config edit would re-prompt.

Environment *values* are excluded from the digest because they resolve to
secrets; including them would make the digest secret-derived and would re-prompt
on every credential rotation. Names are included because adding a new variable
changes what the server can see.

A new `ExecutableKind::McpServer` variant is added rather than reusing
`ShellSetting`, so the prompt can say "MCP server `github`" and list the command
rather than presenting an opaque setting key.

A remote server is digested the same way, over what it sends rather than what it
runs: the endpoint plus the *names* of the headers, including the
`Authorization` header a declared `credential` is sent under. Moving the
endpoint or gaining a header re-prompts; rotating the token does not.

## Decision: project declarations cannot self-authorize

The existing rule — repository configuration cannot self-authorize tools —
extends here with a sharper edge, because an MCP server is more dangerous than
an approval setting: it introduces *new tools* whose effects Smith cannot
inspect.

Two separate guards:

1. **Execution trust.** A project-declared server always requires confirmation,
   regardless of approval mode. `approval.mode = "allow-all"` from user
   configuration authorizes *tool calls*; it does not authorize *spawning a
   server*. These are different questions and the second is asked once per
   content hash.
2. **Preflight rejection.** Repository configuration that attempts to
   auto-approve MCP tools fails startup with the same diagnostic shape as the
   existing self-authorization check.

A user-scope declaration in `~/.smith/config.toml` still requires first-run
confirmation — the user wrote it, but the digest binding is what makes a later
silent modification detectable.

### Headless

`client-surfaces` already requires fail-closed headless approval. An untrusted
server in a headless run does not prompt and does not spawn: it contributes zero
tools and reports why. A run that needs it must be trusted interactively first,
or pass an explicit invocation policy.

## Decision: naming and display

Tools render as `mcp__<server>__<tool>`, matching the shared package's
model-facing form. A double underscore rather than a dot because Anthropic and
OpenAI both restrict tool names to `[a-zA-Z0-9_-]`. The existing fixture in
`smith-tui/src/render/tests/composer.rs` uses the single-segment
`mcp.some_third_party_tool`, which is both un-namespaced and un-sendable; it is
updated to the namespaced form.

`tool-call-display` already requires redaction-safe summaries. Remote tools
extend this: Smith cannot classify server-defined argument fields, so it cannot
know which are sensitive. Arguments are therefore hidden by default and the row
shows the tool name with an explicit "arguments hidden" marker — which is what
the existing fixture already asserts.

## Decision: unreviewed tools may mutate external state

MCP annotations are untrusted hints. The current shared package correctly
refuses to let `readOnlyHint` lower authority, but its default host floor is
still read plus network and it adds a remote write only when the server says
`destructiveHint = true`. Omission therefore underdeclares tools such as
`send_email`, `merge_pull_request`, or `delete_repository`.

The default for every unreviewed tool becomes:

```text
external read of service:<server>
+ possible external write of service:<server>
+ network to the resolved server endpoint
+ data egress to that endpoint
```

This is deliberately not `FsRead` or a workspace filesystem resource. External
service reads and writes are different authority domains and must not inherit
filesystem containment semantics. Possible writes for one server share a
server-scoped conflict key so they do not overlap local file writes.

A narrower classification is accepted only from a host-owned record:

```rust
struct McpToolPolicyKey {
    server_identity: ServerIdentity,
    tool_name: String,
    schema_revision: RegistryRevision,
}
```

Changing the server identity, name, schema, or relevant annotations invalidates
the review. Server-supplied annotations can still add higher-risk categories;
they cannot select or narrow the host record.

The pinned Agent Runtime currently has only local-looking `Read`/`Write`, bare
`Network`, and process effects; `DataEgress` exists as a permission but cannot
be produced by `ToolEffects`. The coordinated upstream change therefore owns
external read/write resources, endpoint-scoped network, data-egress effect
mapping, risk derivation, scheduling keys, and MCP binding tests. Smith owns the
default conservative policy, optional reviewed-policy injection, approval
presentation, and the compatible dependency pin.

## Decision: connection is not on the startup critical path

Servers connect concurrently with session start, not before it. A slow `npx`
download must not delay the first prompt. The status line shows servers still
connecting; their tools become available at the next safe activation boundary,
which the shared package's epoch handling already provides.

This means the first turn of a session may not see a slow server's tools. That
is the correct trade — the alternative is a session that hangs on a third party.

## Failure presentation

`/mcp` lists every configured server with state and, on failure, a reason:

| State        | Shown as                                        |
|--------------|-------------------------------------------------|
| Connected    | tool count                                      |
| Connecting   | elapsed time                                    |
| Untrusted    | "needs approval — run `/mcp trust <name>`"      |
| Failed       | bounded error, secrets redacted                 |
| Disabled     | `enabled = false`, with its source              |

Failures are sticky and visible rather than logged and forgotten — a server that
silently contributes nothing is worse than one that says why.

## Remaining authority work

At commit `19e1696`, `McpOptions` does not supply an effect policy and
`McpServerConfig::new` therefore retains the shared read/network floor. The
pending Section 7 tasks must complete before this change is archived or called
policy-correct.
