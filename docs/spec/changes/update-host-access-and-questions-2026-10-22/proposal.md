---
created_at: 2026-10-23T00:58:55Z
updated_at: 2026-10-23T03:45:07Z
---

## Why
YOLO already permits unrestricted same-user shell access, but filesystem tools reject external paths before approval. This prevents a session from seeing user-supplied reference images without copying them into its project.

## What Changes
- External filesystem requests reach the normal permission surface. `--yolo` / `--approval allow-all` authorizes them automatically; relative paths still resolve against the project.
- External image reads return validated image content for multimodal providers; the vision fallback accepts the same authorized paths.
- Preserve handle-relative project operations, profile tool restrictions, bounded reads, and version-checked edits.
- Always permit custom answers to agent questions and offer a separate discussion action that submits user-authored chat without choosing an answer.

## Approval
The user approved this scope in this conversation: “yep, we need to give tool unrestricted access. as it has shell so it can basically read anything already. but yep need to read image correctly?”

The user additionally requested permission prompts instead of direct outside-path rejection, plus custom answers and discussion on questions.

## Impact
- Affected specs: tool-execution, client-surfaces, client-interaction.
- Affected code: smith-host workspace, smith-tools path/image handling, smith-runtime composition, questionnaire policy, TUI interaction, and tests.
