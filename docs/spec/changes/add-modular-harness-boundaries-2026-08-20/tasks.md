---
created_at: 2026-08-20T22:13:02Z
updated_at: 2026-08-21T01:33:37Z
completed_at: 2026-08-21T01:33:37Z
---

## 0. Coordination and Compatibility

- [x] 0.1 Resolve the active `add-mcp-servers-2026-08-07`
  `extension-system` overlap before implementation.
- [x] 0.2 Inventory every direct consumer of `RuntimeRequest`, the re-exported
  `SessionHandle`, delegation types, and Agent Runtime events; freeze current
  TUI/headless/embedding fixtures as compatibility evidence.
- [x] 0.3 Publish the deprecation and minimum supported Smith client-protocol
  version before removing any public re-export.

## 1. Harness Resolution

- [x] 1.1 Add declarative `HarnessSpec`, immutable `ResolvedHarness`,
  `HarnessIdentity`, and revision/provenance-bearing resolved provider,
  authority, persistence, context, delegation, and module records.
- [x] 1.2 Add `ResolvedModule` with separate `Contribution` declarations,
  requested capabilities, trust classification, and host-computed grants.
- [x] 1.3 Validate module IDs, revisions, contribution collisions, trust, and
  grant coverage before provider I/O or runtime construction; emit a bounded
  resolution report.
- [x] 1.4 Adapt standard TUI, headless, child, test, and embedded hosts to use
  the same resolver and compare equal resolved policies for equal inputs.

## 2. Trusted Native and External Extension Boundaries

- [x] 2.1 Replace `RuntimeRequest.tools` with an explicitly named
  trusted-native module contribution and make the trust classification visible
  in Rustdoc and composition evidence.
- [x] 2.2 Ensure a native contribution cannot claim sandboxing or an untrusted
  provenance class; reject configuration that attempts to load arbitrary
  native dynamic libraries.
- [x] 2.3 Define the process-protocol mapping for tool, skill, command, observer,
  and declarative panel contributions without giving the extension direct
  runtime handles or renderer memory.
- [x] 2.4 Gate any public user-installed plugin surface on a running
  capability-brokered subprocess implementation with bounded framing,
  cancellation, lifecycle, filesystem, network, secret, and approval requests.
- [x] 2.5 Test that contribution declarations alone grant no filesystem,
  process, network, credential, approval, or provider authority.

## 3. Smith Client Protocol

- [x] 3.1 Add versioned Smith-owned input, receipt, ID, lifecycle, usage,
  approval, interaction, tool, child, and terminal event types.
- [x] 3.2 Implement one adapter from Agent Runtime canonical events and
  `SessionHandle` operations to `SmithSession` and `SmithEvent`; preserve causal
  ordering, attribution, redaction, and bounded payload references.
- [x] 3.3 Migrate the TUI reducer and CLI/headless projections to the Smith
  client protocol without changing rendered or machine-output fixtures.
- [x] 3.4 Keep canonical persistence on Agent Runtime events; add replay tests
  proving the client projection can be rebuilt without becoming a second
  execution or persistence mechanism.
- [x] 3.5 Add compatibility tests for unknown future Smith events and remove
  direct Agent Runtime facade imports from presentation crates.

## 4. Staged Factory

- [x] 4.1 Express the factory as private `resolve`, `provider`, `authority`,
  `capabilities`, `persistence`, `delegation`, and `compose` stages whose inputs
  and outputs are typed.
- [x] 4.2 Preserve the single public `build(ResolvedHarness)` composition root
  and existing validation order, provider wrapping, activation order,
  checkpoint durability, contributors, hooks, and runtime policy.
- [x] 4.3 Keep internal visibility narrow and add architecture tests preventing
  entry points or clients from constructing Agent Runtime policy directly.

## 5. Verification

- [x] 5.1 Run formatting, Clippy, workspace/all-feature tests, machine-output
  contract tests, TUI replay/snapshot tests, and Agent Runtime consumer
  conformance.
- [x] 5.2 Add an embedding fixture with two differently resolved harnesses in
  one process and prove their modules, grants, clients, and events do not leak.
- [x] 5.3 Update architecture, extension, Forge-integration, and public Rustdoc
  documentation with the explicit trust-tier table.
