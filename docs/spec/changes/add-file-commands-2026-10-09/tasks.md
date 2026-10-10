---
created_at: 2026-10-10T02:49:20Z
updated_at: 2026-10-10T23:04:33Z
completed_at: 2026-10-10T23:04:33Z
---

## 1. Trust kind

- [x] 1.1 Add `ExecutableKind::SlashCommand` (`"slash_command"`) in
  `crates/smith-config/src/trust.rs`; existing trust files load unchanged.
- [x] 1.2 Tests: decision binds path and content; symlink out of the project
  is refused; an old trust file leaves project commands undecided.

## 2. Discovery

- [x] 2.1 Add a discovery module in `smith-client` that reads
  `<root>/commands/*.md` for the user root and `<project>/.smith/`, sorted,
  bounded (256 per layer, 64 KiB per file), creating no directory.
- [x] 2.2 Parse optional frontmatter (`description`, `argument-hint`); fall
  back to the first non-empty body line for the description; bound both.
- [x] 2.3 Validate names (1..=64 of `a-z`, `0-9`, `-`); refuse names that
  collide with a built-in command; report every refusal as a named problem.
- [x] 2.4 Resolve layers: trusted project shadows user; untrusted or changed
  project commands stay listed, do not run, and do not shadow.
- [x] 2.5 Unit tests for each case in 2.1–2.4, including the count and size
  bounds and non-`.md` files being ignored silently.

## 3. Expansion and dispatch

- [x] 3.1 Implement single-pass `$ARGUMENTS` substitution with append when
  the body has no placeholder; unit tests including arguments that contain
  the placeholder text.
- [x] 3.2 Make command lookup two-stage (built-ins, then file commands) for
  `matches`, `parse`, and `has_exact_name`; add a menu row type covering both.
- [x] 3.3 TUI: file commands in slash completion and `Ctrl+P` with layer
  label and argument hint; `Tab` completes, `Enter` runs.
- [x] 3.4 TUI dispatch: re-read the file, check a project command against its
  trusted digest, and build a `PreparedSubmission` whose `display_text` is the
  typed command and whose committed and expanded text are the expansion.
- [x] 3.5 `/help` lists file commands in their own group.

## 4. `/commands`

- [x] 4.1 `/commands`: list by layer with description and state, shadowed
  entries, and discovery problems.
- [x] 4.2 `/commands trust <name>`: show project-relative path and content
  identity in the shared confirmation, record the decision, admit the command
  in the same session.
- [x] 4.3 `/commands reload`: rebuild the catalog at an idle boundary.
- [x] 4.4 Running an untrusted project command names `/commands trust <name>`
  and sends nothing to the provider.

## 5. Headless

- [x] 5.1 `smith -p "/name args"` expands a discovered file command; `//name`
  is sent literally as `/name`; other input is passed through unchanged.
- [x] 5.2 An untrusted or changed project command fails closed with a
  diagnostic on stderr, a non-zero exit, and no provider request.
- [x] 5.3 Fixture tests for 5.1 and 5.2.

## 6. Records and docs

- [x] 6.1 Record each admitted file command as a content-only contribution
  with its layer provenance in composition evidence.
- [x] 6.2 Document authoring in the README or user docs with one example
  file, and add an example under the repository's own `.smith/commands/` if
  one is useful.

## 7. Verification

- [x] 7.1 `cargo test --workspace --no-fail-fast` with
  `TMPDIR=/private/tmp/smith-fixture-tmp`.
- [x] 7.2 Live check in the installed binary: a user command, a project
  command before and after trust, an edited project command, `/commands
  reload`, and `smith -p "/name"`. (Run 2026-10-10 against the
  worktree build `../tui-file-commands/target/debug/smith` with the real
  config, not the installed binary.)
