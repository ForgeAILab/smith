"""Versioned Smith composition variants for the completion-policy ablation."""

from __future__ import annotations

import dataclasses
import hashlib

ARTIFACT_FIRST_POLICY = (
    "Create every required deliverable at its exact path early in the run, then\n"
    "refine it. Treat the final bytes on disk as the source of truth. After the last\n"
    "edit, reread the deliverable and run an independent check derived only from the task\n"
    "instruction. For example-based transformations, replay the inferred rule\n"
    "against every training example before applying it. Reserve the final portion\n"
    "of the task budget for validation and leave the best complete artifact in\n"
    "place if time becomes uncertain. Report success only after checks against the\n"
    "final artifact pass; otherwise report the remaining failure honestly."
)
ARTIFACT_FIRST_REVISION = "artifact-first-v1"
NO_POLICY_REVISION = "none"
NO_POLICY_DIGEST = hashlib.sha256(b"").hexdigest()


@dataclasses.dataclass(frozen=True)
class Variant:
    name: str
    instructions: str | None
    delegation: bool
    policy_revision: str
    policy_sha256: str
    comparison_axis: str


_POLICY_DIGEST = hashlib.sha256(ARTIFACT_FIRST_POLICY.encode()).hexdigest()
VARIANTS = {
    "current": Variant("current", None, True, NO_POLICY_REVISION, NO_POLICY_DIGEST, "baseline"),
    "artifact-first-v1": Variant(
        "artifact-first-v1",
        ARTIFACT_FIRST_POLICY,
        True,
        ARTIFACT_FIRST_REVISION,
        _POLICY_DIGEST,
        "completion_policy",
    ),
    "artifact-first-v1-no-delegation": Variant(
        "artifact-first-v1-no-delegation",
        ARTIFACT_FIRST_POLICY,
        False,
        ARTIFACT_FIRST_REVISION,
        _POLICY_DIGEST,
        "delegation",
    ),
}


def load_variant(name: str) -> Variant:
    try:
        return VARIANTS[name]
    except KeyError as exc:
        raise ValueError(f"unknown Smith Harbor variant {name!r}") from exc


def variant_provenance(variant: Variant) -> dict[str, object]:
    return {
        "variant": variant.name,
        "effective_delegation": variant.delegation,
        "policy_revision": variant.policy_revision,
        "policy_sha256": variant.policy_sha256,
        "comparison_axis": variant.comparison_axis,
    }
