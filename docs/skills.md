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
