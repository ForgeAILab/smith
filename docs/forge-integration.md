# Forge integration boundary

Forge should integrate at Smith's product boundary, not Agent Runtime's
concrete event enum or `SessionHandle`.

1. Build a declarative `HarnessSpec` from Forge-owned configuration and module
   provenance.
2. Resolve it once and retain `HarnessIdentity`, module/grant evidence, and the
   bounded resolution report.
3. Pass the immutable `ResolvedHarness` to the single Smith factory.
4. Drive work through `SmithSession` and consume `SmithEvent` protocol v1.

Forge-owned trusted Rust may use the trusted-native embedding API, with the
explicit understanding that this code already has ambient host authority.
User-installed executables must use the future capability-brokered subprocess
protocol. MCP remains its own external-service tier, and skills remain
content-only. UI panels receive declarative data rather than runtime handles or
renderer memory.

This boundary lets Agent Runtime change internal event or session APIs while
Smith maintains a revisioned client protocol for TUI, headless, GPUI, Forge,
and other embedders.
