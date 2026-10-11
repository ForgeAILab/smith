# WASM modules

WASM modules are executable components installed inside a [plugin bundle](plugins.md).
They mount through the same [module contract](modules.md) as compiled Rust modules,
and appear as **sandboxed**, with plugin provenance. Default builds enable the
`wasm-modules` feature. Feature-free builds list installed components as **not
supported in this build** and never instantiate them.

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
updates and installation locations: `wasm.<plugin-name hash>.<relative-path hash>`.
The actual id is shown in listings; use that exact id for switches and restrictions.
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
| `session_history` | `session-history` for the current session, and history supplied to context contributors |
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
[plugins.my-plugin.modules."wasm.PLUGIN_HASH.PATH_HASH"]
network = false
workspace_write = false
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
values. User-config limit overrides are not offered in this version.

A trap, timeout, resource exhaustion or invalid result fails that invocation,
records a diagnostic, and leaves the session running. A failed context contributor
adds no patch; a failed output processor preserves the original output. Three
consecutive invocation failures disable the module for the session. Successful
calls reset the consecutive counter but do not reenable a disabled module.
Interactive rebuilds retain session failure counters. Guest instances are fresh
on each mount. Calls into one instance are serialized. The host refreshes status
after mount and tool/pipeline calls; rendering reads the cache without entering
WASM. Failed loading, including an unknown interface version, leaves a failed row
and does not prevent session startup.

## Authoring

Start from [`examples/wasm-module-hello`](../examples/wasm-module-hello/README.md).
It is a complete plugin with a Rust cdylib, one tool, one status item, one workspace
read capability, a build script, lockfile, and built component. The guest owns a
separate Cargo workspace excluded from Smith's workspace.

Implement `smith_module_sdk::Module` (generated `bindings::Guest`) and call
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

The interface is `smith:module@0.1.0`, one `module` world. Current bindings accept
compatible 0.1 patch versions; unsupported major/minor packages report both the
found package and supported package. There is no previous package yet. When a
second interface is introduced, supporting its predecessor needs a separate
adapter. New contribution kinds or host imports require a new package version.
