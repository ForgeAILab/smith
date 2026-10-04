---
created_at: 2026-08-20T20:28:10Z
updated_at: 2026-08-20T21:14:00Z
completed_at:
---

## 1. Executable trust kind

- [x] 1.1 Add `ExecutableKind::Skill` with the stable string `skill`, and
  extend the enum's doc comment to say why privileged instructions are
  authority even though nothing is spawned.
- [x] 1.2 Add `skill_decision_binds_path_and_content_together`.
- [x] 1.3 Add `trust_file_written_before_skills_still_loads`.

## 2. Discovery

- [x] 2.1 Add `smith-runtime/src/skills/discovery.rs` behind the existing
  `skills` facade; keep the public path `smith_runtime::skills::…`.
- [x] 2.2 Parse the bounded frontmatter grammar: `---` fence, `key: value`
  lines, `description` required, `name` optional and required to match the
  directory, every other key ignored.
- [x] 2.3 Enforce the bounds: 256 skills per layer, 1 MiB body, 64 frontmatter
  lines, 1024-character description.
- [x] 2.4 Digest each body once and declare with `Skill::from_verified_file`.
- [x] 2.5 Return discovery problems as typed values carried beside the
  declarations; log nothing.
- [x] 2.6 Skip non-directories and directories without `SKILL.md` silently.
- [x] 2.7 Add `directory_name_is_the_skill_name`.
- [x] 2.8 Add `frontmatter_name_mismatch_is_a_problem_and_registers_nothing`.
- [x] 2.9 Add `missing_description_is_a_problem_and_the_others_still_load`.
- [x] 2.10 Add `unknown_frontmatter_keys_are_ignored`.
- [x] 2.11 Add `oversized_body_is_reported_not_truncated`.
- [x] 2.12 Add `per_layer_count_bound_reports_what_it_dropped`.
- [x] 2.13 Add `invalid_directory_name_is_reported`.
- [x] 2.14 Add `loose_file_and_bodyless_directory_are_skipped_silently`.
- [x] 2.15 Add `discovery_pins_the_bytes_it_read`.
- [x] 2.16 Add `missing_skills_directory_creates_nothing`.

## 3. Workspace trust resolution

- [x] 3.1 Resolve each workspace declaration through
  `Executable::from_file(project, ExecutableKind::Skill, path)` and
  `TrustStore::status`, and pass the result to `with_workspace`.
- [x] 3.2 Reuse the digest computed in 2.4 as the trust digest so the decision
  and the activation pin cannot diverge.
- [x] 3.3 Turn an out-of-project canonicalization into a discovery problem
  rather than an error that stops discovery.
- [x] 3.4 Add `untrusted_project_skill_is_indexed_inert_and_does_not_shadow`.
- [x] 3.5 Add `edited_project_skill_reports_changed_not_trusted`.
- [x] 3.6 Add `symlinked_project_skill_out_of_root_is_refused`.
- [x] 3.7 Add `approved_project_skill_activates_the_reviewed_digest`.

## 4. Host composition

- [x] 4.1 Fold discovered user and workspace declarations onto
  `built_in_sources()` in `start_host`; leave `RuntimeRequest::new` unchanged.
- [x] 4.2 Use the same helper on the headless path so `smith -p` and the TUI
  index one catalog.
- [x] 4.3 Carry discovery problems on the started host for the surface to read.
- [x] 4.4 Add `tui_and_headless_discover_the_same_catalog`.
- [x] 4.5 Add `malformed_skill_does_not_stop_host_start`.
- [x] 4.6 Add `headless_does_not_prompt_for_an_untrusted_project_skill`.

## 5. The `/skills` surface

- [x] 5.1 Add `CommandAction::Skills(SkillsAction::{List, Trust})` to the TUI
  command registry with its argument hint and description.
- [x] 5.2 Render the index grouped by layer with description, activatable
  state, shadowing, trust reason, and discovery problems.
- [x] 5.3 Add the CLI skill context owning the trust store and the discovered
  workspace declarations across rebuilds, mirroring `McpContext`.
- [x] 5.4 Show path and digest before recording a decision, reusing the `/mcp`
  confirmation shape.
- [x] 5.5 Set a pending-recompose flag on a recorded decision and break with
  `InteractiveExit::CapabilitiesChanged` at the next idle frame.
- [x] 5.6 Add `/skills` to `/help` and the command palette through the one
  registry.
- [x] 5.7 Add `skills_list_groups_by_layer_and_names_the_winner`.
- [x] 5.8 Add `skills_list_reports_discovery_problems`.
- [x] 5.9 Add `skills_trust_shows_path_and_digest_before_recording`.
- [x] 5.10 Add `skills_trust_recomposes_only_when_idle`.
- [x] 5.11 Add `skills_trust_unknown_name_lists_known_names`.

## 6. Documentation and verification

- [x] 6.1 Extend `docs/skills.md` with the on-disk layout, the frontmatter
  grammar, the bounds, and the trust rule.
- [x] 6.2 Extend `docs/configuration.md` with the skills directories.
- [x] 6.3 Extend `docs/security.md` with project skills as trusted content.
- [x] 6.4 Run `scripts/ci.sh` and record the result. *(exit 0: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and the pinned `agent-runtime-testkit` suite all pass.)*
