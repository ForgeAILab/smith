## ADDED Requirements

### Requirement: The WASM host is an optional, isolated crate

The WASM component host SHALL live in its own crate, compiled only when the
`wasm-modules` cargo feature of `smith-cli` is enabled, and no other
workspace crate MAY depend on a WASM engine. The host crate MUST depend on
the module contract crate and MUST NOT depend on `smith-tui` or `smith-cli`.
A build without the feature MUST compile, run, and report WASM modules as
not supported in that build. Architecture tests SHALL check these rules from
manifests.

#### Scenario: Build without the feature

- **WHEN** `smith-cli` is built without `wasm-modules`
- **THEN** no WASM engine crate is in the dependency graph
- **AND** an installed WASM module is listed as not supported in this build

#### Scenario: Engine dependency leaks

- **GIVEN** a contributor adds a WASM engine dependency to `smith-runtime`
- **WHEN** the architecture tests run
- **THEN** they fail naming the crate and the dependency
