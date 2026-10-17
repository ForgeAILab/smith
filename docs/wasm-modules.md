# WASM modules

WASM modules are executable components installed inside a [plugin bundle](plugins.md).
They mount through the same [module contract](modules.md) as compiled Rust modules,
and appear as **sandboxed**, with plugin provenance. Default builds enable the
`wasm-modules` feature. Feature-free builds list installed components as **not
supported in this build** and never instantiate them.

Smith loads both interface 0.1 and 0.2 components. Interface 0.2 adds the
`complete` host function and a watcher contribution with `poll`. Version 0.1
components keep their original host interface and behavior.

Version 0.1 offers four contribution kinds: tools, context contributors,
tool-output processors, and cached status items. It provides no provider
registration, approval replacement, commands, observers, arbitrary pipeline
phases, or TUI rendering beyond status items. Built-in tools cannot be replaced.

There is no WASI, ambient filesystem, network, environment, or wall clock.
Every service call is mediated by the host. Discovery (`describe`) has no capability
grants; only bounded logging is available before mount. Tool effects supplied by a guest do
not classify its authority: the runtime uses the conservative unreviewed MCP
policy and the ordinary configured approval path. Install approval grants host
services; it does not auto-approve tool calls.

## Packaging and capabilities

Declare components only in `.smith-plugin/plugin.json`:

```json
{
  "name": "my-plugin",
  "modules": [{
    "wasm": "./module.wasm",
    "capabilities": {
      "workspace_read": true,
      "network": ["api.example.com"],
      "data_egress": ["api.example.com"]
    }
  }]
}
```

Paths must start with `./`, exist, and remain inside the plugin directory after
canonicalization, like other plugin components. A module id is stable across
updates and installation locations: the plugin name, `/`, and the component path
without `.wasm`, such as `relay/modules/mailbox`. An id outside lowercase ASCII
letters, digits, `-`, `_`, `.` and `/`, or over 96 bytes, falls back to
`wasm.<plugin-name hash>.<relative-path hash>`. Case is never folded.
The old hashed id remains an alias: existing installs retain their confirmed
grants without reinstalling, and switches and capability restrictions using the
hashed id still apply. Listings and diagnostics show the readable id when one is
available. If capability restrictions or stored grants contain both spellings,
the readable entry takes precedence. Conflicting switch spellings in one
configuration layer are rejected; across layers, ordinary precedence applies.
WASM plugin names must use ASCII letters, digits, `.`, `_` or `-`, at most
128 bytes, so generated tool names stay within the host interface bounds.
Unknown capability or declaration keys refuse that module with a reason;
other valid content in the bundle remains available.

| Manifest capability | Host functions and scope |
| --- | --- |
| `workspace_read` | `fs-read` with root `workspace` |
| `workspace_write` | `fs-write` with root `workspace` |
| `plugin_data_read` | `fs-read` with root `plugin-data`, and `kv-get` |
| `plugin_data_write` | `fs-write` with root `plugin-data`, and `kv-set` |
| `model` | Interface 0.2 `complete`: send text to the session's active model provider, with host-owned credentials; no `data_egress` grant needed |
| `session_notify` | Session notification capability (interface 0.2) |
| `session_history` | `session-history` for the current session, and history supplied to context contributors (empty without it) |
| `network` | `http` to exact named hosts over HTTPS, or an explicitly named HTTP origin |
| `data_egress` | Additional destination grant required by `http`, including requests without a body |

Boolean capabilities default to false; endpoint lists default to empty.
`log` is always available, with message content withheld from host diagnostics.
Process, credentials, provider authority and approval authority cannot be granted.
Files are confined to their root; symlinks and special files are refused. Workspace
writes cannot reach Smith's private state/cache directories. Plugin data remains
outside the executable component cache. KV values are at most 64 KiB, with at most
256 entries in a bounded store.

