"""Command-line entry point for Smith's pinned Harbor evaluation."""

from __future__ import annotations

import argparse
import datetime as dt
import json
from collections.abc import Sequence
from pathlib import Path

from smith_harbor import analysis
from smith_harbor.artifacts import verify_artifact
from smith_harbor.audit import audit_job
from smith_harbor.auth import default_auth_path, select_auth
from smith_harbor.build_artifacts import build_all
from smith_harbor.constants import ARTIFACT_MANIFEST_PATH, SUPPORTED_TARGETS, default_jobs_dir
from smith_harbor.experiment import run_experiment, write_manifest
from smith_harbor.local_run import default_local_binary, run_base_probe, run_canary
from smith_harbor.profiles import load_profile, resume_job, run_profile
from smith_harbor.variants import VARIANTS


def _default_job_name(profile: str) -> str:
    timestamp = dt.datetime.now(dt.UTC).strftime("%Y%m%dT%H%M%SZ")
    return f"smith-luna-max-{profile}-{timestamp}"


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    commands.add_parser("auth-check", help="validate the selected OAuth entry without printing it")

    build = commands.add_parser("build", help="build both static Linux Smith artifacts")
    build.add_argument("--output-dir", type=Path, default=ARTIFACT_MANIFEST_PATH.parent)
    build.add_argument("--allow-dirty", action="store_true")

    verify = commands.add_parser("verify-artifacts", help="verify both pinned Smith artifacts")
    verify.add_argument("--manifest", type=Path, default=ARTIFACT_MANIFEST_PATH)

    for name, help_text in (
        ("canary", "run one isolated live Luna Max OAuth canary"),
        ("probe", "measure planned base footprint and observed first-request usage"),
    ):
        command = commands.add_parser(name, help=help_text)
        command.add_argument("--binary", type=Path, default=default_local_binary())
        command.add_argument("--output", type=Path)
        if name == "canary":
            command.add_argument("--variant", choices=sorted(VARIANTS), default="current")

    validate = commands.add_parser("validate-profile", help="validate a frozen run profile")
    validate.add_argument("profile", choices=["smoke", "dev", "full"])

    run = commands.add_parser("run", help="run a frozen Harbor profile")
    run.add_argument("profile", choices=["smoke", "dev", "full"])
    run.add_argument("--job-name")
    run.add_argument("--jobs-dir", type=Path, default=default_jobs_dir())
    run.add_argument("--unsafe-concurrency", type=int)
    run.add_argument("--print-config", action="store_true")
    run.add_argument("--variant", choices=sorted(VARIANTS), default="current")

    resume = commands.add_parser("resume", help="resume a Smith Harbor job")
    resume.add_argument("job_path", type=Path)

    audit = commands.add_parser("audit-job", help="scan a completed job for OAuth material")
    audit.add_argument("job_path", type=Path)

    compare = commands.add_parser("analyze", help="compare two Harbor jobs")
    compare.add_argument("baseline", type=Path)
    compare.add_argument("candidate", type=Path)
    compare.add_argument("--json", type=Path, required=True)
    compare.add_argument("--markdown", type=Path, required=True)
    compare.add_argument("--resamples", type=int, default=10_000)
    compare.add_argument("--seed", type=int, default=20260808)
    compare.add_argument("--descriptive-only", action="store_true")

    experiment_plan = commands.add_parser(
        "experiment-plan", help="write the frozen nine-cell dev ablation manifest"
    )
    experiment_plan.add_argument("--manifest", type=Path, required=True)

    experiment_run = commands.add_parser(
        "experiment-run", help="run or resume the frozen serial dev ablation"
    )
    experiment_run.add_argument("--manifest", type=Path, required=True)
    experiment_run.add_argument("--jobs-dir", type=Path, default=default_jobs_dir())

    experiment_analyze = commands.add_parser(
        "experiment-analyze", help="analyze all nine compatible dev ablation jobs"
    )
    experiment_analyze.add_argument("--manifest", type=Path, required=True)
    experiment_analyze.add_argument("--jobs-dir", type=Path, default=default_jobs_dir())
    experiment_analyze.add_argument("--json", type=Path, required=True)
    experiment_analyze.add_argument("--markdown", type=Path, required=True)
    experiment_analyze.add_argument("--resamples", type=int, default=10_000)
    experiment_analyze.add_argument("--seed", type=int, default=20260808)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    if args.command == "auth-check":
        selected = select_auth(default_auth_path(), "chatgpt")
        print(json.dumps({"schema_version": 1, "status": "ready", "entry": selected.entry}))
        return 0
    if args.command == "build":
        print(build_all(args.output_dir, allow_dirty=args.allow_dirty))
        return 0
    if args.command == "verify-artifacts":
        for target in SUPPORTED_TARGETS:
            artifact = verify_artifact(args.manifest, target)
            print(f"{target}\t{artifact.sha256}\t{artifact.version}")
        return 0
    if args.command == "canary":
        run_canary(args.binary, args.output, variant_name=args.variant)
        return 0
    if args.command == "probe":
        run_base_probe(args.binary, args.output)
        return 0
    if args.command == "validate-profile":
        profile = load_profile(args.profile)
        print(
            json.dumps(
                {
                    "schema_version": 1,
                    "profile": profile.name,
                    "tasks": len(profile.tasks),
                    "rollouts": profile.rollouts,
                    "concurrency": profile.concurrency,
                },
                sort_keys=True,
            )
        )
        return 0
    if args.command == "run":
        job_name = args.job_name or _default_job_name(args.profile)
        return run_profile(
            args.profile,
            job_name=job_name,
            jobs_dir=args.jobs_dir,
            unsafe_concurrency=args.unsafe_concurrency,
            print_config=args.print_config,
            variant_name=args.variant,
        )
    if args.command == "resume":
        return resume_job(args.job_path)
    if args.command == "audit-job":
        print(json.dumps(audit_job(args.job_path), indent=2, sort_keys=True))
        return 0
    if args.command == "analyze":
        report = analysis.compare_jobs(
            analysis.load_job(args.baseline),
            analysis.load_job(args.candidate),
            resamples=args.resamples,
            seed=args.seed,
            descriptive_only=args.descriptive_only,
        )
        analysis.write_reports(report, args.json, args.markdown)
        return 0
    if args.command == "experiment-plan":
        write_manifest(args.manifest)
        return 0
    if args.command == "experiment-run":
        return run_experiment(manifest_path=args.manifest, jobs_dir=args.jobs_dir)
    if args.command == "experiment-analyze":
        report = analysis.compare_experiment(
            args.manifest,
            args.jobs_dir,
            resamples=args.resamples,
            seed=args.seed,
        )
        analysis.write_experiment_reports(report, args.json, args.markdown)
        return 0
    raise AssertionError(f"unhandled command {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
