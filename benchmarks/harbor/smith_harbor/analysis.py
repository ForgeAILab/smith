"""Task-paired Harbor job comparison with deterministic bootstrap intervals."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import math
import os
import random
import statistics
from collections import defaultdict
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

COMPATIBILITY_KEYS = (
    "dataset",
    "dataset_ref",
    "task_count",
    "task_set_sha256",
    "provider",
    "provider_kind",
    "model",
    "effort",
    "approval",
    "persistence",
    "rollouts",
    "timeout_multiplier",
    "resource_policy",
    "environment_backend",
    "docker_context",
    "network_policy",
    "oauth_concurrency",
    "smith_artifact_set_sha256",
)


class AnalysisError(ValueError):
    """Harbor jobs cannot support the requested statistical claim."""


@dataclass(frozen=True)
class TrialObservation:
    task: str
    reward: float
    tokens: float | None
    latency_seconds: float | None
    reported_success: bool
    verifier_success: bool
    failed: bool


@dataclass(frozen=True)
class LoadedJob:
    path: Path
    name: str
    invariants: dict[str, object]
    trials: tuple[TrialObservation, ...]


def _read_object(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AnalysisError(f"could not read JSON object {path.name!r}") from exc
    if not isinstance(value, dict):
        raise AnalysisError(f"{path.name!r} must contain a JSON object")
    return value


def _iso_seconds(timing: object) -> float | None:
    if not isinstance(timing, dict):
        return None
    start, finish = timing.get("started_at"), timing.get("finished_at")
    if not isinstance(start, str) or not isinstance(finish, str):
        return None
    try:
        start_at = dt.datetime.fromisoformat(start.replace("Z", "+00:00"))
        finish_at = dt.datetime.fromisoformat(finish.replace("Z", "+00:00"))
    except ValueError:
        return None
    seconds = (finish_at - start_at).total_seconds()
    return seconds if seconds >= 0 else None


def _reward(trial: Mapping[str, object]) -> float:
    verifier = trial.get("verifier_result")
    rewards = verifier.get("rewards") if isinstance(verifier, dict) else None
    if not isinstance(rewards, dict) or not rewards:
        return 0.0
    preferred = rewards.get("reward")
    if isinstance(preferred, (int, float)) and not isinstance(preferred, bool):
        return float(preferred)
    numeric = [
        float(value)
        for value in rewards.values()
        if isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)
    ]
    return statistics.fmean(numeric) if numeric else 0.0


def _contexts(trial: Mapping[str, object]) -> list[Mapping[str, object]]:
    direct = trial.get("agent_result")
    if isinstance(direct, dict):
        return [direct]
    steps = trial.get("step_results")
    if not isinstance(steps, list):
        return []
    return [
        value
        for step in steps
        if isinstance(step, dict) and isinstance((value := step.get("agent_result")), dict)
    ]


def _tokens(contexts: Sequence[Mapping[str, object]]) -> float | None:
    if not contexts:
        return None
    total = 0
    observed = False
    for context in contexts:
        for field in ("n_input_tokens", "n_output_tokens"):
            value = context.get(field)
            if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
                total += value
                observed = True
    return float(total) if observed else None


def _reported_success(contexts: Sequence[Mapping[str, object]]) -> bool:
    statuses: list[bool] = []
    for context in contexts:
        metadata = context.get("metadata")
        value = metadata.get("reported_success") if isinstance(metadata, dict) else None
        if isinstance(value, bool):
            statuses.append(value)
    return bool(statuses) and all(statuses)


def load_job(path: Path) -> LoadedJob:
    root = path.expanduser().resolve()
    result = _read_object(root / "result.json")
    provenance = _read_object(root / "smith-harbor-provenance.json")
    invariants = provenance.get("run_invariants")
    if not isinstance(invariants, dict):
        raise AnalysisError("Smith job provenance lacks run invariants")
    embedded_trials = result.get("trial_results")
    if isinstance(embedded_trials, list):
        raw_trials = embedded_trials
    else:
        raw_trials = [
            _read_object(trial_path)
            for trial_path in sorted(root.glob("*/result.json"))
            if trial_path.parent != root
        ]
        expected_count = result.get("n_total_trials")
        if isinstance(expected_count, int) and expected_count != len(raw_trials):
            raise AnalysisError("Harbor job trial result count is incomplete")
    if not raw_trials:
        raise AnalysisError("Harbor job contains no trial results")
    observations: list[TrialObservation] = []
    for raw in raw_trials:
        if not isinstance(raw, dict):
            raise AnalysisError("Harbor job contains a malformed trial result")
        task = raw.get("task_name")
        if not isinstance(task, str) or not task:
            raise AnalysisError("Harbor trial result lacks a task name")
        contexts = _contexts(raw)
        failed = raw.get("exception_info") is not None
        reward = 0.0 if failed else _reward(raw)
        observations.append(
            TrialObservation(
                task=task,
                reward=reward,
                tokens=_tokens(contexts),
                latency_seconds=_iso_seconds(raw.get("agent_execution")),
                reported_success=_reported_success(contexts),
                verifier_success=reward > 0.0,
                failed=failed,
            )
        )
    return LoadedJob(
        path=root,
        name=root.name,
        invariants=dict(invariants),
        trials=tuple(observations),
    )


def _group(job: LoadedJob) -> dict[str, list[TrialObservation]]:
    grouped: dict[str, list[TrialObservation]] = defaultdict(list)
    for trial in job.trials:
        grouped[trial.task].append(trial)
    return dict(grouped)


def validate_compatible(baseline: LoadedJob, candidate: LoadedJob) -> None:
    differences = [
        key
        for key in COMPATIBILITY_KEYS
        if baseline.invariants.get(key) != candidate.invariants.get(key)
    ]
    if differences:
        raise AnalysisError(
            "paired comparison invariants differ: " + ", ".join(sorted(differences))
        )
    baseline_groups, candidate_groups = _group(baseline), _group(candidate)
    if set(baseline_groups) != set(candidate_groups):
        raise AnalysisError("paired comparison task sets differ")
    mismatched = [
        task
        for task in baseline_groups
        if len(baseline_groups[task]) != len(candidate_groups[task])
    ]
    if mismatched:
        raise AnalysisError("paired comparison rollout counts differ within tasks")


def _mean_optional(values: Iterable[float | None]) -> float | None:
    collected = list(values)
    if not collected or any(value is None for value in collected):
        return None
    return statistics.fmean(value for value in collected if value is not None)


def task_means(job: LoadedJob) -> dict[str, dict[str, float | None]]:
    return {
        task: {
            "reward": statistics.fmean(trial.reward for trial in trials),
            "tokens": _mean_optional(trial.tokens for trial in trials),
            "latency_seconds": _mean_optional(trial.latency_seconds for trial in trials),
            "failure_rate": statistics.fmean(float(trial.failed) for trial in trials),
            "reported_success_rate": statistics.fmean(
                float(trial.reported_success) for trial in trials
            ),
            "verifier_success_rate": statistics.fmean(
                float(trial.verifier_success) for trial in trials
            ),
        }
        for task, trials in _group(job).items()
    }


def _percentile(sorted_values: Sequence[float], fraction: float) -> float:
    if not sorted_values:
        raise AnalysisError("cannot compute a percentile of no values")
    position = (len(sorted_values) - 1) * fraction
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return sorted_values[lower]
    weight = position - lower
    return sorted_values[lower] * (1 - weight) + sorted_values[upper] * weight


def bootstrap_interval(
    paired_values: Sequence[float],
    *,
    resamples: int = 10_000,
    seed: int = 20260808,
) -> dict[str, float | int]:
    if not paired_values:
        raise AnalysisError("paired bootstrap requires at least one task")
    if resamples < 10_000:
        raise AnalysisError("paired bootstrap requires at least 10,000 resamples")
    generator = random.Random(seed)
    count = len(paired_values)
    samples = [
        statistics.fmean(paired_values[generator.randrange(count)] for _ in range(count))
        for _ in range(resamples)
    ]
    samples.sort()
    return {
        "estimate": statistics.fmean(paired_values),
        "ci_2_5": _percentile(samples, 0.025),
        "ci_97_5": _percentile(samples, 0.975),
        "resamples": resamples,
        "seed": seed,
        "tasks": count,
    }


def _paired_metric(
    baseline: Mapping[str, Mapping[str, float | None]],
    candidate: Mapping[str, Mapping[str, float | None]],
    field: str,
    transform: Callable[[float, float], float],
) -> list[float] | None:
    values: list[float] = []
    for task in sorted(baseline):
        old, new = baseline[task][field], candidate[task][field]
        if old is None or new is None:
            return None
        values.append(transform(old, new))
    return values


def _verdict(interval: Mapping[str, float | int], *, lower_is_better: bool) -> str:
    low, high = float(interval["ci_2_5"]), float(interval["ci_97_5"])
    if low <= 0 <= high:
        return "not_statistically_clear"
    estimate = float(interval["estimate"])
    if lower_is_better:
        return "reduced" if estimate < 0 else "increased"
    return "improved" if estimate > 0 else "decreased"


def _cross_tab(job: LoadedJob) -> dict[str, int]:
    counts = {
        "reported_yes_verifier_yes": 0,
        "reported_yes_verifier_no": 0,
        "reported_no_verifier_yes": 0,
        "reported_no_verifier_no": 0,
    }
    for trial in job.trials:
        reported = "yes" if trial.reported_success else "no"
        verifier = "yes" if trial.verifier_success else "no"
        counts[f"reported_{reported}_verifier_{verifier}"] += 1
    return counts


def compare_jobs(
    baseline: LoadedJob,
    candidate: LoadedJob,
    *,
    resamples: int = 10_000,
    seed: int = 20260808,
    descriptive_only: bool = False,
) -> dict[str, object]:
    if descriptive_only:
        return {
            "schema_version": 1,
            "comparison_type": "descriptive_only_unpaired",
            "warning": "No paired performance claim is valid for this report.",
            "baseline": _descriptive(baseline),
            "candidate": _descriptive(candidate),
        }
    validate_compatible(baseline, candidate)
    old, new = task_means(baseline), task_means(candidate)
    rewards = _paired_metric(old, new, "reward", lambda before, after: after - before)
    assert rewards is not None
    token_changes = _paired_metric(
        old,
        new,
        "tokens",
        lambda before, after: ((after / before) - 1) * 100 if before > 0 else math.nan,
    )
    latency_changes = _paired_metric(
        old,
        new,
        "latency_seconds",
        lambda before, after: ((after / before) - 1) * 100 if before > 0 else math.nan,
    )
    if token_changes is not None and any(not math.isfinite(value) for value in token_changes):
        token_changes = None
    if latency_changes is not None and any(not math.isfinite(value) for value in latency_changes):
        latency_changes = None
    failure_changes = _paired_metric(
        old, new, "failure_rate", lambda before, after: (after - before) * 100
    )
    reported_changes = _paired_metric(
        old, new, "reported_success_rate", lambda before, after: (after - before) * 100
    )
    verifier_changes = _paired_metric(
        old, new, "verifier_success_rate", lambda before, after: (after - before) * 100
    )
    reward_interval = bootstrap_interval(rewards, resamples=resamples, seed=seed)
    metrics: dict[str, object] = {
        "reward_difference": {
            **reward_interval,
            "unit": "absolute_reward",
            "verdict": _verdict(reward_interval, lower_is_better=False),
        },
        "token_percentage_change": _interval_or_unavailable(
            token_changes, resamples=resamples, seed=seed, lower_is_better=True
        ),
        "latency_percentage_change": _interval_or_unavailable(
            latency_changes, resamples=resamples, seed=seed, lower_is_better=True
        ),
        "failure_rate_difference": _interval(
            failure_changes,
            resamples=resamples,
            seed=seed,
            lower_is_better=True,
            unit="percentage_points",
        ),
        "reported_success_rate_difference": _interval(
            reported_changes,
            resamples=resamples,
            seed=seed,
            lower_is_better=False,
            unit="percentage_points",
        ),
        "verifier_success_rate_difference": _interval(
            verifier_changes,
            resamples=resamples,
            seed=seed,
            lower_is_better=False,
            unit="percentage_points",
        ),
    }
    return {
        "schema_version": 1,
        "comparison_type": "task_paired_bootstrap",
        "baseline": baseline.name,
        "candidate": candidate.name,
        "invariants": baseline.invariants,
        "metrics": metrics,
        "reported_verifier_cross_tabs": {
            "baseline": _cross_tab(baseline),
            "candidate": _cross_tab(candidate),
        },
        "failed_trials": {
            "baseline": sum(trial.failed for trial in baseline.trials),
            "candidate": sum(trial.failed for trial in candidate.trials),
        },
    }


def _interval_or_unavailable(
    values: Sequence[float] | None,
    *,
    resamples: int,
    seed: int,
    lower_is_better: bool,
) -> dict[str, object]:
    if values is None:
        return {
            "status": "unavailable",
            "reason": "one or more retained trials lack observed metric data",
        }
    interval = bootstrap_interval(values, resamples=resamples, seed=seed)
    return {
        **interval,
        "unit": "percent",
        "verdict": _verdict(interval, lower_is_better=lower_is_better),
    }


def _interval(
    values: Sequence[float] | None,
    *,
    resamples: int,
    seed: int,
    lower_is_better: bool,
    unit: str,
) -> dict[str, object]:
    result = _interval_or_unavailable(
        values,
        resamples=resamples,
        seed=seed,
        lower_is_better=lower_is_better,
    )
    if result.get("status") != "unavailable":
        result["unit"] = unit
    return result


def _descriptive(job: LoadedJob) -> dict[str, object]:
    return {
        "job": job.name,
        "invariants": job.invariants,
        "trials": len(job.trials),
        "tasks": len(_group(job)),
        "mean_reward": statistics.fmean(trial.reward for trial in job.trials),
        "failed_trials": sum(trial.failed for trial in job.trials),
        "reported_verifier_cross_tab": _cross_tab(job),
    }


def render_markdown(report: Mapping[str, object]) -> str:
    if report.get("comparison_type") != "task_paired_bootstrap":
        return (
            "# Smith Harbor comparison\n\n"
            "This is a descriptive-only, unpaired report. "
            "No improvement or reduction claim is valid.\n"
        )
    metrics = report["metrics"]
    assert isinstance(metrics, dict)
    lines = [
        "# Smith Harbor paired comparison",
        "",
        f"Baseline: `{report['baseline']}`  ",
        f"Candidate: `{report['candidate']}`",
        "",
        "| Metric | Estimate | 95% interval | Verdict |",
        "| --- | ---: | ---: | --- |",
    ]
    for label, key in [
        ("Reward difference", "reward_difference"),
        ("Token change", "token_percentage_change"),
        ("Latency change", "latency_percentage_change"),
        ("Failure-rate difference", "failure_rate_difference"),
        ("Reported-success difference", "reported_success_rate_difference"),
        ("Verifier-success difference", "verifier_success_rate_difference"),
    ]:
        metric = metrics[key]
        assert isinstance(metric, dict)
        if metric.get("status") == "unavailable":
            lines.append(f"| {label} | unavailable | unavailable | unavailable |")
            continue
        suffix = "%" if metric.get("unit") in {"percent", "percentage_points"} else ""
        lines.append(
            f"| {label} | {float(metric['estimate']):.3f}{suffix} | "
            f"[{float(metric['ci_2_5']):.3f}, {float(metric['ci_97_5']):.3f}]{suffix} | "
            f"{metric['verdict']} |"
        )
    lines.extend(
        [
            "",
            "Verdicts use improvement/reduction language only when the paired 95% "
            "interval excludes zero.",
            "",
        ]
    )
    return "\n".join(lines)


def write_reports(report: Mapping[str, object], json_path: Path, markdown_path: Path) -> None:
    for path, content in (
        (json_path, json.dumps(report, indent=2, sort_keys=True) + "\n"),
        (markdown_path, render_markdown(report)),
    ):
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_suffix(path.suffix + ".tmp")
        temporary.write_text(content, encoding="utf-8")
        os.replace(temporary, path)


def combine_variant_jobs(paths: Sequence[Path], variant_name: str) -> LoadedJob:
    """Combine three compatible one-rollout cells into task-level rollouts."""
    if len(paths) != 3:
        raise AnalysisError("each experiment variant requires exactly three jobs")
    jobs = [load_job(path) for path in paths]
    names = {job.name for job in jobs}
    if len(names) != 3:
        raise AnalysisError("experiment jobs must be distinct")
    for job in jobs:
        if job.invariants.get("variant") != variant_name:
            raise AnalysisError("experiment job variant does not match the manifest cell")
        if job.invariants.get("rollouts") != 1:
            raise AnalysisError("experiment cells must contain one rollout per task")
        validate_compatible(jobs[0], job)
        groups = _group(job)
        if len(groups) != job.invariants.get("task_count"):
            raise AnalysisError("experiment cell task count does not match its invariant")
        if any(len(values) != 1 for values in groups.values()):
            raise AnalysisError("experiment cell does not contain exactly one trial per task")
    invariants = dict(jobs[0].invariants)
    invariants["rollouts"] = 3
    return LoadedJob(
        path=Path("."),
        name=variant_name,
        invariants=invariants,
        trials=tuple(trial for job in jobs for trial in job.trials),
    )


def compare_experiment(
    manifest_path: Path,
    jobs_dir: Path,
    *,
    resamples: int = 10_000,
    seed: int = 20260808,
) -> dict[str, object]:
    from smith_harbor.experiment import experiment_manifest
    from smith_harbor.variants import VARIANTS, variant_provenance

    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AnalysisError("experiment manifest is unreadable") from exc
    if manifest != experiment_manifest():
        raise AnalysisError("experiment manifest does not match the frozen schedule")
    cells = manifest.get("cells")
    assert isinstance(cells, list)
    grouped_paths: dict[str, list[Path]] = defaultdict(list)
    for cell in cells:
        assert isinstance(cell, dict)
        variant_name = str(cell["variant"])
        expected_variant = variant_provenance(VARIANTS[variant_name])
        if any(cell.get(key) != value for key, value in expected_variant.items()):
            raise AnalysisError("experiment cell changes an undeclared variant axis")
        grouped_paths[variant_name].append(jobs_dir / str(cell["job_name"]))
    combined = {name: combine_variant_jobs(paths, name) for name, paths in grouped_paths.items()}
    policy = compare_jobs(
        combined["current"],
        combined["artifact-first-v1"],
        resamples=resamples,
        seed=seed,
    )
    delegation = compare_jobs(
        combined["artifact-first-v1"],
        combined["artifact-first-v1-no-delegation"],
        resamples=resamples,
        seed=seed,
    )
    return {
        "schema_version": 1,
        "experiment_id": manifest["experiment_id"],
        "evidence_scope": "internal_development_ablation",
        "generalization": "none_beyond_the_inspected_development_set",
        "expected_trajectories": manifest["expected_trajectories"],
        "contrasts": {
            "completion_policy": policy,
            "delegation": delegation,
        },
    }


def render_experiment_markdown(report: Mapping[str, object]) -> str:
    contrasts = report.get("contrasts")
    if not isinstance(contrasts, dict):
        raise AnalysisError("experiment report lacks contrasts")
    sections = [
        "# Smith Harbor completion-policy ablation",
        "",
        "This is an internal development ablation over inspected/tuning tasks. It is not a ",
        "holdout result and makes no full-suite, cross-model, or cross-harness generalization.",
        "",
    ]
    for label, key in (
        ("Completion policy effect", "completion_policy"),
        ("Incremental no-delegation effect", "delegation"),
    ):
        comparison = contrasts.get(key)
        if not isinstance(comparison, dict):
            raise AnalysisError("experiment report has a malformed contrast")
        rendered = render_markdown(comparison).splitlines()
        sections.extend([f"## {label}", "", *rendered[2:], ""])
    return "\n".join(sections)


def write_experiment_reports(
    report: Mapping[str, object], json_path: Path, markdown_path: Path
) -> None:
    for path, content in (
        (json_path, json.dumps(report, indent=2, sort_keys=True) + "\n"),
        (markdown_path, render_experiment_markdown(report)),
    ):
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_suffix(path.suffix + ".tmp")
        temporary.write_text(content, encoding="utf-8")
        os.replace(temporary, path)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--json", type=Path, required=True)
    parser.add_argument("--markdown", type=Path, required=True)
    parser.add_argument("--resamples", type=int, default=10_000)
    parser.add_argument("--seed", type=int, default=20260808)
    parser.add_argument("--descriptive-only", action="store_true")
    args = parser.parse_args(argv)
    report = compare_jobs(
        load_job(args.baseline),
        load_job(args.candidate),
        resamples=args.resamples,
        seed=args.seed,
        descriptive_only=args.descriptive_only,
    )
    write_reports(report, args.json, args.markdown)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
