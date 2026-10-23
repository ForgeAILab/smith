## Context
The project workspace owns handle-relative filesystem operations. The shell already has unrestricted same-user host authority under allow-all.

## Decisions
Keep the project root as the logical cwd. At runtime composition, enable a host filesystem capability alongside the project capability. External I/O uses the host capability after approval; project operations retain their project handle. All I/O continues with the existing byte limits and version checks. Standalone workspaces remain confined until host access is enabled.

Prepared external calls identify their actual filesystem mount and canonical path. Existing central authority routes off-project filesystem resources through approval; interactive runs prompt and allow-all authorizes them automatically. A profile still controls which tools are registered. Child sessions share the resolved workspace authority. WASM mounts and project instruction trust remain project-scoped.

Image ingestion uses the same validation, resizing, and typed image payload as project images. A text-only main model uses its configured vision route, whose path resolver shares the authorized workspace.

## Validation
Use temporary project and host directories. Exercise outside reads, listing/search, create/replace/delete, relative paths, and symlinks. Confirm a fake image-capable provider receives an actual image block on its follow-up request. Keep confinement and read-only posture regression coverage.

## Questions
A Smith wrapper around the shared questionnaire tool always enables free-form answers, including on restored prepared calls, so the broker can validate custom responses. The TUI adds an Other answer field and a Chat about this question action. Discussion uses a separate draft, Esc returns to the question without losing staged answers, and Enter sends the typed text as ordinary active-turn input while declining the unanswered questionnaire. Sensitive discussion text is registered with the same redactor before the broker resumes. Existing composer drafts are retained.
