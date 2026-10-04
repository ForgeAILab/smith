## ADDED Requirements

### Requirement: Portable Linux release artifacts

Every stable Smith release SHALL publish fully static musl binaries for
x86_64 and ARM64 under the canonical Linux archive names. A release MAY also
publish GNU/Linux alternatives, but their archive names and minimum glibc
baseline MUST be explicit.

#### Scenario: Canonical x86_64 Linux archive runs on Debian 12

- **GIVEN** the canonical x86_64 Linux archive from a candidate release
- **WHEN** its binary is extracted and executed in a clean Debian 12 runtime
- **THEN** `smith --version` succeeds without installing build dependencies
- **AND** the reported version matches the release tag

#### Scenario: Canonical x86_64 Linux archive runs on Alpine

- **GIVEN** the same canonical x86_64 Linux archive
- **WHEN** its binary is extracted and executed in a clean Alpine runtime
- **THEN** `smith --version` succeeds without a glibc compatibility layer

#### Scenario: ARM64 portable artifact is executable

- **GIVEN** the canonical ARM64 Linux archive from a candidate release
- **WHEN** its binary is executed on a native or emulated ARM64 Linux runtime
- **THEN** `smith --version` succeeds and matches the release tag

### Requirement: Portable artifacts preserve credential choices

The portable Linux build MUST retain Smith's vendored platform-keyring
implementation while continuing to support explicit environment references
and owner-only plaintext user configuration. It MUST NOT silently replace a
requested protected credential source with plaintext.

#### Scenario: Static linkage retains the keyring implementation

- **GIVEN** either musl release binary
- **WHEN** its ELF linkage and compiled dependency graph are inspected
- **THEN** the binary has no dynamic interpreter or unresolved shared library
- **AND** the platform keyring code remains enabled

#### Scenario: Headless Linux has no credential service

- **GIVEN** a musl Smith binary running without a usable Secret Service
- **WHEN** a configured keychain credential is resolved
- **THEN** Smith returns its bounded credential-service diagnostic
- **AND** it does not write the credential into plaintext config automatically

### Requirement: Linux bootstrap selection

The npm bootstrapper SHALL select the canonical portable archive for every
supported Linux architecture without making the result depend on the host
glibc version or Node's libc implementation.

#### Scenario: npm starts Smith on glibc Linux

- **GIVEN** the matching `@forgeailab/smith` package on x86_64 or ARM64 glibc
  Linux
- **WHEN** the bootstrapper resolves its release archive
- **THEN** it selects the canonical static musl archive for that architecture

#### Scenario: npm starts Smith on musl Linux

- **GIVEN** the matching package on x86_64 or ARM64 musl Linux
- **WHEN** the bootstrapper resolves its release archive
- **THEN** it selects the same canonical static musl archive

### Requirement: Release publication is gated and version-coherent

A Smith release SHALL be created only after every declared platform artifact
and compatibility smoke succeeds. The binary version, Git tag, GitHub release,
npm package version, archive checksums, and npm-selected release tag MUST refer
to the same immutable patch version.

#### Scenario: Publish version 0.0.2

- **GIVEN** a reviewed commit whose Smith workspace reports `0.0.2`
- **WHEN** tag `v0.0.2` triggers the release workflow
- **THEN** every build and compatibility gate succeeds before release creation
- **AND** GitHub publishes `v0.0.2` with checksums for every archive
- **AND** npm publishes `@forgeailab/smith@0.0.2` selecting tag `v0.0.2`

#### Scenario: A portable build or smoke fails

- **GIVEN** any declared artifact fails to build, is dynamically linked, has
  the wrong architecture or version, or fails a compatibility smoke
- **WHEN** the workflow evaluates the release dependencies
- **THEN** it creates neither the GitHub release nor the npm package
