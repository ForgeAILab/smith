---
created_at: 2026-08-20T22:13:02Z
updated_at: 2026-08-20T22:22:11Z
---

## Why

Smith has one sound runtime composition path, but its public embedding surface
still exposes Agent Runtime handles and accepts arbitrary in-process tools
without identifying them as trusted native code. This is a dependency routing
boundary, not yet the stable product, module, or untrusted-plugin boundary
needed by future GPUI, Forge, and user-installed extensions.

## What Changes

- Add a typed `HarnessSpec -> Resolver -> ResolvedHarness -> factory` pipeline.
  The resolved value is immutable, provenance-bearing, and is the only input
  accepted by the public Smith composition root.
- Model modules explicitly with identity, revision, provenance, trust,
  contributions, requested capabilities, and granted capabilities.
  Contributions describe what a module provides; grants independently bound
  what executable code may do.
- Add Smith-owned `SmithSession`, command receipts, IDs, and `SmithEvent`
  projection types for presentation clients. Agent Runtime remains the sole
  execution mechanism and canonical persisted event source behind the adapter.
- **BREAKING** rename unrestricted `RuntimeRequest.tools` injection to an
  explicitly trusted-native embedding contribution and remove it from the
  future untrusted plugin API.
- Classify extension tiers in types and documentation: built-in/trusted native,
  out-of-process extension, MCP, content-only skill, and declarative UI
  contribution. Arbitrary native dynamic libraries remain unsupported.
- Split the oversized factory implementation into private resolution,
  provider, authority, capability, persistence, delegation, and assembly
  stages while retaining one public build entry point.

## Impact

- Affected specs: `runtime-integration`, `client-surfaces`, `extension-system`
- Affected code: `smith-runtime` factory/host/public exports/abilities,
  `smith-cli` runtime host and headless projection, `smith-tui` reducer input,
  Smith embedding tests, extension documentation, and future Forge adapters
- Compatibility: direct embedders receive a migration adapter for one
  deprecation window. TUI and headless output semantics remain stable even
  though they stop importing concrete Agent Runtime session/event types.
- Active-change coordination: `add-mcp-servers-2026-08-07` also changes
  `extension-system`. Implementation of this change begins only after its
  authority-hardening amendment is complete and the change is archived, or
  after the two delta sets are explicitly merged by their owner.

## Release Gate

Smith MUST NOT advertise a user-installed native plugin SDK until the existing
versioned subprocess extension protocol and its capability broker are
implemented. Until then, native tool injection is documented and typed as a
trusted embedding interface only.
