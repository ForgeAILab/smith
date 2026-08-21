# Runtime architecture

Smith has one public composition flow:

```text
HarnessSpec -> resolve() -> ResolvedHarness -> factory::build() -> SmithRuntime
                                                        |
                                                        v
                                                   SmithSession
                                                        |
                                                        v
                                                   SmithEvent v1
```

`HarnessSpec` is declarative. Resolution validates module identity, revision,
provenance, trust tier, contribution collisions, requested capabilities, and
host-computed grants before provider construction. `ResolvedHarness` retains
immutable provider, authority, persistence, context, delegation, and module
evidence plus a bounded non-secret report. A contribution describes what a
module supplies; its capability request and grant separately describe what it
may do.

The factory is physically staged under `smith-runtime/src/factory/` as
`resolve`, `provider`, `authority`, `capabilities`, `persistence`,
`delegation`, and `compose`. These modules are private. Only
`factory::build(ResolvedHarness)` constructs Agent Runtime policy. A deprecated,
hidden protocol-v1 adapter resolves a trusted `RuntimeRequest` for existing
embedders and delegates to that same root; it is not a second composition path.

Agent Runtime owns canonical execution, persistence events, retries,
cancellation, provider mechanics, and the prepared-call executor. Smith owns
configuration provenance, trust, capability grants, workspace policy,
approval, module composition, and the client protocol. Canonical events stay
in the journal. `SmithEvent` is a deterministic, redaction-preserving
projection used by terminal, headless, replay, and future clients.

Smith client protocol v1 is the current and minimum supported revision. The
old public `SessionHandle` re-export is deprecated for one migration release;
new embedders use `SmithSession`, `SmithInput`, Smith receipts, and
`SmithEvent`. Unknown future payloads preserve their envelope and causal slot.