Network grants are lowercase exact hosts, with no wildcard or subdomain matching.
For local services use an explicit origin, such as `http://127.0.0.1:8080`.
An HTTP origin grants that exact scheme, host and port. The service supports GET,
HEAD, POST, PUT, PATCH, DELETE and OPTIONS through Smith's existing HTTP client,
sharing its TLS verification, proxy discovery and timeouts. All redirects fail,
even within the granted host. Guest headers never obtain provider credentials.
Request and response bodies are bounded to 1 MiB and also subject to the complete
ABI payload limit. Headers are bounded to 64 entries, with names at most 128 bytes
and values at most 4096 bytes. Transport errors never echo request secrets.

`smith plugin add` shows each module and every requested capability before the
bundle digest and confirmation. Component bytes and manifest requests participate
in the whole-directory digest. Confirmed grants are stored with the installed
record, and run only under the matching plugin trust digest. Preparing an update
leaves the installed version running. A changed request shows old, new and newly
requested capabilities; replacement content cannot run before confirmation.

## User restrictions and switching

Only the user file may narrow capabilities, by the id shown in `/modules`:

```toml
[plugins.my-plugin.modules."my-plugin/module"]
network = false
workspace_write = false
model = false
session_notify = false
model_max_output_tokens = 1024
model_max_calls_per_turn = 2
```

Endpoint lists retain only their intersection with confirmed grants. `false`
withholds a capability; `true` retains confirmed authority and creates none.
Project/repository files and profile tables cannot change grants: those settings
are ignored and reported in `/plugins`. A withheld capability is shown as
**withheld by the user**. The manifest's requested authority remains visible at
install/update review.

`/modules`, `smith config modules`, `/plugins` and `smith plugin list` show sandboxed
provenance and capability scopes. `/modules <id> on|off` uses the existing reviewed
user-config switch and safe-boundary rebuild. Disabling the plugin removes its
modules together with its content. `/modules` and `/plugins` in a session also
show live module failures.

## Limits and failures

Default limits are 64 MiB of linear memory per instance, 5 seconds for a tool,
500 milliseconds for describe/mount/pipeline calls, and 5 milliseconds for status.
Arguments and results have a 1 MiB aggregate ceiling, with additional WIT limits
on strings, lists, identifiers, JSON schemas and nesting. Oversized values fail;
they are never silently truncated. Status labels have at most 80 Unicode scalar
values. Other execution limit overrides are not offered in this version.

Interface 0.2 `complete` performs one tool-free completion on the session's current
model route, including its installed CLI agent when selected. It accepts optional
system text, at most 64 text messages,
and a positive output token ceiling. The default ceilings are 4096 output tokens
per call and 4 calls per turn per module. Input must also fit the model's input
limit, conservatively budgeted as one token per UTF-8 byte plus message framing.
Only the global user plugin table may
lower `model_max_output_tokens` or `model_max_calls_per_turn`; zero disables model
calls. Values above the defaults, project settings and profile settings are ignored
and reported. An over-limit request returns `limit-reached`, never a silent clamp.

Both routes use Agent Runtime's shared one-shot API. Provider routes receive the
module's output-token bound; installed agents control their own generation limits.
For an installed agent, Smith renders the role-labelled conversation as one user
input and withholds its own tools. CLI usage is recorded with no catalog price.
All routes retain the call-count, deadline and payload limits. A successful
truncated answer returns `length`; filtered, empty, tool-calling or unfinished
answers return a recoverable error.

Only tool calls may use `complete`. Describe, mount, status, poll, context
contributors and tool-output processors receive `not-available-in-phase`. Model
calls have a separate 120 second deadline; time inside the host call does not
consume the guest's 5 second tool deadline. Model refusals and provider errors are
recoverable host errors and do not count toward disabling the module. Each provider
attempt is attributed to the installed module id and included in session usage and
cost, including provider-reported usage before a failed or interrupted response.

