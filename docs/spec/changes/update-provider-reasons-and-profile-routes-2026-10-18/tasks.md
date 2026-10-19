---
created_at: 2026-10-19T00:05:00Z
updated_at: 2026-10-19T00:05:00Z
---

## 1. Provider reasons

- [x] 1.1 Read the reason from an SSE-framed error body; label with a string
  `code` when there is no `status`.
- [x] 1.2 Drop a reason that echoes the bare token of a sent header value.
- [ ] 1.3 Move to Agent Runtime v0.7.0 so the reason survives the adapters and
  a rejection displays as `Provider:`.

## 2. Profile routes

- [x] 2.1 Hash a value's layer and key, not its file, in profile revisions.
- [x] 2.2 Resolve a stale route key by profile name; fingerprint an
  unroutable child so it never matches.
- [x] 2.3 Test: a parent resumes after its child profile changed or stopped
  being a child profile, and reports the child changed.

## 3. Live conformance

- [x] 3.1 `live_conformance`: tool turn, continuation, SIGKILL after a tool
  result then resume, every ordered pair of routes, hosted search plus tool.
- [ ] 3.2 Run it against the owner's routes before release and record the
  report.
