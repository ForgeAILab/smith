---
created_at: 2026-09-25T16:30:00-04:00
updated_at: 2026-09-25T20:10:47Z
---

# Proposal: Add the latest coding models

## Why

The installed Codex catalog and Smith's refreshed Models.dev cache advertise
GPT-6 Sol and GPT-6 Luna, but Smith's compiled ChatGPT and installed-Codex
model lists stop at GPT-6 Astra. The models therefore work in Codex itself but
do not appear in Smith's model picker. The embedded Models.dev snapshot also
predates these GPT-6 variants and Claude Opus 5.5, so a new offline install
does not discover the same catalog entries as a refreshed install.

Claude Code already accepts rolling `opus`, `sonnet`, `fable`, and `haiku`
aliases. Those aliases should remain Smith's stable Claude CLI choices instead
of adding a dated Claude model identifier that would become stale again.

## What Changes

- Add GPT-6 Sol and GPT-6 Luna to Smith's reviewed direct-ChatGPT model set,
  with the same published 272k default and 872k extended windows as GPT-6
  Astra.
- Add GPT-6 Sol and GPT-6 Luna to the installed Codex model picker.
- Refresh the embedded Models.dev snapshot from Smith's validated normalized
  cache so offline installs include the current OpenAI and Claude entries.
- Retain Claude Code's rolling aliases and test that the latest model remains
  selectable through the stable `opus` alias.
- Release, build, and locally install Smith 0.2.14.

## Impact

- Affected specs: `configuration`, `client-surfaces`
- Affected code: `smith-config` trusted model and CLI-agent catalogs,
  `smith-runtime` embedded Models.dev snapshot, release metadata, tests, and
  model documentation
- Existing profiles and model identifiers remain compatible; all additions
  are additive

## Approval Boundary

The user's request to update and release Smith authorizes the additive model
catalog changes, snapshot refresh, version bump, tests, commit, local install,
and built-in Codex/Claude CLI updater commands. It does not authorize changing
provider credentials, pushing or tagging a remote release, or inventing a
custom gateway route for Claude Opus 5.5.