A trap, timeout, resource exhaustion or invalid result fails that invocation,
records a diagnostic, and leaves the session running. A failed context contributor
adds no patch. A context contributor without the session-history grant still
runs with empty history and keeps its tools available. If the host cannot fit a
contributor view within the payload limits, it skips that contribution for the
request without counting a module failure. A failed output processor preserves
the original output. Three consecutive invocation failures disable the module
for the session. Successful calls reset the consecutive counter but do not reenable a disabled module.
Interactive rebuilds retain session failure counters. Guest instances are fresh
on each mount. Calls into one instance are serialized. The host refreshes status
after mount and tool/pipeline calls; rendering reads the cache without entering
WASM. Failed loading, including an unknown interface version, leaves a failed row
and does not prevent session startup.

## Watcher notifications

Interface 0.2 modules can register a `watcher` ID and implement `poll(id)`.
The installed module must have the confirmed `session_notify` grant. Without
it Smith refuses the watcher at mount and shows the reason in `/modules`;
the module's other contributions remain available. Setting
`session_notify = false` in the user file prevents polling and lists the
capability as withheld by the user.

The root session's host owns the schedule. The first poll occurs after five
seconds, with at most one poll every five seconds per module and a 500 ms
deadline. Multiple watcher IDs share that budget in mount order. Polls use the
same instance lock as tools and pipeline calls; a busy instance skips the tick
instead of queuing work. Polls do not add status-refresh calls to their deadline.
Each poll returns at most eight notices, each at most
4096 UTF-8 bytes. A module cannot call `complete` from `poll`.

Notices are batched over the monitor policy's 200 ms window. Its flood limits
are 1,000 events or 1 MiB in a rolling ten-second window per module; exceeding
either stops that module's watcher schedule. The bounded session inbox may
also refuse delivery when full, stopping the source rather than building an
unbounded queue. The TUI displays notices with the installed plugin name;
headless stream JSON emits `module_notification` envelopes, separate from
canonical runtime sequence numbers. Other headless formats show notifications
on stderr. Credential redaction applies to displayed notice text.

The model receives notices as user-role inbox content, explicitly framed as
untrusted plugin data with JSON preserving notice boundaries. Delivery waits
for the next safe provider/tool boundary and never interrupts a stream or
tool result. An idle interactive session takes an ordinary, accounted turn,
including in approval mode `ask`; that turn may request approval as usual.

Child sessions never poll. Headless polling stops when the turn ends, before
any background-exit wait; watchers cannot keep the process alive. Shutdown or
a composition rebuild cancels the old schedule, and the new composition gets
one fresh schedule. A disabled module is not polled. The first failed,
oversized or timed-out poll stops that module's schedule and exposes a host
diagnostic; it does not retry a poisoned instance or fail the session. Poll
failures still count toward the module's ordinary three-failure disable rule.

## Authoring

Start from [`examples/wasm-module-hello`](../examples/wasm-module-hello/README.md).
[`examples/wasm-module-relay`](../examples/wasm-module-relay/README.md) is a larger
one: four tools, a context contributor, a watcher, a status item, and plugin data
as storage. [`examples/wasm-module-second-opinion`](../examples/wasm-module-second-opinion/README.md)
combines `complete` and `session_history` for a review by the current model.
The hello example is a complete plugin with a Rust cdylib, one tool, one status item, one workspace
read capability, a build script, lockfile, and built component. The guest owns a
separate Cargo workspace excluded from Smith's workspace.

Implement `smith_module_sdk::Module` and call
`smith_module_sdk::export!(YourModule)`. SDK tool/status/context builders help
construct typed declarations. The export wrapper retains a versioned Smith import.
Unused exports still exist and return an unknown-id error. Build the core module
for `wasm32-unknown-unknown`, then componentise it with the same `wit-component`
encoder used by `scripts/build-wasm-fixtures.sh`; do not link a WASI target.

```sh
rustup target add wasm32-unknown-unknown --toolchain stable
bash examples/wasm-module-hello/build.sh
smith plugin add ./examples/wasm-module-hello
```

The supported interfaces are `smith:module@0.1.0` and `smith:module@0.2.0`,
each with one `module` world. Older 0.1 components load through a compatibility
adapter and cannot reach the 0.2 functions. Unsupported package versions report
both the found version and the supported versions.
