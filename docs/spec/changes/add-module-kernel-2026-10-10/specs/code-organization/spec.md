## ADDED Requirements

### Requirement: One crate per module behind a cargo feature

Each first-party module SHALL live in its own crate that depends on the
module contract crate and Agent Runtime, and MAY depend on `smith-tools`. A
module crate MUST NOT depend on `smith-runtime`, `smith-config`, `smith-tui`,
or `smith-cli`; what it needs from the host MUST reach it through the module
context. The `smith-cli` crate SHALL
expose one cargo feature per first-party module, all enabled by default, and
a build with default features disabled MUST compile and run as a working
Smith with no optional module. Architecture tests SHALL check these rules
from manifests.

#### Scenario: Minimal build

- **WHEN** `smith-cli` is built with default features disabled
- **THEN** the build succeeds
- **AND** the resulting binary starts a session with the core tools
- **AND** its module listing shows every first-party module as not built

#### Scenario: Module crate reaches into the runtime crate

- **GIVEN** a contributor adds a `smith-runtime` dependency to a module
  crate
- **WHEN** the architecture tests run
- **THEN** they fail naming the crate and the dependency

#### Scenario: Module without a feature

- **GIVEN** a first-party module crate that `smith-cli` depends on
  unconditionally
- **WHEN** the architecture tests run
- **THEN** they fail naming the module

### Requirement: One compiled-in module list

The modules compiled into a binary SHALL be named in exactly one place in
the composition root, one entry per module, each first-party entry gated by
that module's cargo feature. Modules MUST NOT register themselves through
link-time or static-constructor mechanisms.

#### Scenario: Adding a module to a build

- **WHEN** a build adds one entry to the compiled-in module list and the
  crate dependency it names
- **THEN** the module is known to that binary
- **AND** no other source file needs to change for it to be listed and
  mountable

#### Scenario: Hidden registration is attempted

- **GIVEN** a module crate that registers itself through a link-time
  collection
- **WHEN** the architecture tests run
- **THEN** they fail naming the crate
