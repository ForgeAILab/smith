---
created_at: 2026-10-23T23:30:00Z
updated_at: 2026-10-23T23:30:00Z
---

## Why

In a live session the agent told the user it had no web search and could not
delegate, although both tools were registered. Discovery failed four ways:

- The model miscopied the hashed provider alias of `registry.activate`
  (`registry_activate_<32 hex>`).
- `registry.search` ranked many host skills above `web_search`, and the
  top-8 cut dropped it.
- On a second search `web_search` was retrieved and then silently rejected
  as redundant: Smith gives module tools such as `web_fetch` and `web_search`
  only permission-derived affordances, so equal permissions read as equal
  coverage.
- The model searched `domain: "agent"`, but Smith's `agent` tool is in the
  `tool` domain.

## What Changes

- Move the agent-runtime pin to the 0.7-line commit carrying runtime change
  `update-tool-names-and-discovery-2026-10-23`: provider tool names without
  hash suffixes, collision rejection, offered-tool lists on unknown calls,
  name-part search matching, domain interleaving and fallback, `not_staged`
  reporting, and two-term skill pre-activation.
- Smith's own ChatGPT provider drops its `smith_tool_<hash>` alias and sends
  the same readable provider name as the runtime adapters
  (`registry.search` → `registry_search`); tools sharing one are rejected as
  before.
- A tool without a curated affordance (module, plugin, or remote tool) also
  offers one unique to its name, so selection never treats two such tools as
  redundant.
- Every root and child session gets one developer fragment naming the
  registered tools it may activate but does not start with (registered tools
  minus the pinned core and anything the capability limits deny).
- Remote and sandboxed tool name checks compare provider names, so a tool
  that a provider would see under a taken name is skipped with a warning
  instead of failing the session. The CLI bridge uses the same normalizer.

## Impact

- Affected specs: `runtime-integration`.
- Affected code: root `Cargo.toml`/`Cargo.lock` pin,
  `smith-runtime/src/{abilities,capability_limits,cli_bridge,delegation}.rs`,
  `smith-runtime/src/chatgpt/encoding.rs`,
  `smith-runtime/src/factory/{capabilities,construction,modules}.rs`.
- Wire tool names change once for `registry.search`, `registry.activate`,
  `artifact.*` and any other dotted name; the provider cache prefix misses
  once. Persisted sessions are unaffected.
- Headless fixtures change where pre-activation bound a skill on one shared
  word.
