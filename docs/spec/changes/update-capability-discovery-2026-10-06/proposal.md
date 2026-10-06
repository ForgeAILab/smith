---
created_at: 2026-10-06T22:55:29Z
updated_at: 2026-10-06T22:55:29Z
---

## Why

A session starts with only `registry.search`. Every other tool, including
`shell`, appears only when the prompt or a search hits one of its
hand-written keywords by exact word. On 2026-10-06 a user asked Smith to scan
their own server: nothing matched, the model's own searches returned Smith's
reference skills, and it answered that it had no terminal. v0.3.10 widened
the `shell` keyword list, which fixes that one prompt and no others. The
owner's direction: the core tools are always there, the agent finds and
activates the rest itself, and restriction is a limit set on the profile or
session, not a side effect of what search happens to return.

## What Changes

- **Core tools are always active.** `read`, `list`, `search`, `edit`,
  `shell`, `task_output`, and `task_stop` are in every activation epoch
  from the first request, narrowed only by the profile's posture and the
  capability limits below. They no longer depend on retrieval. **This
  reverses the current rule** that `edit` and `shell` are not advertised
  merely because they are installed.
- **The agent can browse, not only search.** `registry.search` accepts an
  empty query and an optional domain, and returns the authorized catalog as
  name plus one-line summary, paged. A search with no match says so and
  reports how many capabilities exist per domain, so "nothing matched" can
  never read as "nothing exists".
- **The agent can activate by name.** A new `registry.activate` tool takes
  registry ids the agent has seen and stages them for the next request,
  within the same budget and authorization as today.
- **Search matches descriptions, not only keyword lists.** Retrieval scores
  the name, summary, and description text of each card. Hand-written
  keywords remain as a boost, not as the only way in. Tools rank above
  reference skills on equal scores.
- **Limits are explicit.** A profile may declare `capabilities.allow` and
  `capabilities.deny` patterns over registry ids. A denied capability is
  absent from the session: not active, not listed, not searchable. A session
  can narrow further with `/capabilities deny <id>`, never widen past its
  profile.
- **`/capabilities`** lists what is active, what is available to activate,
  and what the profile or session denies.
- The `shell` keywords added in v0.3.10 are removed again; they are
  redundant once the tool is always active.

## Impact

- Affected specs: `runtime-integration`, `configuration`, `client-surfaces`
- Affected code:
  - Agent Runtime (`fix/smith-command-provider-compat`):
    `capability/retrieval.rs`, `capability/preactivation.rs`,
    `harness/capability_search.rs`, `harness/live_abilities/`; then a pin
    bump in Smith's `Cargo.toml`.
  - Smith: `smith-runtime/src/factory/construction.rs` and `abilities.rs`
    (pinned core, limits), `smith-config` (profile keys), `smith-client`
    and `smith-tui` (`/capabilities`).
- Prompt cache: the tool list changes less often, since core tools no longer
  arrive mid-session. Every first request grows by the core schemas (about
  seven small tool schemas).
- Not in scope: embedding or model-based retrieval, changing approval or
  permission checks, and per-tool argument restrictions.
