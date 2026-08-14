"""Build static Linux Smith binaries once in architecture-matched containers."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import shutil
import subprocess
import tempfile
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from smith_harbor.artifacts import elf_machine, sha256_file, verify_artifact
from smith_harbor.constants import ARTIFACT_MANIFEST_PATH, REPOSITORY_ROOT, SUPPORTED_TARGETS

BUILD_IMAGE = (
    "ghcr.io/rust-cross/cargo-zigbuild@"
    "sha256:b8364c2c60cdcc9b95c402d17654bff517410926a35678bd89dd924b8158d6ae"
)
BUILD_PLATFORM = "linux/arm64"
BUILD_PACKAGES = "build-essential cmake perl pkg-config"


def _git(*args: str) -> str:
    return subprocess.run(
        ["git", *args],
        cwd=REPOSITORY_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def source_state(allow_dirty: bool) -> tuple[str, bool]:
    revision = _git("rev-parse", "HEAD")
    dirty = bool(_git("status", "--porcelain=v1"))
    if dirty and not allow_dirty:
        raise RuntimeError(
            "Smith source tree is dirty; commit/stash changes or rerun with --allow-dirty "
            "for a labelled development artifact"
        )
    return revision, dirty


def _copy_tracked_worktree(source: Path, destination: Path) -> None:
    """Stage actual tracked contents without Git metadata, build output, or secrets."""
    completed = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=source,
        check=True,
        capture_output=True,
    )
    destination.mkdir(parents=True, exist_ok=True)
    for raw_path in completed.stdout.split(b"\0"):
        if not raw_path:
            continue
        relative = Path(os.fsdecode(raw_path))
        source_path = source / relative
        destination_path = destination / relative
        destination_path.parent.mkdir(parents=True, exist_ok=True)
        if source_path.is_symlink():
            destination_path.symlink_to(os.readlink(source_path))
        elif source_path.is_file():
            shutil.copy2(source_path, destination_path)


def _stage_build_context(root: Path) -> None:
    """Stage only Smith's tracked build graph; ignored local Cargo patches stay local."""
    _copy_tracked_worktree(REPOSITORY_ROOT, root / REPOSITORY_ROOT.name)


def _docker_build(target: str, output_dir: Path) -> tuple[Path, str, str]:
    container_repo = f"/workspace/{REPOSITORY_ROOT.name}"
    container_target = f"/tmp/smith-harbor-build/{target}"
    command = (
        "set -eu; "
        "apt-get update -qq; "
        f"DEBIAN_FRONTEND=noninteractive apt-get install -y -qq {BUILD_PACKAGES} >/dev/null; "
        "CARGO_BUILD_JOBS=2 "
        f"CARGO_TARGET_DIR={container_target} "
        f"cargo zigbuild --locked --release -p smith-cli --target {target}; "
        f"{container_target}/{target}/release/smith --version"
    )
    with tempfile.TemporaryDirectory(prefix="smith-harbor-build-context-") as raw_dir:
        staging = Path(raw_dir)
        _stage_build_context(staging)
        created = subprocess.run(
            [
                "docker",
                "create",
                "--platform",
                BUILD_PLATFORM,
                "--workdir",
                container_repo,
                BUILD_IMAGE,
                "sh",
                "-c",
                command,
            ],
            check=True,
            capture_output=True,
            text=True,
        )
        container = created.stdout.strip()
        try:
            subprocess.run(
                ["docker", "cp", f"{staging}/.", f"{container}:/workspace"],
                check=True,
                capture_output=True,
                text=True,
            )
            completed = subprocess.run(
                ["docker", "start", "--attach", container],
                check=False,
                capture_output=True,
                text=True,
            )
            if completed.returncode != 0:
                detail = (completed.stderr or completed.stdout or "no container output")[-4000:]
                raise RuntimeError(f"{target} Docker build failed: {detail}")
            destination = output_dir / f"smith-{target}"
            subprocess.run(
                [
                    "docker",
                    "cp",
                    f"{container}:{container_target}/{target}/release/smith",
                    destination,
                ],
                check=True,
                capture_output=True,
                text=True,
            )
        finally:
            subprocess.run(
                ["docker", "rm", "--force", container],
                check=False,
                capture_output=True,
                text=True,
            )
    destination.chmod(destination.stat().st_mode | 0o700)
    version_lines = [line.strip() for line in completed.stdout.splitlines() if line.strip()]
    if not version_lines or not version_lines[-1].startswith("smith "):
        raise RuntimeError(f"{target} build did not report a Smith version")
    return destination, version_lines[-1], command


def build_all(output_dir: Path, *, allow_dirty: bool = False) -> Path:
    revision, dirty = source_state(allow_dirty)
    output_dir.mkdir(parents=True, exist_ok=True)
    records: list[dict[str, Any]] = []
    for target in SUPPORTED_TARGETS:
        binary, version, command = _docker_build(target, output_dir)
        records.append(
            {
                "target": target,
                "path": binary.name,
                "sha256": sha256_file(binary),
                "size": binary.stat().st_size,
                "elf_machine": elf_machine(binary),
                "version": version,
                "build_command": command,
                "build_platform": BUILD_PLATFORM,
                "runtime_platform": SUPPORTED_TARGETS[target]["docker_platform"],
            }
        )
    manifest = {
        "schema_version": 1,
        "smith_revision": revision,
        "dirty": dirty,
        "built_at": dt.datetime.now(dt.UTC).isoformat(),
        "build_image": BUILD_IMAGE,
        "artifacts": records,
    }
    manifest_path = output_dir / "manifest.json"
    temporary = output_dir / ".manifest.json.tmp"
    temporary.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, manifest_path)
    for target in SUPPORTED_TARGETS:
        verify_artifact(manifest_path, target)
    return manifest_path


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=ARTIFACT_MANIFEST_PATH.parent)
    parser.add_argument("--allow-dirty", action="store_true")
    args = parser.parse_args(argv)
    manifest = build_all(args.output_dir, allow_dirty=args.allow_dirty)
    print(manifest)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
