---
created_at: 2026-11-27T20:40:00Z
updated_at: 2026-11-27T20:40:00Z
---

## Why

Connecting Anthropic with `/connect` on an existing install replaced the
owner's `default_profile` (`code` → `claude`) and their seven-profile
`profile_order` with `[claude, claude-plan, claude-review]`. Choosing "Keep
current default" made no difference, because the quick-key plans for xAI,
Google, and Anthropic call `select_default` unconditionally. The spec already
says setup changes the default only when the user confirms it.

The same connect then showed two text-only screens: a summary review, then a
collision review listing the values it was about to replace. Both only
accept Enter. For a catalog connection whose only typed input is an API key,
the summary adds nothing, and the collision review only fired because of the
bug. The empty key field also gave no hint about what to type.

## What Changes

- Quick-key submissions (xAI, Google, Anthropic) carry the user's default
  choice. "Keep current default" writes only the provider section and its
  credential. The catalog keeps its models selectable through `/model`.
- When setup makes something the default, it adds the new profiles to the
  end of an existing user `profile_order` instead of replacing it. When the
  user file has no `profile_order`, setup leaves it unset, since the derived
  order already lists every main profile.
- First-run setup always makes its connection the default and skips the
  default choice, because an empty install has no default to keep.
- In-session catalog key connections (`/connect anthropic`, `/connect
  google`) write right after the default choice, with no summary review. The
  collision review still appears when the write would replace existing values.
- In a session, a setup review is a numbered choice (save, or go back), not
  text that only accepts Enter. Standalone `smith setup` keeps its review.
- An empty API-key field shows a placeholder saying to paste the key.

## Impact

- Code: `smith-tui` setup reducer and rendering, `smith-cli` setup plan,
  transaction, and completion summary.
- Specs: `configuration` (additive setup) and `client-surfaces` (in-session
  connection).
- Users who already connected with the bug keep their current config; this
  change does not migrate it.
