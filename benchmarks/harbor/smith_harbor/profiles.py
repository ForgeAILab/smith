"""Frozen Harbor Index profile loading and job configuration."""

from __future__ import annotations

import dataclasses
import hashlib
import importlib.metadata
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from smith_harbor.artifacts import verify_artifact
from smith_harbor.auth import default_auth_path, select_auth
from smith_harbor.constants import (
    APPROVAL,
    ARTIFACT_MANIFEST_PATH,
    BRIDGE_VERSION,
    DATASET_MANIFEST_PATH,
    DATASET_NAME,
    DATASET_REF,
    DATASET_TASK_COUNT,
    EFFORT,
    HARBOR_MODEL,
    HARBOR_VERSION,
    MODEL,
    PACKAGE_ROOT,
    PROFILES_DIR,
    PROVIDER,
    PROVIDER_KIND,
    SUPPORTED_TARGETS,
    default_jobs_dir,
)
from smith_harbor.variants import Variant, load_variant, variant_provenance


class ProfileError(ValueError):
    """A frozen profile or requested run violates its declared invariants."""


_INTERRUPT_GRACE_SECONDS = 15.0


def _run_owned_process(
    command: Sequence[str],
    *,
    cwd: Path,
    env: dict[str, str],
) -> int:
    """Run Harbor in its own process group and tear the group down on interrupt."""
    process = subprocess.Popen(
        list(command),
        cwd=cwd,
        env=env,
        start_new_session=True,
    )
    try:
        return process.wait()
    except KeyboardInterrupt:
        for interrupt_signal in (signal.SIGINT, signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(process.pid, interrupt_signal)
            except ProcessLookupError:
                break
            try:
                process.wait(timeout=_INTERRUPT_GRACE_SECONDS)
                break
            except subprocess.TimeoutExpired:
                continue
        raise


def _harbor_cli() -> str:
    sibling = Path(sys.executable).with_name("harbor")
    if sibling.is_file() and os.access(sibling, os.X_OK):
        return str(sibling)
    discovered = shutil.which("harbor")
    if discovered is None:
        raise ProfileError("Harbor CLI is unavailable in the active Python environment")
    return discovered


def docker_context() -> str:
    override = os.environ.get("SMITH_HARBOR_DOCKER_CONTEXT")
    if override:
        return override
    completed = subprocess.run(
        ["docker", "context", "show"],
        capture_output=True,
        text=True,
        check=False,
    )
    selected = completed.stdout.strip()
    if completed.returncode != 0 or not selected:
        raise ProfileError("the active Docker context could not be resolved")
    return selected


@dataclasses.dataclass(frozen=True)
class DatasetTask:
    name: str
    ref: str


@dataclasses.dataclass(frozen=True)
class DatasetManifest:
    name: str
    ref: str
    tasks: tuple[DatasetTask, ...]


@dataclasses.dataclass(frozen=True)
class RunProfile:
    name: str
    description: str
    tasks: tuple[DatasetTask, ...]
    rollouts: int
    concurrency: int
    timeout_multiplier: float


def _load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ProfileError(f"profile document {path.name!r} is unavailable or malformed") from exc
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise ProfileError(f"profile document {path.name!r} must use schema version 1")
    return value


def load_dataset_manifest() -> DatasetManifest:
    raw = _load_json(DATASET_MANIFEST_PATH)
    if raw.get("name") != DATASET_NAME or raw.get("ref") != DATASET_REF:
        raise ProfileError("Harbor Index manifest identity does not match the pinned constants")
    task_values = raw.get("tasks")
    if not isinstance(task_values, list):
        raise ProfileError("Harbor Index manifest tasks must be a list")
    tasks: list[DatasetTask] = []
    for item in task_values:
        if not isinstance(item, dict):
            raise ProfileError("Harbor Index task entries must be objects")
        name, ref = item.get("name"), item.get("ref")
        if (
            not isinstance(name, str)
            or not name
            or not isinstance(ref, str)
            or not ref.startswith("sha256:")
        ):
            raise ProfileError("Harbor Index task entry lacks a pinned name/ref")
        tasks.append(DatasetTask(name, ref))
    if len(tasks) != DATASET_TASK_COUNT or raw.get("task_count") != DATASET_TASK_COUNT:
        raise ProfileError(f"Harbor Index manifest must contain {DATASET_TASK_COUNT} tasks")
    if len({task.name for task in tasks}) != len(tasks):
        raise ProfileError("Harbor Index manifest contains duplicate task names")
    return DatasetManifest(name=DATASET_NAME, ref=DATASET_REF, tasks=tuple(tasks))


def load_profile(name: str) -> RunProfile:
    if name not in {"smoke", "dev", "full"}:
        raise ProfileError("profile must be one of smoke, dev, or full")
    raw = _load_json(PROFILES_DIR / f"{name}.json")
    if (
        raw.get("name") != name
        or raw.get("dataset") != DATASET_NAME
        or raw.get("dataset_ref") != DATASET_REF
    ):
        raise ProfileError(f"profile {name!r} does not select the pinned Harbor Index dataset")
    dataset = load_dataset_manifest()
    by_name = {task.name: task for task in dataset.tasks}
    requested = raw.get("tasks")
    if requested == "all":
        tasks = dataset.tasks
    elif isinstance(requested, list) and all(isinstance(item, str) for item in requested):
        if len(set(requested)) != len(requested):
            raise ProfileError(f"profile {name!r} contains duplicate tasks")
        missing = [task_name for task_name in requested if task_name not in by_name]
        if missing:
            raise ProfileError(f"profile {name!r} names unknown tasks: {missing}")
        tasks = tuple(by_name[task_name] for task_name in requested)
    else:
        raise ProfileError(f"profile {name!r} has an invalid task selection")
    rollouts = raw.get("rollouts")
    concurrency = raw.get("concurrency")
    timeout = raw.get("timeout_multiplier")
    if not isinstance(rollouts, int) or rollouts < 1:
        raise ProfileError("profile rollouts must be a positive integer")
    if concurrency != 1:
        raise ProfileError("frozen OAuth profiles must default to concurrency one")
    if not isinstance(timeout, (int, float)) or isinstance(timeout, bool) or timeout <= 0:
        raise ProfileError("profile timeout multiplier must be positive")
    description = raw.get("description")
    if not isinstance(description, str) or not description:
        raise ProfileError("profile description must be nonempty")
    return RunProfile(name, description, tasks, rollouts, concurrency, float(timeout))


def task_set_sha256(tasks: Sequence[DatasetTask]) -> str:
    payload = json.dumps(
        [{"name": task.name, "ref": task.ref} for task in tasks],
        sort_keys=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(payload.encode()).hexdigest()


def _artifact_set_sha256() -> str:
    return hashlib.sha256(ARTIFACT_MANIFEST_PATH.read_bytes()).hexdigest()


def run_invariants(
    profile: RunProfile, *, concurrency: int, variant: Variant | None = None
) -> dict[str, object]:
    selected = variant or load_variant("current")
    return {
        "harbor_version": HARBOR_VERSION,
        "bridge_version": BRIDGE_VERSION,
        "dataset": DATASET_NAME,
        "dataset_ref": DATASET_REF,
        "task_count": len(profile.tasks),
        "task_set_sha256": task_set_sha256(profile.tasks),
        "provider": PROVIDER,
        "provider_kind": PROVIDER_KIND,
        "model": MODEL,
        "effort": EFFORT,
        "approval": APPROVAL,
        "persistence": False,
        "rollouts": profile.rollouts,
        "timeout_multiplier": profile.timeout_multiplier,
        "resource_policy": "task_declared",
        "environment_backend": "docker",
        "docker_context": docker_context(),
        "network_policy": {
            "task_declared": True,
            "agent_extra_allowed_hosts": ["auth.openai.com", "chatgpt.com"],
        },
        "oauth_concurrency": concurrency,
        "cost_provenance": "unknown_subscription_oauth",
        "smith_artifact_set_sha256": _artifact_set_sha256(),
        **variant_provenance(selected),
    }


def job_config(
    profile: RunProfile,
    *,
    job_name: str,
    jobs_dir: Path,
    concurrency: int = 1,
    variant: Variant | None = None,
) -> dict[str, object]:
    if concurrency < 1:
        raise ProfileError("concurrency must be positive")
    unsafe = concurrency > 1
    selected = variant or load_variant("current")
    invariants = run_invariants(profile, concurrency=concurrency, variant=selected)
    return {
        "job_name": job_name,
        "jobs_dir": str(jobs_dir.resolve()),
        "n_attempts": profile.rollouts,
        "timeout_multiplier": profile.timeout_multiplier,
        "n_concurrent_trials": concurrency,
        "quiet": False,
        "environment": {"type": "docker", "delete": True},
        "agents": [
            {
                "import_path": "smith_harbor.smith_agent:SmithAgent",
                "model_name": HARBOR_MODEL,
                "n_concurrent": concurrency,
                "extra_allowed_hosts": ["auth.openai.com", "chatgpt.com"],
                "include_logs": [
                    "instruction.txt",
                    "smith-stream.jsonl",
                    "smith-stderr.log",
                    "smith-exit-code.txt",
                    "provenance.json",
                    "trajectory.json",
                    "converter-diagnostics.json",
                ],
                "kwargs": {
                    "profile_name": profile.name,
                    "variant_name": selected.name,
                    "run_invariants": invariants,
                    "allow_concurrent_oauth": unsafe,
                },
            }
        ],
        "datasets": [
            {
                "name": DATASET_NAME,
                "ref": DATASET_REF,
                "task_names": [task.name for task in profile.tasks],
            }
        ],
        "artifacts": [],
    }


def preflight(profile: RunProfile) -> None:
    select_auth(default_auth_path(), "chatgpt")
    if importlib.metadata.version("harbor") != HARBOR_VERSION:
        raise ProfileError(f"installed Harbor must be exactly {HARBOR_VERSION}")
    for target in SUPPORTED_TARGETS:
        verify_artifact(ARTIFACT_MANIFEST_PATH, target)
    if profile.name == "full" and len(profile.tasks) != DATASET_TASK_COUNT:
        raise ProfileError("full profile does not contain the complete Harbor Index")


def write_job_provenance(
    job_dir: Path,
    profile: RunProfile,
    concurrency: int,
    variant: Variant | None = None,
) -> None:
    selected = variant or load_variant("current")
    artifact_manifest = json.loads(ARTIFACT_MANIFEST_PATH.read_text(encoding="utf-8"))
    document = {
        "schema_version": 1,
        "profile": dataclasses.asdict(profile),
        "run_invariants": run_invariants(profile, concurrency=concurrency, variant=selected),
        "variant": variant_provenance(selected),
        "smith_artifacts": artifact_manifest,
        "credentials": {
            "provider_path": "chatgpt-responses OAuth",
            "selected_entry": "chatgpt",
            "copy_policy": "serial_selected_entry_refresh_handoff",
            "host_merge": "locked_compare_and_swap_selected_entry_only",
            "credential_values_recorded": False,
        },
    }
    path = job_dir / "smith-harbor-provenance.json"
    temporary = job_dir / ".smith-harbor-provenance.json.tmp"
    temporary.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def run_profile(
    profile_name: str,
    *,
    job_name: str,
    jobs_dir: Path | None = None,
    unsafe_concurrency: int | None = None,
    print_config: bool = False,
    variant_name: str = "current",
) -> int:
    profile = load_profile(profile_name)
    variant = load_variant(variant_name)
    jobs_dir = jobs_dir or default_jobs_dir()
    concurrency = unsafe_concurrency or profile.concurrency
    if concurrency > 1 and unsafe_concurrency is None:
        raise ProfileError("concurrent OAuth copies require --unsafe-concurrency")
    if concurrency > 1 and "unsafe-oauth" not in job_name:
        raise ProfileError("concurrent OAuth job names must contain 'unsafe-oauth'")
    config = job_config(
        profile,
        job_name=job_name,
        jobs_dir=jobs_dir,
        concurrency=concurrency,
        variant=variant,
    )
    if print_config:
        print(json.dumps(config, indent=2, sort_keys=True))
        return 0
    preflight(profile)
    context = docker_context()
    jobs_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="smith-harbor-job-") as raw_dir:
        config_path = Path(raw_dir) / "job.json"
        config_path.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
        try:
            return_code = _run_owned_process(
                [_harbor_cli(), "run", "--config", str(config_path), "--yes"],
                cwd=PACKAGE_ROOT,
                env={**os.environ, "DOCKER_CONTEXT": context},
            )
        finally:
            job_dir = jobs_dir / job_name
            if job_dir.is_dir():
                write_job_provenance(job_dir, profile, concurrency, variant)
    return return_code


def resume_job(job_path: Path) -> int:
    resolved = job_path.expanduser().resolve()
    if not (resolved / "config.json").is_file():
        raise ProfileError("Harbor job path does not contain config.json")
    preflight(load_profile(_profile_name_from_job(resolved)))
    provenance = _load_json(resolved / "smith-harbor-provenance.json")
    invariants = provenance.get("run_invariants")
    context = invariants.get("docker_context") if isinstance(invariants, dict) else None
    if not isinstance(context, str) or not context:
        context = docker_context()
    return _run_owned_process(
        [_harbor_cli(), "jobs", "resume", "--job-path", str(resolved)],
        cwd=PACKAGE_ROOT,
        env={**os.environ, "DOCKER_CONTEXT": context},
    )


def _profile_name_from_job(job_path: Path) -> str:
    provenance_path = job_path / "smith-harbor-provenance.json"
    raw = _load_json(provenance_path)
    profile = raw.get("profile")
    if not isinstance(profile, dict) or profile.get("name") not in {"smoke", "dev", "full"}:
        raise ProfileError("job provenance does not identify a frozen Smith profile")
    return str(profile["name"])
