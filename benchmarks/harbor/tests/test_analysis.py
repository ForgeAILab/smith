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


def test_cache_weighted_input_and_hit_rate_surface_cache_thrashing(tmp_path: Path) -> None:
    def make_job(name: str, input_tokens: int, cache_tokens: int) -> Path:
        job = tmp_path / name
        job.mkdir()
        (job / "smith-harbor-provenance.json").write_text(
            json.dumps({"run_invariants": {"dataset": "frozen"}}),
            encoding="utf-8",
        )
        trials = [
            {
                "task_name": f"task-{index}",
                "verifier_result": {"rewards": {"reward": 1.0}},
                "exception_info": None,
                "agent_result": {
                    "n_input_tokens": input_tokens,
                    "n_cache_tokens": cache_tokens,
                    "n_output_tokens": 1_000,
                },
            }
            for index in range(3)
        ]
        (job / "result.json").write_text(json.dumps({"trial_results": trials}), encoding="utf-8")
        return job

    baseline = load_job(make_job("baseline", input_tokens=100_000, cache_tokens=90_000))
    candidate = load_job(make_job("candidate", input_tokens=80_000, cache_tokens=10_000))

    report = compare_jobs(baseline, candidate, seed=7)
    metrics = report["metrics"]
    assert metrics["token_percentage_change"]["verdict"] == "reduced"
    assert metrics["cache_weighted_input_percentage_change"]["verdict"] == "increased"
    assert metrics["cache_hit_rate_difference"]["verdict"] == "decreased"

    markdown = render_markdown(report)
    assert "Cache-weighted input change" in markdown
    assert "Cache hit-rate difference" in markdown


def test_paired_comparison_refuses_different_serving_limits() -> None:
    baseline = LoadedJob(
        Path("baseline"),
        "baseline",
        {"dataset": "frozen", "serving_limits": {"max_input_tokens": 255_616}},
        _job("baseline", [1.0]).trials,
    )
    candidate = LoadedJob(
        Path("candidate"),
        "candidate",
        {"dataset": "frozen", "serving_limits": {"max_input_tokens": 64_000}},
        _job("candidate", [1.0]).trials,
    )
    with pytest.raises(AnalysisError, match="serving_limits"):
        compare_jobs(baseline, candidate)
