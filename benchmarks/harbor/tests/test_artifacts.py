from __future__ import annotations

import json
import struct
from pathlib import Path

import pytest

from smith_harbor.artifacts import (
    ArtifactError,
    sha256_file,
    target_for_machine,
    verify_artifact,
)

TARGET = "x86_64-unknown-linux-musl"


def _artifact(tmp_path: Path, *, machine: int = 62) -> tuple[Path, Path]:
    binary = tmp_path / "smith-x86_64-unknown-linux-musl"
    header = bytearray(64)
    header[:4] = b"\x7fELF"
    header[4] = 2
    header[5] = 1
    struct.pack_into("<H", header, 18, machine)
    binary.write_bytes(header)
    binary.chmod(0o700)
    manifest = tmp_path / "manifest.json"
    manifest.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "smith_revision": "a" * 40,
                "dirty": False,
                "artifacts": [
                    {
                        "target": TARGET,
                        "path": binary.name,
                        "size": binary.stat().st_size,
                        "sha256": sha256_file(binary),
                        "version": "smith 0.1.0",
                    }
                ],
            }
        ),
        encoding="utf-8",
    )
    return manifest, binary


def test_artifact_preflight_verifies_digest_architecture_and_mode(tmp_path: Path) -> None:
    manifest, binary = _artifact(tmp_path)

    verified = verify_artifact(manifest, TARGET)

    assert verified.path == binary.resolve()
    assert verified.sha256 == sha256_file(binary)
    assert target_for_machine("amd64\n") == TARGET


def test_artifact_preflight_rejects_digest_change(tmp_path: Path) -> None:
    manifest, binary = _artifact(tmp_path)
    binary.write_bytes(binary.read_bytes() + b"changed")

    with pytest.raises(ArtifactError, match="size"):
        verify_artifact(manifest, TARGET)


def test_artifact_preflight_rejects_wrong_architecture(tmp_path: Path) -> None:
    manifest, _ = _artifact(tmp_path, machine=183)

    with pytest.raises(ArtifactError, match="architecture"):
        verify_artifact(manifest, TARGET)


def test_artifact_preflight_rejects_non_executable(tmp_path: Path) -> None:
    manifest, binary = _artifact(tmp_path)
    binary.chmod(0o600)

    with pytest.raises(ArtifactError, match="owner-executable"):
        verify_artifact(manifest, TARGET)


def test_artifact_preflight_rejects_dynamic_elf_interpreter(tmp_path: Path) -> None:
    manifest, binary = _artifact(tmp_path)
    header = bytearray(binary.read_bytes())
    struct.pack_into("<Q", header, 32, 64)
    struct.pack_into("<H", header, 54, 56)
    struct.pack_into("<H", header, 56, 1)
    program_header = bytearray(56)
    struct.pack_into("<I", program_header, 0, 3)
    binary.write_bytes(header + program_header)
    binary.chmod(0o700)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    document["artifacts"][0]["size"] = binary.stat().st_size
    document["artifacts"][0]["sha256"] = sha256_file(binary)
    manifest.write_text(json.dumps(document), encoding="utf-8")

    with pytest.raises(ArtifactError, match="statically linked"):
        verify_artifact(manifest, TARGET)
