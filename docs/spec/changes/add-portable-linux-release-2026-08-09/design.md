---
created_at: 2026-08-09T05:52:22Z
updated_at: 2026-08-09T06:23:07Z
---

## Context

The `v0.0.1` Linux archives use GNU targets built by a moving
`ubuntu-latest` runner. ELF version requirements from that runner exceed the
glibc available on Debian 12. The npm bootstrapper distinguishes GNU and musl
hosts, but `v0.0.1` publishes only GNU archives, so neither branch provides a
portable answer.

The Harbor evaluation worktree contains a digest-pinned Cargo Zigbuild builder
that has already emitted stripped, statically linked Smith binaries for both
musl targets. That evidence also resolves the keyring question: the existing
`vendored` D-Bus feature compiles into the static artifacts, so portable Linux
does not require removing Secret Service integration.

## Goals / Non-Goals

- Goals:
  - Make the ordinary Linux download work on Debian 12, Alpine, and newer
    mainstream distributions without compiling on the destination.
  - Preserve x86_64 and ARM64 coverage and existing credential behavior.
  - Make artifact names and npm selection deterministic.
  - Publish the correction as immutable patch release `v0.0.2`.
- Non-Goals:
  - Guarantee compatibility with non-Linux kernels or unsupported CPU
    architectures.
  - Introduce automatic plaintext credential fallback.
  - Repair `v0.0.1` in place.

## Decisions

### Canonical Linux means musl

`smith-x86_64-linux.tar.gz` and `smith-aarch64-linux.tar.gz` will contain the
corresponding musl binaries. These are the default documented and npm-selected
artifacts. The stable names avoid asking an installer to reason about the
host's glibc version.

Native GNU builds remain available as
`smith-x86_64-linux-gnu.tar.gz` and
`smith-aarch64-linux-gnu.tar.gz`. They use a fixed older Ubuntu baseline so
their minimum glibc is deliberate. They are alternatives, not npm defaults.

Alternatives considered:

- Publishing musl only under `-linux-musl` would leave the existing npm glibc
  branch selecting the broken class of artifact.
- Building GNU only on Ubuntu 22.04 would fix Debian 12 today but retain a
  moving minimum-libc maintenance obligation.
- Replacing GNU archives in place without explicit naming would make it hard
  for users to know which libc contract they downloaded.

### Keep the platform keyring

The musl release uses Smith's existing keyring dependency with vendored D-Bus.
The artifacts must remain fully static even with that feature enabled. Setup
continues to offer platform credential storage, environment references, and
explicit owner-only plaintext config. There is no target-specific silent
fallback.

If the keyring service is unavailable at runtime, Smith's existing bounded
diagnostic and explicit recovery choices remain the behavior. Compile-time
removal is unnecessary because the static builds have already succeeded.

### Build from a pinned toolchain image

The release workflow will use a digest-pinned Cargo Zigbuild image for musl,
with one native-architecture build leg per target when hosted runners permit.
The command uses `--locked`, the repository Rust version policy, an isolated
target directory, and the tagged checkout. The workflow itself packages the
result so a container cannot publish independently.

GNU artifacts move from `ubuntu-latest` to a fixed supported older Ubuntu
runner. The release notes name their baseline and recommend musl for broad
portability.

### Verify before publication

For each musl binary the workflow checks ELF architecture, executable mode,
absence of a dynamic interpreter/shared-library dependency, and
`smith --version`. The x86_64 archive additionally runs in clean Debian 12 and
Alpine containers. ARM64 gets a native or emulated version smoke before the
release job can consume it.

The release job depends on all build and smoke jobs, flattens every archive,
and generates one `SHA256SUMS`. npm publication continues to depend on the
GitHub release and uses trusted publishing.

## Risks / Trade-offs

- Static musl binaries may be larger than dynamically linked GNU binaries;
  portability is more important for the default archive.
- D-Bus/Secret Service availability is a runtime desktop-session property;
  static linking removes the library dependency but cannot create a session
  service on a headless server.
- Adding two GNU alternatives increases release size and matrix time. Their
  explicit names prevent installer ambiguity.
- A digest-pinned build image is reproducible but needs deliberate digest
  updates when the Rust toolchain changes.

## Migration Plan

1. Add and test the portable release matrix without changing `v0.0.1`.
2. Make npm Linux selection target the canonical musl archive.
3. Bump the workspace and lockfile to `0.0.2` and update installation docs.
4. Run local checks, then commit and push the implementation.
5. Create and push `v0.0.2`; monitor all build, smoke, GitHub release, and npm
   jobs to completion.
6. Download the public x86_64 canonical archive on the Debian 12 server that
   exposed the bug and verify `smith 0.0.2` there.

## Open Questions

None. The failed Debian 12 binary and successful static musl builds resolve the
artifact and keyring choices for this patch.
