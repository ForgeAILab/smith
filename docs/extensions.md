# Extension boundaries

Smith does not treat every extension mechanism as the same trust boundary.

| Extension | Execution | Trust and authority |
| --- | --- | --- |
| Smith built-in | In-process Rust | Product-trusted; prepared calls still pass central authorization |
| Trusted native embedding | In-process `Arc<dyn Tool>` | Full ambient same-user process authority; explicitly not sandboxed |
| MCP server | Separate process or remote HTTP service | Explicit executable trust plus conservative per-tool external read/write, endpoint network, and egress authority |
| User-installed executable plugin | Capability-brokered subprocess | Not available yet; executable declarations fail closed until the broker is running |
| Skill | Parsed bounded content | No executable or approval authority |
| Declarative panel | Bounded client data | No renderer memory or runtime handle |

Native dynamic-library loading is unsupported. A user manifest cannot select
trusted-native execution. MCP annotations are untrusted hints: missing,
`readOnlyHint = true`, or `destructiveHint = false` never lowers the
unreviewed floor. A narrower classification must be a host-owned record bound
to server identity, exact tool name, and schema revision. Honest destructive
hints may raise risk but cannot replace the host floor.

A declared MCP server starts only after its executable is trusted for the
project, and until then it contributes no tools. When an interactive session
opens or resumes, the transcript names each untrusted server once and points
to `/mcp trust <name>`; a plugin's servers are trusted by confirming the
plugin. A headless run leaves them stopped and reports the required trust in
the result's `diagnostics`, or on stderr for text output.

On Unix, Smith starts declared and plugin stdio MCP servers under a small
watchdog that never loads configuration. Servers inherit Smith's process
group and communicate over the same stdin and stdout pipes. On normal exit
Smith sends SIGTERM to the watchdog and allows 500 ms before bounded cleanup.
The watchdog stops its server with SIGTERM followed by SIGKILL after 300 ms
if needed, and reaps it. It also watches its parent PID, so even a busy server stops
if Smith is SIGKILLed. Closing a connection on reconfigure still gives the
server up to 3 s to exit before forced cleanup, while connections held by an
in-flight call remain usable. Embeddings without an injected Smith executable
start servers directly and warn once; those servers may outlive their host.

The reserved process protocol maps tool, skill, command, redaction-safe
observer, and declarative-panel contributions into bounded messages. It must
broker cancellation, lifecycle, filesystem handles, endpoint-scoped network,
secret use, and approval requests. Smith exposes no public executable-plugin
surface until that implementation exists; declarations alone grant nothing.
