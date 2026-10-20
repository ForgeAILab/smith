---
created_at: 2026-10-19T18:00:00Z
updated_at: 2026-10-20T01:29:31Z
---

## Why

A Smith agent knows little about driving Smith itself. The built-in skills
are reference documents (`smith.configuration`, `smith.headless`,
`smith.persistence`, `smith.security`): they say what every key means, but
not how to do a task end to end, which command the user types and which the
agent may run, or how to check the result. So the agent gives vague advice
or edits config blindly.

The agent is also never told when a capability is off for a reason the user
can fix. With a text-only main model and no `vision` setting, the vision
module is just inactive. The agent sees an image placeholder, says it cannot
see the image, and stops. It does not say that one config line would fix
this, or which of the user's profiles could serve it. Image generation with
no image binding has the same problem, though its switch defaults off on
such providers. The owner chose to keep vision
opt-in (image bytes go to a second provider), so the fix is to tell the agent
and the user, not to pick a vision model automatically.

## What Changes

- Add **built-in task guides**: short how-to skills named
  `smith.guide.<topic>`, compiled whole from `docs/guides/<topic>.md`. Each
  guide covers one user-facing task. It names the exact keys and commands,
  says who runs each step (the user, or the agent through ordinary approved
  tools), says how to verify the result, and notes that config changes apply
  when the session is next rebuilt. Initial set:
  `overview`, `setup`, `models-and-thinking`, `vision`,
  `profiles-and-advisor`, `sessions`, `extending`, `headless`,
  `troubleshooting`.
- Add **setup hints** to the module contract: an inactive module may carry a
  user-fixable hint (what is missing, the guide to activate, and candidate
  values found in the user's config). Smith turns the hints into one
  revisioned developer fragment, so the agent can raise the gap when it
  matters.
- The `vision` module returns a hint when the main model is text-only and
  `vision` is unset. The hint names the main-enabled profiles whose model
  accepts images. It returns none when `vision = false`.
- The `image-generation` module returns a hint when the active provider has
  no image binding.
- Configuration keeps `vision = false` apart from unset after resolution, and
  `smith config explain vision` reports which applies.
- The image placeholder format does not change.
- No automatic vision selection, no new config keys, no config reload.

## Impact

- Affected specs: `harness-policy`, `extension-system`, `tool-execution`,
  `configuration`.
- Affected code: `crates/smith-runtime/src/built_in_skills.rs`, new
  `docs/guides/*.md`, `docs/skills.md`, `crates/smith-module/src/lib.rs`
  (`Mounted::Inactive`, `ModuleContext`), module planner/composition in
  `crates/smith-runtime`, `crates/modules/vision`,
  `crates/modules/image-generation`, `crates/smith-config` vision resolution
  and `config explain`.
- Prompt: sessions with a hint gain one small developer fragment. It stays
  stable until the mounted set or config changes, so the cache prefix stays
  stable too.
