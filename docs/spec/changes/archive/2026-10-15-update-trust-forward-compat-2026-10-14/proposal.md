---
created_at: 2026-10-14T12:55:57Z
updated_at: 2026-10-16T01:30:42Z
---

## Why

Every Smith binary on a machine shares one `~/.smith/trust.json`, and the
binaries do not upgrade together. When file commands added the
`slash_command` kind, a build from main recorded one such decision, and the
released 0.3.10 then refused to start in every project:

```
smith: discovering skills: Config: the trust file `~/.smith/trust.json` is not
readable as trust decisions: unknown variant `slash_command`, expected one of
`extension`, `hook`, `credential_helper`, `shell_setting`, `mcp_server`, `skill`
```

The existing "Existing trust files remain readable" scenario covers an old
file read by a new Smith. Nothing covers a new file read by an old Smith, so
the same failure returns each time a kind, decision value, or record field
is added.

## What Changes

- `TrustStore::open` accepts a record it does not understand, provided the
  record is a JSON object with string `kind` and `label` fields. An unknown
  kind, an unknown decision value, or a missing or mistyped field all count
  as not understood.
- A record that is not understood authorizes nothing. Its artifact's status is
  reported as if no decision had been recorded, so it is gated like any other
  undecided artifact.
- Every record this Smith does not replace is written back with the same
  fields and values when the file is persisted, including records it did not
  understand. This keeps an older Smith from erasing a newer Smith's
  decisions the first time it records one of its own.
- A new decision replaces any existing record with the same `kind` and
  `label` strings, whether or not that record was understood. A kind and
  label pair never has two records.
- Forgetting one artifact removes every record with its `kind` and `label`,
  whether or not it was understood. Forgetting a project removes all of that
  project's records.
- The following stay fatal, as now: a file that is not JSON, a `version` other
  than 1, a project entry that is not a list, and a record that is not an
  object with string `kind` and `label` fields.

## Out of Scope

- 0.3.10 and earlier, which are already released. The fix applies from the
  next release onward.
- Showing ignored records in the TUI or in `/commands`.
- Field order and whitespace. The persisted file keeps fields and values but
  not formatting.
- Changing `SCHEMA_VERSION` or migrating existing files. The files in use are
  already compatible.

## Impact

- Affected specs: `configuration`
- Affected code: `crates/smith-config/src/trust.rs` (`TrustFile`,
  `TrustStore::{open, status, record, records, forget, forget_executable,
  persist}`, `find`)
