# Skills

A skill is a named, bounded instruction body with a searchable descriptor.
Smith indexes descriptors first and materializes a body only when the agent
activates that skill, so an unused skill costs its descriptor, not its text.
Activation contributes instructions only: a skill can guide behavior but can
never grant a tool, permission, approval, credential, executable trust, or a
wider workspace.

## Source layers and precedence

Skill declarations resolve in one deterministic order, lowest precedence
first:

1. built-in — compiled into Smith;
2. user — installed in the user's Smith profile;
3. workspace — supplied by the active project, gated by project trust;
4. session — supplied explicitly by the embedding host.

A higher layer shadows a lower one by exact name. Duplicate names inside one
layer are an error. Untrusted, changed, or denied workspace declarations
remain visible as bounded index metadata but are never activatable and cannot
shadow an admitted lower-precedence skill.

## Where skills live

Smith reads skills from a fixed layout:

```
~/.smith/skills/<name>/SKILL.md              # user layer
<project>/.smith/skills/<name>/SKILL.md      # workspace layer
```

The **directory name is the skill's name**. Frontmatter cannot choose it: a
name that can only be read by opening the body cannot appear in an index built
without opening bodies, and a project skill able to name itself could aim its
shadowing at a user skill the reader never sees named in the path. A `name`
field is accepted when it matches the directory exactly and is reported as a
problem when it does not, so a skill authored for another tool loads without
being able to redirect.

Neither directory is created by Smith, and neither location is configurable.

### The file

```markdown
---
name: rust-review
description: Review Rust implementation boundaries before changing unsafe code.
---

Read the unsafe blocks first, then the trait boundaries they cross.
```

The frontmatter grammar is deliberately narrower than YAML: an opening `---`,
then `key: value` lines, then a closing `---`. Only `name` and `description`
are interpreted; a `description` is required. Keys another tool writes —
`license`, `allowed-tools`, and so on — are ignored and not retained, so no
file-authored key reaches a descriptor.

Retrieval keywords are derived from the name and the description, so the
description is what decides when a skill is offered. Write it as the task it
answers, not as a title.

### Bounds

| Bound | Value |
| --- | --- |
| Skills per layer | 256 |
| `SKILL.md` size | 1 MiB |
| Frontmatter lines | 64 |
| `description` length | 1024 characters |

A skill Smith cannot use is excluded and reported by name in `/skills`; it is
never a reason to refuse to start. A directory with no `SKILL.md`, and a loose
file beside the skill directories, are skipped without comment — they are not
broken skills, they are not skills.

### Content pinning

Discovery digests each `SKILL.md` and pins the declaration to those exact
bytes, in every layer. A body rewritten after the session started — by a pull,
by another process, by the agent itself — fails closed at activation instead of
entering context unreviewed. Building the index still opens no body.

### Project skills need trust

A project `SKILL.md` is repository content asking to write instructions into
the model's context, so it is gated by the same hash-bound project trust as a
hook or an MCP server. An unapproved, edited, or declined project skill stays
visible in the index, never activates, and never shadows a user or built-in
skill of the same name. A `SKILL.md` that resolves outside the project root — a
symlink aimed elsewhere — is refused outright rather than covered by the
project's trust.

`/skills` lists the catalog and the reasons; `/skills trust NAME` shows the
project-relative path and the content digest, records the decision, and the
session picks the skill up at the next idle boundary without restarting.

## Built-in harness references

Smith ships one built-in skill per shipped reference document. Each body is
embedded at compile time, so the activated instructions are byte-identical to
the documentation at the revision the binary was built from and are available
in any workspace, offline:

| Skill | Embedded document | Covers |
| --- | --- | --- |
| `smith.configuration` | `docs/configuration.md` | Config layering, profiles, providers, credentials, model limits, reasoning controls, policy keys, environment variables, CLI flags |
| `smith.headless` | `docs/headless-protocol.md` | `smith -p` input modes, output formats, event framing, non-interactive resume |
| `smith.persistence` | `docs/persistence-recovery.md` | Session snapshots, journals, protected checkpoints, resume and recovery |
| `smith.security` | `docs/security.md` | Trust boundaries, approvals, credential handling, why text is not authority |

The interactive TUI and `smith -p` compose the same built-in set through the
shared `smith-runtime` factory. A direct embedder that supplies its own skill
sources replaces the set entirely and receives no implicit built-in entries.
A user, trusted-workspace, or session declaration that reuses a built-in name
shadows the shipped body while the built-in entry stays in the index.
