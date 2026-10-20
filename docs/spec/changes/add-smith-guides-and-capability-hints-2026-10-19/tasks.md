---
created_at: 2026-10-19T18:00:00Z
updated_at: 2026-10-20T01:29:31Z
completed_at: 2026-10-20T01:29:31Z
---

## 1. Task guides

- [x] 1.1 Write `docs/guides/{overview,setup,models-and-thinking,vision,profiles-and-advisor,sessions,extending,headless,troubleshooting}.md`. Each one says who runs each step, how to verify the result, and that changes apply when the session is next rebuilt.
- [x] 1.2 Register each guide in `built_in_skills.rs` as a whole-body skill `smith.guide.<topic>` with an authored routing description.
- [x] 1.3 Tests: bodies are byte-identical to the docs; the token bound holds (1,200 per guide, 600 for overview); every `smith` subcommand, slash command and config key named in a guide exists.
- [x] 1.4 Add a guides table to `docs/skills.md` (the README does not list the built-in skills, so it needs no change).

## 2. Vision tri-state

- [x] 2.1 Resolve `vision` to `Unset | Disabled(source) | Selected(target)` in `smith-config`.
- [x] 2.2 Make `smith config explain vision` report `unset` and `disabled by <source>`; add tests.

## 3. Setup hints

- [x] 3.1 Add `setup_hint: Option<SetupHint>` to `Mounted::Inactive` and `image_capable_profiles` to `ModuleContext`.
- [x] 3.2 Compute `image_capable_profiles` from the frozen catalog with no credential lookup.
- [x] 3.3 Compose the `smith.capability-hints` developer fragment from the inactive modules' hints, revisioned by content; omit it when there are no hints.
- [x] 3.4 Show hints in `/modules` and `smith config modules`.
- [x] 3.5 Vision module: return a hint only for a text-only main model with `vision` unset.
- [x] 3.6 Image-generation module: return a hint when the active provider has no image binding.
- [x] 3.7 Tests: the vision hint names the candidates; no hint with `vision = false`; no hint for a vision main model; read-only posture keeps the hint only while it admits `vision`; the fragment is stable across turns.

## 4. Verification

- [x] 4.1 Run the full gate (see the gate recipe).
- [x] 4.2 Live check with the `code` profile (glm-5.3) and `vision` removed: attach an image, and the agent names `visual` and offers to add the line.
- [x] 4.3 Same check with `vision = false`: no offer.
