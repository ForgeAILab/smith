"""Deterministic, resumable serial launcher for the Luna Max dev ablation."""

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

from smith_harbor.constants import ARTIFACT_MANIFEST_PATH, default_jobs_dir
from smith_harbor.profiles import (
    ProfileError,
    load_profile,
    resume_job,
    run_invariants,
    run_profile,
)
from smith_harbor.variants import load_variant, variant_provenance

EXPERIMENT_ID = "smith-luna-max-dev-completion-policy-v1"
ROUND_ORDER = (
    ("current", "artifact-first-v1", "artifact-first-v1-no-delegation"),
    ("artifact-first-v1", "artifact-first-v1-no-delegation", "current"),
    ("artifact-first-v1-no-delegation", "current", "artifact-first-v1"),
)


def _artifact_manifest() -> dict[str, Any]:
    value = json.loads(ARTIFACT_MANIFEST_PATH.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ProfileError("Smith artifact manifest must be a JSON object")
    return value


def job_name(round_number: int, variant_name: str) -> str:
    short = {
        "current": "current",
        "artifact-first-v1": "artifact-first",
        "artifact-first-v1-no-delegation": "artifact-first-no-delegation",
    }[variant_name]
    return f"smith-luna-max-dev-ablation-r{round_number}-{short}"


def experiment_manifest() -> dict[str, object]:
    profile = load_profile("dev")
    cells = []
    for round_number, variants in enumerate(ROUND_ORDER, start=1):
        for position, variant_name in enumerate(variants, start=1):
            variant = load_variant(variant_name)
            cells.append(
                {
                    "round": round_number,
                    "position": position,
                    "variant": variant_name,
                    "job_name": job_name(round_number, variant_name),
                    "expected_trials": len(profile.tasks),
                    **variant_provenance(variant),
                }
            )
    common_invariants = run_invariants(profile, concurrency=1)
    return {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "profile": "dev",
        "task_count": len(profile.tasks),
        "task_set_sha256": common_invariants["task_set_sha256"],
        "environment_backend": common_invariants["environment_backend"],
        "docker_context": common_invariants["docker_context"],
        "rounds": 3,
        "rollouts_per_cell": 1,
        "oauth_concurrency": 1,
        "expected_jobs": 9,
        "expected_trajectories": 234,
        "upload_allowed": False,
        "smith_artifacts": _artifact_manifest(),
        "cells": cells,
    }


def write_manifest(path: Path) -> dict[str, object]:
    expected = experiment_manifest()
    if path.exists():
        try:
            existing = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise ProfileError("existing experiment manifest is unreadable") from exc
        if existing != expected:
            raise ProfileError("existing experiment manifest does not match frozen inputs")
        return expected
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(expected, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)
    return expected


def _trial_result_count(job_dir: Path) -> int:
    return sum(1 for path in job_dir.glob("*/result.json") if path.parent != job_dir)


def job_complete(job_dir: Path, expected_trials: int) -> bool:
    try:
        result = json.loads((job_dir / "result.json").read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return False
    return (
        isinstance(result, dict)
        and isinstance(result.get("finished_at"), str)
        and result.get("n_total_trials") == expected_trials
        and _trial_result_count(job_dir) == expected_trials
    )


def _validate_existing(job_dir: Path, variant_name: str) -> None:
    try:
        provenance = json.loads(
            (job_dir / "smith-harbor-provenance.json").read_text(encoding="utf-8")
        )
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ProfileError(f"existing job {job_dir.name!r} lacks valid Smith provenance") from exc
    expected = run_invariants(
        load_profile("dev"), concurrency=1, variant=load_variant(variant_name)
    )
    if not isinstance(provenance, dict) or provenance.get("run_invariants") != expected:
        raise ProfileError(f"existing job {job_dir.name!r} does not match frozen invariants")
    if provenance.get("smith_artifacts") != _artifact_manifest():
        raise ProfileError(f"existing job {job_dir.name!r} uses a different Smith artifact")


def run_experiment(*, manifest_path: Path, jobs_dir: Path | None = None) -> int:
    jobs_root = (jobs_dir or default_jobs_dir()).expanduser().resolve()
    manifest = write_manifest(manifest_path)
    cells = manifest["cells"]
    assert isinstance(cells, list)
    for raw_cell in cells:
        assert isinstance(raw_cell, dict)
        variant_name = str(raw_cell["variant"])
        name = str(raw_cell["job_name"])
        expected_trials = int(raw_cell["expected_trials"])
        job_dir = jobs_root / name
        if job_dir.exists():
            _validate_existing(job_dir, variant_name)
            if job_complete(job_dir, expected_trials):
                continue
            code = resume_job(job_dir)
        else:
            code = run_profile(
                "dev",
                job_name=name,
                jobs_dir=jobs_root,
                variant_name=variant_name,
            )
        if code != 0 or not job_complete(job_dir, expected_trials):
            return code or 1
    return 0
