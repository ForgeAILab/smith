## ADDED Requirements

### Requirement: Crate boundaries are checked by structure

Smith's architecture tests SHALL check crate boundaries from manifests and
source structure rather than file names or literal strings: `smith-tui` has
no `smith-config` dependency, `smith-client` has no terminal-library
dependency, headless code does not import `smith-tui`, production
`smith-cli` code has no glob imports, and `smith-runtime` exposes only an
allow-listed set of modules. Modules of `smith-runtime` with no consumer
outside the crate MUST be crate-private, and `unreachable_pub` SHALL be
denied for that crate.

#### Scenario: A new public module

- **GIVEN** a contributor adds a `pub mod` to `smith-runtime` that is not on
  the allow-list
- **WHEN** the tests run
- **THEN** the architecture test fails and names the module
