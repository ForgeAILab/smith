---
name: smith-profiles
description: Discover which Smith agent profiles are configured on this machine and choose the right one for a dispatch. Load before passing --profile, or when a dispatch needs a posture stronger than --approval deny.
---

# Choosing a Smith profile

A profile is a named bundle of provider, model, reasoning effort, posture, and
system instructions. `--profile <NAME>` selects one for a dispatch. Profiles are
the user's own configuration — they differ on every machine, so **discover them,
never assume the names in this document exist**.

## Discover what is configured

One call. It resolves `extends` chains and marks the default:

```bash
"${CLAUDE_PLUGIN_ROOT}/scripts/smith-profiles"          # aligned table
"${CLAUDE_PLUGIN_ROOT}/scripts/smith-profiles" --json   # same data, machine-readable
```

Run it before any dispatch where the profile matters. It is a local config read
— no API call, no cost, instant.

The equivalent by hand, if the script is unavailable:

```bash
smith config explain profile_order          # every profile name
smith config explain profile                # the default
smith config explain model --profile <NAME> # what one name resolves to
```

### Do not read the credential files

`~/.smith/auth.json` and `accounts.json` hold live OAuth access and refresh
tokens as nested JSON strings, which naive redaction misses. Never read them.

`~/.smith/config.toml` is **not** secret-free either: `[providers.*]` tables
carry plaintext `api_key` values. Read `[profiles.*]` if you need a field the
script omits, but never print a `[providers.*]` table, and never dump the file
or a line range of it. The script exists so you do not have to touch it at all.

## Posture is the real read-only guarantee

Each profile declares a `posture`, and it outranks `--approval`:

| Posture | Meaning |
| --- | --- |
| `build` | Registers write-capable tools (`edit`, `shell`). Needs `--approval allow-all` to actually use them. |
| `plan` | Read-only. Inspects and produces an implementation plan. |
| `review` | Read-only. Produces prioritized, evidence-backed findings. |

`--approval deny` *denies* a write at its approval boundary — the tool is
registered, the call is declined, and the run continues and reports the refusal.
A `plan` or `review` posture never registers `edit` or `shell` **at all**, so
there is nothing to deny. When the user wants a hard read-only guarantee rather
than a refusal, select a read-only profile; that is what `--profile` is for here.

A profile may also carry `use = ["main", "child"]`. `--profile` only accepts a
main-enabled profile.

## Picking one

- **Omit `--profile`** for ordinary dispatches. The user's default is already the
  profile they chose for this project; overriding it silently substitutes your
  judgment for theirs.
- **Read-only investigation, diagnosis, or a plan** → a `plan`-posture profile.
- **Independent review of work already done** → a `review`-posture profile.
- **A second opinion from a different model** → a profile on a different
  provider than the default. That is the whole point of delegating: a genuinely
  independent pass, not the same model twice.
- **Hard reasoning** → a profile declaring a raised `reasoning.effort`.

Name the profile you chose and why when you report back, so the user can correct
the selection rather than re-run the task.

## When a profile fails

A profile that is configured is not necessarily usable — credentials expire and
plans run out of quota. The envelope says which:

- `"error": "Limit: the provider reports this credential's usage window is spent"`
  — that account is exhausted; `account.resets_at_ms` says when it returns.
- `"error": "Provider: You've hit your usage limit..."` — the upstream plan, not
  Smith.
- `"error": "Config: the provider rejected the request"` — a bad or stale key.

Report the failure and name a working alternative. Do **not** silently fall back
to another profile: the user asked for a specific model, and a different one
answering in its place is a different answer wearing the same label.
