# Improvement review after v0.7.2 (2026-10-19)

Two independent reviews of `c534dc6` (v0.7.2), merged and deduplicated:

1. **Firsthand Smith session.** A Smith agent did real work — cloned and
   reviewed `google-research/harness2`, ran its offline demo — and then
   reviewed this repository from a sibling workspace. Findings marked
   *session-observed* come from friction the agent hit while running under
   Smith's current tool defaults.
2. **External model review.** Claude Opus (Claude Code CLI 2.1.303,
   `--effort medium`, read-only tools) reviewed the repository against the
   same question: the five highest-leverage next improvements. Its file:line
   citations were spot-checked against the tree before being included.

In-flight change specs already cover project memory, session fork, the native
Gemini provider, and daily model sync; everything below is outside those.

## Findings, ranked

### 1. `read` results are offloaded at 8 KB, defeating the tool's own bounds

*Session-observed, worst friction of the session.*

- `context.tool_output_inline_bytes` defaults to `8 * 1024`
  (`crates/smith-config/src/resolve/load.rs:614`). Anything larger is
  offloaded to the artifact store with a text preview capped at 1200 chars,
  and `artifact_page_bytes` is `inline/4` clamped to 4096
  (`crates/smith-runtime/src/tool_output.rs:34-61`).
- In the session, a 209-line README (~11 KB serialized) came back as a stub
  plus an artifact id. Recovering it inline meant `artifact.read` pages of
  <=2 KB — up to six model round trips — plus two more if the reader tool is
  not yet staged. The agent routed around the mechanism with `sed` through
  `shell`, which silently degrades model behavior and spends approval
  authority on reads.
- `read` is already self-bounded (`DEFAULT_LINE_LIMIT = 2000`,
  `crates/smith-tools/src/read.rs:24`). Offloading a bounded read rarely
  saves the window it costs in retrieval turns.

First step: exempt the bounded read-only trio (`read`, `list`, `search`) from
offload, or raise the default to 32-64 KB and keep offload for genuinely
unbounded output (`shell`). Suggested change id:
`fix-tool-output-offload-reads`.

### 2. `shell` is unsandboxed, so every build or test is a prompt or `--yolo`

- `crates/smith-tools/src/shell.rs:62,95,187` prepares `shell` as a
  deliberately unsandboxed same-user host shell. Unless the run is
  `allow-all`, every invocation reaches the approval policy; headless
  mutation in `ask` posture fails closed with exit 4.
- The most routine agent work — running a test suite, a build, a linter — is
  therefore the most frequently interrupted. That pressure is exactly what
  pushes users toward disabling approvals.

First step: spec a `sandboxed-shell` resource kind (workspace read/write, no
network by default; macOS Seatbelt, Linux Landlock) alongside `host-shell`,
auto-allowed in `build` posture. Suggested change id: `add-sandboxed-shell`.

### 3. `search` is literal-substring only and sequential

- `crates/smith-tools/src/search.rs:45-75,144-198`: the pattern is matched
  verbatim; the only filter is a single extension; there are no context
  lines; files are read one at a time, each in its own `spawn_blocking`,
  across up to 100k entries.
- This is the agent's primary navigation tool. Missing regex, globs, and
  gitignore-aware traversal shows up in every exploring turn.

First step: swap the inner loop for `grep-searcher` + `ignore` behind the
same prepared `FsRead` permission; add `regex`, `glob`, and `context`
arguments. Suggested change id: `upgrade-search-regex`.

### 4. Single-root workspace turns safe sibling reads into shell calls

*Session-observed; security inversion, not just friction.*

- `read` refuses paths outside the project root
  (`crates/smith-tools/src/read.rs:59`), and there is no
  `workspace.additional_roots` option anywhere in `smith-config`.
- When the session was asked to look at `../tui`, the *safe* operation —
  reading a sibling repository — was reachable only through the
  *maximum-authority* tool: an unsandboxed host shell with inherited
  credentials, network, and egress. The boundary currently widens authority
  instead of narrowing it.

First step: `workspace.additional_roots` config that grants `FsRead`-scoped
access to named sibling paths, with the boundary still enforced in one
place. Suggested change id: `add-workspace-additional-roots`.

### 5. `edit` carries one operation per call

- `crates/smith-tools/src/edit.rs:63-66,114-125`: each call takes one
  `replace`/`create`/`overwrite`/`delete` (plus `replace_all`). An N-spot
  refactor costs N model round trips and N approval prompts, and a partial
  failure leaves the file half-edited.

First step: an `edits: [{old_string,new_string,replace_all}]` array scoped to
one file, prepared as a single fingerprinted authority and applied
all-or-nothing. Suggested change id: `add-atomic-multi-edit`.

## Deferred items with verified evidence

- **Two-way headless protocol.** `docs/GOAL.md:94-98` lists a bidirectional
  headless interaction protocol as deferred; `docs/headless-protocol.md:298`
  shows a pending question ending the run (`interaction_required`, exit 5)
  and approvals in `ask` mode ending with exit 4. Embedders, IDEs, and SDK
  callers cannot keep a session going across an interaction.
- **Plugin hooks never run.** `docs/plugins.md:157`: "Hooks do not run."
  Bundles are installed and digest-verified, but `PreToolUse`/`PostToolUse`
  hooks — format guards, lint-on-edit, policy enforcement; the main reason
  plugins ship — are only listed.
- **`docs/GOAL.md` is stale.** It still describes the v0.0.1 goal and the
  `../agent-runtime` sibling-path pin. Cheap fix, low leverage.

## Method notes

- The first external review attempt was killed at a 10-minute deadline with
  no output: `claude -p` buffers until completion, so a deadline kill loses
  everything. The retry used `--effort medium` and completed in ~4 minutes.
- The external reviewer emitted one warning about this repository's
  `.claude/settings.local.json`: an allow rule of the form
  `Bash(rg -rn --glob '*.rs' ... *)` has a leading wildcard that also
  approves options inserted at that position. Worth tightening independently.
