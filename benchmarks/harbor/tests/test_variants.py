from __future__ import annotations

import json
import tomllib
from pathlib import Path

from smith_harbor.constants import smith_config_toml
from smith_harbor.experiment import ROUND_ORDER, experiment_manifest, job_complete
from smith_harbor.profiles import job_config, load_profile
from smith_harbor.variants import ARTIFACT_FIRST_POLICY, VARIANTS

EXPECTED_POLICY = """Create every required deliverable at its exact path early in the run, then
refine it. Treat the final bytes on disk as the source of truth. After the last
edit, reread the deliverable and run an independent check derived only from the task
instruction. For example-based transformations, replay the inferred rule
against every training example before applying it. Reserve the final portion
of the task budget for validation and leave the best complete artifact in
place if time becomes uncertain. Report success only after checks against the
final artifact pass; otherwise report the remaining failure honestly."""


def test_variants_are_versioned_generic_profile_fragments(tmp_path: Path) -> None:
    profile = load_profile("dev")
    task_names = {task.name for task in profile.tasks}
    assert ARTIFACT_FIRST_POLICY == EXPECTED_POLICY
    assert not any(task in ARTIFACT_FIRST_POLICY for task in task_names)
    assert "/logs" not in ARTIFACT_FIRST_POLICY
    assert "verifier" not in ARTIFACT_FIRST_POLICY.lower()
    assert "expected answer" not in ARTIFACT_FIRST_POLICY.lower()

    for name, variant in VARIANTS.items():
        parsed = tomllib.loads(
            smith_config_toml(
                profile_instructions=variant.instructions,
                delegation=variant.delegation,
            )
        )
        harbor = parsed["profiles"]["harbor"]
        assert harbor["delegation"] is variant.delegation
        assert harbor.get("instructions") == variant.instructions
        config = job_config(
            profile,
            job_name=f"test-{name}",
            jobs_dir=tmp_path,
            variant=variant,
        )
        rendered = json.dumps(config)
        assert ARTIFACT_FIRST_POLICY not in rendered
        assert "auth.json" not in rendered


def test_experiment_manifest_freezes_cyclic_order_without_task_names() -> None:
    manifest = experiment_manifest()
    cells = manifest["cells"]
    assert manifest["expected_trajectories"] == 234
    assert [
        tuple(cell["variant"] for cell in cells if cell["round"] == round_number)
        for round_number in range(1, 4)
    ] == list(ROUND_ORDER)
    rendered = json.dumps(manifest)
    assert not any(task.name in rendered for task in load_profile("dev").tasks)


def test_job_completion_requires_all_trial_results(tmp_path: Path) -> None:
    job = tmp_path / "job"
    job.mkdir()
    (job / "result.json").write_text(
        json.dumps({"finished_at": "2026-08-08T00:00:00Z", "n_total_trials": 2}),
        encoding="utf-8",
    )
    for name in ("a", "b"):
        trial = job / name
        trial.mkdir()
        (trial / "result.json").write_text("{}", encoding="utf-8")
    assert job_complete(job, 2)
    (job / "b" / "result.json").unlink()
    assert not job_complete(job, 2)
