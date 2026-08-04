---
created_at: 2026-07-31T22:06:35Z
updated_at: 2026-07-31T22:07:08Z
---

## Why

Smith's execution and safety foundations now work in a real coding turn, but a
clean-room Z.AI Coding Plan benchmark exposed an agent-experience gap. The
idle TUI hides agent identity and discovery, the default GLM-5.2 response
budget can expire before the first edit, capability retrieval can choose broad
shell access instead of exact read/edit tools, and a successful result can
retain a non-terminal todo item. Protected mid-turn recovery also still needs
an operating-system credential prompt, contrary to the requested local
no-prompt setup.

OpenCode is a behavioral reference for discoverability and work visibility,
not an implementation dependency. Smith should adopt the useful interaction
patterns while retaining exact prepared authorization, depth-one delegation,
redacted journals, project cleanliness, and transcript-first presentation.

## What Changes

- Enrich the idle composer/footer with the active agent mode, provider/model,
  project and branch, and context provenance without adding a permanent header,
  pane, or shortcut strip; expose the local command/composer guide through `?`
  and `/help` on demand.
- Require two `Ctrl+C` presses within one second to exit, with the first press
  clearing and locally stashing the draft for `Up`-arrow recovery.
- Add bounded Smith-owned `build`, `plan`, and `review` agent modes. **BREAKING:**
  `Tab` on an empty idle composer cycles these modes instead of being a no-op;
  command, questionnaire, and completion overlays retain their current `Tab`
  behavior.
- Add one typed `@` completion surface for authorized workspace-file
  attachments and explicit read-only child-agent presets, plus a `!` local
  shell shortcut that executes only through Smith's prepared tool authority
  and approval path.
- Replace noisy repeated lifecycle notices and aggregate work rows with one
  replay-equivalent todo pane anchored above the composer, add a details
  toggle for explicit tool lifecycle detail, and expose local session/child
  timeline and safe redo navigation without adding project metadata files.
- Make successful and unsuccessful terminal results reconcile todo state,
  discard uncommitted reasoning on limits/retries, and keep machine output
  structurally honest.
- Tune intent retrieval so coding tasks prefer exact read/edit capabilities
  over broad shell authority, and give cataloged GLM-5.2 Coding Plan profiles a
  validated 32,768-token request budget unless a higher-precedence source
  explicitly overrides it.
- Add an explicit owner-only user-config or environment checkpoint-key source.
  Selecting it bypasses Keychain/Secret Service completely while checkpoints
  remain authenticated-encrypted; there is no plaintext-checkpoint fallback.
- Add scenario and live-product evaluations against a disposable coding task,
  including no-Keychain durable resume and narrow/normal/wide terminal QA.

## Impact

- Affected specs: `client-interaction`, `child-agents`, `harness-policy`,
  `configuration`, `session-recovery`, `change-review`
- Affected code: `crates/smith-tui`, `crates/smith-cli`,
  `crates/smith-runtime`, `crates/smith-config`, setup/persistence adapters,
  `DESIGN.md`, and product-evaluation fixtures
- Supersedes only the idle empty-composer `Tab` no-op scenario in
  `update-smith-interaction-model`; the composer remains the sole persistent
  focus target. It extends, rather than weakens, the prepared-approval,
  one-level child, and protected-checkpoint requirements in active changes.
