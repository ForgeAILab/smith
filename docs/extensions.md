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

The reserved process protocol maps tool, skill, command, redaction-safe
observer, and declarative-panel contributions into bounded messages. It must
broker cancellation, lifecycle, filesystem handles, endpoint-scoped network,
secret use, and approval requests. Smith exposes no public executable-plugin
surface until that implementation exists; declarations alone grant nothing.
