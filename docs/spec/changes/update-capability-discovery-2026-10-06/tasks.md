---
created_at: 2026-10-06T22:55:29Z
updated_at: 2026-10-06T22:55:29Z
completed_at:
---

## 1. Agent Runtime

- [x] 1.1 Host-declared pinned initial activation, bound before retrieval and
  counted first against the capability budget; start fails closed when the
  pinned set does not fit.
- [x] 1.2 `registry.search`: empty query and optional domain return the
  authorized catalog, paged; a no-match result reports per-domain counts.
- [x] 1.3 `registry.activate`: stage named ids from the authorized view for
  the next request, within budget.
- [x] 1.4 Retrieval scores name, summary, and description text; keywords
  boost; tools win ties over reference skills.
- [x] 1.5 Allow and deny id patterns in the view filter; deny wins.
- [x] 1.6 Conformance tests for 1.1 to 1.5; push the compat branch.

## 2. Smith runtime and configuration

- [x] 2.1 Bump the Agent Runtime pin and lockfile.
- [x] 2.2 Declare the pinned core in the factory, narrowed by posture.
- [x] 2.3 `capabilities.allow` and `capabilities.deny` on profiles, with
  validation and provenance; pass them to the view filter.
- [x] 2.4 Session deny set on `Selection`, applied at the safe-boundary
  rebuild; it cannot widen the profile.
- [x] 2.5 Remove the host and network keywords added to `shell` in v0.3.10.

## 3. Surfaces

- [x] 3.1 `/capabilities` lists active, available, and denied entries with
  the source of each denial.
- [x] 3.2 `/capabilities deny <id>` and `/capabilities allow <id>` for the
  session's own denials.
- [x] 3.3 Tool rows for `registry.activate`; listing rows stay suppressed
  like `registry.search`.

## 4. Verification

- [x] 4.1 Factory tests: first request of a build-posture session carries the
  core tools; a read-only posture carries only the read subset; a denied id
  is absent from tools, listing, and search.
- [x] 4.2 Replay the 2026-10-06 prompt ("scan <host>") against the fake
  provider: `shell` is in the first request's tool list.
- [ ] 4.3 Live tmux walkthrough on the real config: browse, activate a skill
  by name, deny `tool:shell` for the session, confirm it leaves the list.
- [ ] 4.4 Cache A/B against the previous release, since the tool list
  changed.
