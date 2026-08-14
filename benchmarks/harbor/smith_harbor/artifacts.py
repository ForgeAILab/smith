"""Static Smith artifact manifests and preflight verification."""

from __future__ import annotations

import hashlib
import json
import stat
import struct
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from smith_harbor.constants import SUPPORTED_TARGETS


class ArtifactError(ValueError):
    """A Smith artifact or manifest failed verification."""


@dataclass(frozen=True)
class VerifiedArtifact:
    """A digest- and architecture-verified Smith executable."""

    target: str
    path: Path
    sha256: str
    size: int
    version: str
    revision: str
    dirty: bool
    manifest: dict[str, Any]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def elf_machine(path: Path) -> int:
    """Return e_machine for a 64-bit, little-endian ELF executable."""
    with path.open("rb") as handle:
        header = handle.read(64)
    if len(header) < 64 or header[:4] != b"\x7fELF":
        raise ArtifactError("Smith artifact is not an ELF executable")
    if header[4] != 2 or header[5] != 1:
        raise ArtifactError("Smith artifact must be 64-bit little-endian ELF")
    return int(struct.unpack_from("<H", header, 18)[0])


def elf_has_interpreter(path: Path) -> bool:
    """Return whether a 64-bit little-endian ELF declares a dynamic loader."""
    with path.open("rb") as handle:
        header = handle.read(64)
        if len(header) < 64 or header[:6] != b"\x7fELF\x02\x01":
            raise ArtifactError("Smith artifact is not a supported ELF executable")
        program_offset = struct.unpack_from("<Q", header, 32)[0]
        entry_size = struct.unpack_from("<H", header, 54)[0]
        entry_count = struct.unpack_from("<H", header, 56)[0]
        if entry_count and entry_size < 56:
            raise ArtifactError("Smith artifact has a malformed ELF program table")
        for index in range(entry_count):
            handle.seek(program_offset + index * entry_size)
            entry = handle.read(entry_size)
            if len(entry) != entry_size:
                raise ArtifactError("Smith artifact has a truncated ELF program table")
            if struct.unpack_from("<I", entry)[0] == 3:  # PT_INTERP
                return True
    return False


def load_manifest(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ArtifactError("Smith artifact manifest is unavailable or malformed") from exc
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise ArtifactError("Smith artifact manifest must use schema version 1")
    artifacts = value.get("artifacts")
    if not isinstance(artifacts, list) or not artifacts:
        raise ArtifactError("Smith artifact manifest has no artifacts")
    return value


def _artifact_record(manifest: dict[str, Any], target: str) -> dict[str, Any]:
    if target not in SUPPORTED_TARGETS:
        raise ArtifactError(f"unsupported Smith target {target!r}")
    matches = [
        item
        for item in manifest["artifacts"]
        if isinstance(item, dict) and item.get("target") == target
    ]
    if len(matches) != 1:
        raise ArtifactError(f"manifest must contain exactly one {target!r} artifact")
    return matches[0]


def verify_artifact(
    manifest_path: Path,
    target: str,
    *,
    execute_version: bool = False,
) -> VerifiedArtifact:
    """Verify manifest fields, digest, ELF architecture, mode, and optional version."""
    manifest = load_manifest(manifest_path)
    record = _artifact_record(manifest, target)
    raw_path = record.get("path")
    if not isinstance(raw_path, str) or not raw_path or Path(raw_path).is_absolute():
        raise ArtifactError("artifact path must be a nonempty manifest-relative path")
    path = (manifest_path.parent / raw_path).resolve()
    try:
        path.relative_to(manifest_path.parent.resolve())
    except ValueError as exc:
        raise ArtifactError("artifact path escapes the manifest directory") from exc
    try:
        metadata = path.lstat()
    except OSError as exc:
        raise ArtifactError(f"artifact for {target!r} is unavailable") from exc
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ArtifactError("Smith artifact must be a regular non-symlink file")
    if not metadata.st_mode & stat.S_IXUSR:
        raise ArtifactError("Smith artifact is not owner-executable")

    expected_size = record.get("size")
    if not isinstance(expected_size, int) or expected_size != metadata.st_size:
        raise ArtifactError("Smith artifact size does not match its manifest")
    expected_digest = record.get("sha256")
    if not isinstance(expected_digest, str) or sha256_file(path) != expected_digest:
        raise ArtifactError("Smith artifact digest does not match its manifest")
    expected_machine = SUPPORTED_TARGETS[target]["elf_machine"]
    if elf_machine(path) != expected_machine:
        raise ArtifactError("Smith artifact ELF architecture does not match its target")
    if elf_has_interpreter(path):
        raise ArtifactError("Smith artifact must be statically linked without an ELF interpreter")

    version = record.get("version")
    revision = manifest.get("smith_revision")
    dirty = manifest.get("dirty")
    if not isinstance(version, str) or not version.startswith("smith "):
        raise ArtifactError("Smith artifact manifest lacks verified version output")
    if not isinstance(revision, str) or len(revision) != 40:
        raise ArtifactError("Smith artifact manifest lacks a full Git revision")
    if not isinstance(dirty, bool):
        raise ArtifactError("Smith artifact manifest lacks dirty-state provenance")
    if execute_version:
        try:
            completed = subprocess.run(
                [path, "--version"],
                check=True,
                capture_output=True,
                text=True,
                timeout=15,
            )
        except (OSError, subprocess.SubprocessError) as exc:
            raise ArtifactError("Smith artifact did not execute --version successfully") from exc
        if completed.stdout.strip() != version:
            raise ArtifactError("Smith artifact version output does not match its manifest")

    return VerifiedArtifact(
        target=target,
        path=path,
        sha256=expected_digest,
        size=expected_size,
        version=version,
        revision=revision,
        dirty=dirty,
        manifest=manifest,
    )


def target_for_machine(machine: str) -> str:
    normalized = machine.strip().lower()
    for target, details in SUPPORTED_TARGETS.items():
        aliases = details["uname_machine"]
        if isinstance(aliases, tuple) and normalized in aliases:
            return target
    raise ArtifactError(f"unsupported Linux architecture {machine!r}")
