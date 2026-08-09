---
created_at: 2026-08-09T05:52:22Z
updated_at: 2026-08-09T06:23:07Z
completed_at:
---

## 0. Approval

- [x] 0.1 Approve this proposal before implementation or publication.

## 1. Portable build matrix

- [x] 1.1 Add digest-pinned Cargo Zigbuild legs for
  `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` with locked,
  isolated builds.
- [x] 1.2 Package musl binaries as the canonical
  `smith-{arch}-linux.tar.gz` archives.
- [x] 1.3 Rename GNU alternatives to `smith-{arch}-linux-gnu.tar.gz` and build
  them on a fixed older glibc baseline.
- [x] 1.4 Keep macOS artifact names and behavior unchanged.

## 2. Artifact and compatibility gates

- [x] 2.1 Verify each musl artifact's ELF architecture, executable mode,
  static linkage, and reported Smith version.
- [x] 2.2 Smoke the x86_64 canonical archive in clean Debian 12 and Alpine
  containers.
- [x] 2.3 Execute the ARM64 binary on a native or emulated ARM64 Linux runtime.
- [x] 2.4 Generate checksums over every release archive and make release
  creation depend on every build/smoke gate.

## 3. npm bootstrapper

- [x] 3.1 Select the canonical musl archive for every supported Linux host,
  independent of Node's libc.
- [x] 3.2 Add Node tests for macOS, Linux x86_64/ARM64, unsupported platforms,
  archive naming, and release-tag selection.
- [x] 3.3 Document the portable default and explicitly named GNU alternative.

## 4. Version and documentation

- [x] 4.1 Bump the Cargo workspace and lockfile from `0.0.1` to `0.0.2` without
  changing the npm template's publish-time versioning contract.
- [x] 4.2 Update install/release documentation with the new artifact names,
  Linux portability contract, credential choices, and checksum command.
- [x] 4.3 Add a release note explaining the `v0.0.1` glibc limitation and the
  source-build workaround for users who cannot upgrade immediately.

## 5. Verification

- [x] 5.1 Run formatting, Clippy with warnings denied, workspace tests, npm
  bootstrapper tests, and release-workflow validation.
- [x] 5.2 Build both musl targets with the exact release command and confirm
  the vendored keyring/D-Bus path remains statically linked.
- [x] 5.3 Validate this spec change strictly after task updates.

## 6. Publish `v0.0.2`

- [x] 6.1 Commit the scoped implementation without including unrelated dirty
  worktree changes and push the release branch.
- [ ] 6.2 Create and push immutable tag `v0.0.2` at the reviewed commit.
- [ ] 6.3 Monitor the release workflow until all Linux/macOS builds, smokes,
  checksums, GitHub Release creation, and npm trusted publication succeed.
- [ ] 6.4 Verify the public archives and `@forgeailab/smith@0.0.2`, then install
  the canonical x86_64 Linux archive on the Debian 12 server and run
  `smith --version`.
