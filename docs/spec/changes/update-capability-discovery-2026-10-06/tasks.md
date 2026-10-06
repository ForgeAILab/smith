---
created_at: 2026-10-06T22:55:29Z
updated_at: 2026-10-06T22:55:29Z
completed_at:
---

## 1. Agent Runtime

- [ ] 1.1 Host-declared pinned initial activation, bound before retrieval and
  counted first against the capability budget; start fails closed when the
  pinned set does not fit.
- [ ] 1.2 `registry.search`: empty query and optional domain return the
  authorized catalog, paged; a no-match result reports per-domain counts.
- [ ] 1.3 `registry.activate`: stage named ids from the authorized view for
  the next request, within budget.
- [ ] 1.4 Retrieval scores name, summary, and description text; keywords
  boost; tools win ties over reference skills.
- [ ] 1.5 Allow and deny id patterns in the view filter; deny wins.
- [ ] 1.6 Conformance tests for 1.1 to 1.5; push the compat branch.

## 2. Smith runtime and configuration

- [ ] 2.1 Bump the Agent Runtime pin and lockfile.
- [ ] 2.2 Declare the pinned core in the factory, narrowed by posture.
- [ ] 2.3 `capabilities.allow` and `capabilities.deny` on profiles, with
  validation and provenance; pass them to the view filter.
- [ ] 2.4 Session deny set on `Selection`, applied at the safe-boundary
  rebuild; it cannot widen the profile.
- [ ] 2.5 Remove the host and network keywords added to `shell` in v0.3.10.

## 3. Surfaces

- [ ] 3.1 `/capabilities` lists active, available, and denied entries with
  the source of each denial.
- [ ] 3.2 `/capabilities deny <id>` and `/capabilities allow <id>` for the
  session's own denials.
- [ ] 3.3 Tool rows for `registry.activate`; listing rows stay suppressed
  like `registry.search`.

## 4. Verification

- [ ] 4.1 Factory tests: first request of a build-posture session carries the
  core tools; a read-only posture carries only the read subset; a denied id
  is absent from tools, listing, and search.
- [ ] 4.2 Replay the 2026-10-06 prompt ("scan <host>") against the fake
  provider: `shell` is in the first request's tool list.
- [ ] 4.3 Live tmux walkthrough on the real config: browse, activate a skill
  by name, deny `tool:shell` for the session, confirm it leaves the list.
- [ ] 4.4 Cache A/B against the previous release, since the tool list
  changed.
