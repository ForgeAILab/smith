---
created_at: 2026-08-20T20:28:10Z
updated_at: 2026-08-20T20:28:10Z
---

# Design

## On-disk layout

```
~/.smith/skills/<name>/SKILL.md              # user layer
<project>/.smith/skills/<name>/SKILL.md      # workspace layer
```

A directory per skill, not a flat file, because a skill's supporting assets
will live beside its body in a later change and moving the body afterwards
would invalidate every recorded trust decision.

**The directory name is the skill's identity.** Frontmatter cannot choose it.
Two reasons: a name only readable by opening the file cannot be shown in a
bounded index built without opening files, and a project skill that could name
itself would be able to aim its shadowing at a user skill the reader never sees
named in the path. A frontmatter `name` is accepted when it matches the
directory exactly and is a discovery problem when it does not — this keeps
skills authored for other tools loadable without letting them redirect.

Names are validated by the existing `skills::validate_name`: 1..=96 ASCII
letters, digits, `.`, `_`, or `-`. A directory whose name fails that is a
discovery problem, not a silent skip: a user who names a directory `my skill`
must be told why it did not appear.

Entries in `skills/` that are not directories are skipped silently, so a
`README.md` beside the skills is not an error. A directory with no `SKILL.md`
is likewise skipped silently — it is not a skill.

## Frontmatter

```
---
name: rust-review
description: Review Rust implementation boundaries before changing unsafe code.
---

<instruction body>
```

A strict, dependency-free parser: the first line must be exactly `---`, then
`key: value` lines until a closing `---`. Keys are lowercase ASCII with `-`;
the value is the rest of the line, trimmed. Only `name` and `description` are
interpreted. Other keys — `license`, `allowed-tools`, whatever another tool
writes — are ignored and not retained, so no file-authored key/value reaches a
descriptor. `description` is required and bounded to 1024 characters; it is the
only text retrieval scores against, and an unbounded one would be a free slot
in every index.

No YAML dependency is added. The grammar is deliberately narrower than YAML so
that a file which parses here parses the same way everywhere, and so a skill
cannot smuggle structure through a YAML feature.

## Bounds

| Bound | Value | Why |
| --- | --- | --- |
| Skills per layer | 256 | A pathological directory cannot make host start unbounded |
| `SKILL.md` bytes | 1 MiB | Read cap, independent of the runtime's later context bound |
| Frontmatter lines | 64 | Parsing is bounded before `description` is found |
| `description` chars | 1024 | Bounds the indexed card |

Exceeding a bound is a discovery problem for that skill. Exceeding the
per-layer count stops discovery in that layer and reports how many were
dropped; a silent truncation would read as "that is all there is".

## Digest pinning

Discovery reads `SKILL.md`, digests it, and declares
`Skill::from_verified_file(name, description, path, sha256)`. Two properties
follow:

- Descriptor resolution still opens nothing. The digest comes from discovery,
  which has already read the file; the descriptor is built from the name,
  description, and that digest.
- Activation proves it loaded the exact bytes discovery saw. A body rewritten
  after the session started — by a `git pull`, by another process, by the agent
  itself — fails closed at activation rather than entering context unreviewed.

This applies to the user layer too. A user skill is trusted content, but
"trusted" is a statement about the bytes the user wrote, not about whatever the
file holds three hours later.

## Workspace trust

For each discovered workspace skill:

```rust
let executable = Executable::from_file(project, ExecutableKind::Skill, &path)?;
let status = trust.status(project, &executable)?;
sources = sources.with_workspace(skill, status);
```

`Executable::from_file` is reused unchanged and supplies three properties this
path needs: the label is the project-relative path, so a rewritten skill is
recognizably the same skill with a new digest; the digest is over the file's
bytes; and a `SKILL.md` that canonicalizes outside the project root — a symlink
in `.smith/skills/` aimed at `~/.ssh/config`, or at a user skill it wants to
launder into the workspace layer — is refused rather than covered by the
project's trust.

The digest the trust decision binds and the digest the activation pin checks
are the same value, computed once. A decision therefore cannot authorize one
body while activation loads another.

`ExecutableKind::Skill` joins the existing kinds. The enum's doc comment says
Smith will not *run* project-supplied authority unasked; a skill body is not
run, so the comment is extended rather than stretched: privileged instructions
are authority Smith exercises on the project's behalf, and the reason plain
declarative settings have no variant — they carry no execution and gating them
would make Smith prompt for a model name — does not cover text that becomes
part of the system's instructions.

Adding a variant is backward compatible for the persisted `trust.json`: older
files never contain it, and `TrustStatus` for an unrecorded artifact is already
`Untrusted`.

## Composition

`RuntimeRequest::new` keeps `built_in_sources()` unchanged, so a direct
embedder that supplies its own sources is still unaffected and the built-in set
is still what a request starts from. The CLI host — the one place that knows
the user state root, the project root, and the trust store — folds discovered
declarations onto that base in `start_host`, and the headless path uses the
same helper so `smith -p` and the TUI index one catalog.

Discovery problems are carried on the returned value, not logged: `/skills` is
where a user looks for them, and a problem that only reaches a log is the
failure this change exists to prevent.

## `/skills`

Modeled on `/mcp`, which solves the same shape of problem.

```
/skills              list every indexed entry and every discovery problem
/skills trust NAME   show path and digest, record the decision, recompose
```

The list renders `Smith::skill_index()` grouped by layer, in the layer order
precedence uses. Each row carries the name, the description, and a state:
activatable, shadowed by a higher layer, or the trust reason it is withheld —
`untrusted · needs approval — run /skills trust NAME`, `untrusted · its content
changed`, `refused · you declined it`. A shadowed entry stays visible because
"which layer won" is the question the index exists to answer.

`/skills trust NAME` renders the same confirmation shape `/mcp trust` does — the
project-relative path and the content digest the decision binds — records
`TrustDecision::Allow`, and sets a pending-recompose flag. The TUI breaks with
`InteractiveExit::CapabilitiesChanged` at the next idle frame, exactly as it
does for a newly connected MCP server, so the session keeps its identity and
transcript while the catalog is rebuilt around the newly admitted skill.
Recomposition is required because the resolved catalog is baked into the
runtime at factory build; it happens only at an idle boundary because swapping
the ability set under a running turn is what the epoch rules exist to prevent.

Headless `smith -p` has no trust-granting surface, matching MCP: an untrusted
workspace skill stays indexed and inert, and no prompt is issued.

## Module placement

`smith-config` cannot host discovery: it does not depend on
`agent-runtime-ability` and so cannot name `Skill`. Discovery therefore lives in
`smith-runtime` beside the policy it feeds, as `skills.rs` plus a private
`skills/discovery.rs`, following the existing `resolve.rs` + `resolve/` shape.
The public path stays `smith_runtime::skills::…`, so the facade keeps its
compatibility exports as `code-organization` requires.
