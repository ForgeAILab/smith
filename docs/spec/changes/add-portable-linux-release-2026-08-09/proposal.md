---
created_at: 2026-08-09T05:52:22Z
updated_at: 2026-08-09T06:23:07Z
---

## Why

The public `v0.0.1` GNU/Linux binary was built on `ubuntu-latest` and requires
glibc 2.38/2.39. It therefore fails before `main` on Debian 12, whose glibc is
2.36, even though Debian 12 is a current and otherwise supported server
environment. Building the same tag from source on that server works, proving
that the defect is in artifact portability rather than Smith's runtime.

Smith already has a digest-pinned Cargo Zigbuild path that produced fully
static x86_64 and ARM64 musl binaries, including the vendored D-Bus backend
used by the platform keyring. The public release should reuse that proven
shape instead of requiring users to compile Smith or dropping credential
service support.

## What Changes

- Publish fully static `x86_64-unknown-linux-musl` and
  `aarch64-unknown-linux-musl` binaries under Smith's canonical Linux archive
  names.
- Keep optional GNU/Linux artifacts under explicit `-linux-gnu` names and
  build them on an older fixed glibc baseline rather than `ubuntu-latest`.
- Make the npm bootstrapper select the canonical musl archive for every Linux
  host, whether Node itself uses glibc or musl.
- Preserve macOS Keychain and Linux Secret Service support. Musl builds keep
  the existing vendored D-Bus/keyring implementation; owner-only plaintext
  config and environment references remain explicit alternatives.
- Add artifact checks that prove the musl binaries are static, execute the
  native architecture's binary, and exercise the x86_64 artifact on both
  Debian 12 and Alpine before a release can be created.
- Bump the Smith workspace to `0.0.2`, then publish GitHub tag/release
  `v0.0.2` and npm package `@forgeailab/smith@0.0.2` only after every gate is
  green.

## Impact

- Affected specs: `release-distribution` (new)
- Affected code: Cargo workspace version and lockfile, release workflow, npm
  bootstrapper/tests, installation/release documentation
- Compatibility: existing macOS archive names remain unchanged; Linux npm
  users transparently move to the portable static binary
- Security: no credential-storage downgrade; musl keeps keyring support and
  the existing explicit owner-only plaintext/environment choices
- External publication: one new immutable GitHub release/tag and one npm
  package version after verification

## Out of Scope

- Rewriting the credential broker or changing its storage precedence.
- Silently falling back from a requested keychain reference to plaintext.
- Supporting Linux architectures other than x86_64 and ARM64 in `v0.0.2`.
- Retagging, replacing, or mutating the immutable `v0.0.1` release.
- Signing release artifacts; checksums remain the integrity mechanism for this
  patch release.

## Approval Boundary

Approval authorizes the build, bootstrapper, version, tests, documentation,
commit, tag, GitHub release, and npm publication work described in
`tasks.md`. It authorizes publishing `v0.0.2` only when the release workflow,
static-link checks, compatibility smokes, checksums, GitHub release, and npm
publish all succeed.

It does not authorize mutating `v0.0.1`, deleting existing release artifacts,
dropping keyring support, storing credentials without explicit user choice, or
overwriting unrelated changes already present in this dirty worktree.
