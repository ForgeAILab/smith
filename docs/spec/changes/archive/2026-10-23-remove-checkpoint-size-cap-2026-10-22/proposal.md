---
created_at: 2026-10-23T02:59:56Z
updated_at: 2026-10-23T03:04:51Z
---

## Why
Image-heavy sessions can exceed the fixed 32 MiB protected-checkpoint limit even when their provider requests fit the model's context budget. The limit was introduced with protected checkpoint storage and is independent of provider limits.

## What Changes
- Remove the fixed payload and envelope size ceilings from protected checkpoint writes and reads.
- Reuse the existing uncapped owner-only file reader without changing the encrypted envelope format, authentication, checkpoint validation, or atomic replacement.
- Add a regression test that saves and reopens an image-containing checkpoint larger than 32 MiB.

## Approval
The user approved removing this cap in this conversation: “can we drop it? we do not need cap”. This change implements that requested scope.

## Impact
- Affected specs: session-recovery.
- Affected code: `crates/smith-runtime/src/checkpoint.rs`.
- Large checkpoints remain subject to available memory and filesystem capacity. This change does not optimize checkpoint serialization or context estimation.
