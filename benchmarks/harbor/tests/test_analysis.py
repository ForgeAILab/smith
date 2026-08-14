from __future__ import annotations

import json
from pathlib import Path

import pytest

from smith_harbor.analysis import (
    AnalysisError,
    LoadedJob,
    TrialObservation,
    bootstrap_interval,
    compare_jobs,
    load_job,
    render_markdown,
)


def _job(name: str, rewards: list[float], *, tokens: float | None = 100.0) -> LoadedJob:
    trials = tuple(
        TrialObservation(
            task=f"task-{index}",
            reward=reward,
            tokens=tokens,
            latency_seconds=10.0,
            reported_success=reward > 0,
            verifier_success=reward > 0,
            failed=False,
        )
        for index, reward in enumerate(rewards)
    )
    return LoadedJob(Path(name), name, {"dataset": "frozen"}, trials)


def test_bootstrap_is_deterministic_and_claims_follow_interval() -> None:
    first = bootstrap_interval([1.0, 2.0, 3.0], seed=7)
    second = bootstrap_interval([1.0, 2.0, 3.0], seed=7)
    assert first == second

    baseline = _job("baseline", [0.0, 0.0, 0.0])
    candidate = _job("candidate", [1.0, 1.0, 1.0])
    report = compare_jobs(baseline, candidate, seed=7)
    reward = report["metrics"]["reward_difference"]
    assert reward["verdict"] == "improved"
    assert "improvement/reduction language" in render_markdown(report)


def test_missing_metrics_remain_visible_as_unavailable() -> None:
    baseline = _job("baseline", [0.0, 1.0], tokens=None)
    candidate = _job("candidate", [1.0, 1.0])

    report = compare_jobs(baseline, candidate)

    assert report["metrics"]["token_percentage_change"]["status"] == "unavailable"
    assert "failure_rate_difference" in report["metrics"]
    assert "reported_success_rate_difference" in report["metrics"]
    assert "verifier_success_rate_difference" in report["metrics"]


def test_paired_comparison_refuses_invariant_or_rollout_drift() -> None:
    baseline = _job("baseline", [0.0])
    candidate = LoadedJob(
        Path("candidate"),
        "candidate",
        {"dataset": "different"},
        baseline.trials,
    )
    with pytest.raises(AnalysisError, match="invariants differ"):
        compare_jobs(baseline, candidate)


def test_failed_harbor_trial_is_retained_with_zero_reward(tmp_path: Path) -> None:
    job = tmp_path / "job"
    job.mkdir()
    (job / "smith-harbor-provenance.json").write_text(
        json.dumps({"run_invariants": {"dataset": "frozen"}}),
        encoding="utf-8",
    )
    (job / "result.json").write_text(
        json.dumps(
            {
                "trial_results": [
                    {
                        "task_name": "task-1",
                        "verifier_result": {"rewards": {"reward": 1.0}},
                        "exception_info": {"type": "AgentError"},
                    }
                ]
            }
        ),
        encoding="utf-8",
    )

    loaded = load_job(job)

    assert len(loaded.trials) == 1
    assert loaded.trials[0].failed is True
    assert loaded.trials[0].reward == 0.0


def test_harbor_020_trial_directory_layout_is_loaded(tmp_path: Path) -> None:
    job = tmp_path / "job"
    trial = job / "task-1__trial"
    trial.mkdir(parents=True)
    (job / "smith-harbor-provenance.json").write_text(
        json.dumps({"run_invariants": {"dataset": "frozen"}}),
        encoding="utf-8",
    )
    (job / "result.json").write_text(
        json.dumps({"n_total_trials": 1}),
        encoding="utf-8",
    )
    (trial / "result.json").write_text(
        json.dumps(
            {
                "task_name": "task-1",
                "verifier_result": {"rewards": {"reward": 0.5}},
                "exception_info": None,
            }
        ),
        encoding="utf-8",
    )

    loaded = load_job(job)

    assert len(loaded.trials) == 1
    assert loaded.trials[0].reward == 0.5
