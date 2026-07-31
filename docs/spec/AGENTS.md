# Spec Instructions

## TL;DR

- Stage 1: write `docs/spec/changes/<id>/` (proposal + tasks + deltas), validate,
  and get approval.
- Stage 2: implement tasks sequentially without scope creep.
- Stage 3: archive after shipping and update truth specs.

## Delta rules

- Delta files live under
  `docs/spec/changes/<id>/specs/<capability>/spec.md`.
- The first non-empty line MUST be
  `## ADDED Requirements`, `## MODIFIED Requirements`,
  `## REMOVED Requirements`, or `## RENAMED Requirements`.
- Each `### Requirement:` MUST include descriptive text before its scenarios.
- Each requirement MUST include at least one scenario using
  `#### Scenario:` with exactly four hashes.
