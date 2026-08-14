"""Credential-content audit for a completed local Harbor job."""

from __future__ import annotations

import stat
from pathlib import Path

from smith_harbor.auth import SelectedAuth, default_auth_path, select_auth


class JobAuditError(RuntimeError):
    """A completed job contains credential material or cannot be audited safely."""


def _contains(path: Path, needle: bytes) -> bool:
    if not needle:
        return False
    overlap = b""
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value = overlap + chunk
            if needle in value:
                return True
            overlap = value[-(len(needle) - 1) :] if len(needle) > 1 else b""
    return False


def audit_job(
    job_path: Path,
    *,
    selected: SelectedAuth | None = None,
    source_auth_path: Path | None = None,
) -> dict[str, object]:
    """Scan regular job files without printing credential values or source paths."""
    root = job_path.expanduser().resolve()
    if not root.is_dir() or not (root / "result.json").is_file():
        raise JobAuditError("job audit target is not a completed Harbor job directory")
    source = source_auth_path or default_auth_path()
    credential = selected or select_auth(source, "chatgpt")
    needles = {
        "credential_value": credential.value.encode(),
        "source_auth_path": str(source.expanduser()).encode(),
        "host_temp_auth_path": b"smith-harbor-auth-",
    }
    hits = {name: 0 for name in needles}
    files_scanned = 0
    bytes_scanned = 0
    auth_documents = 0
    for path in root.rglob("*"):
        try:
            metadata = path.lstat()
        except OSError as exc:
            raise JobAuditError("job audit could not inspect a collected path") from exc
        if not stat.S_ISREG(metadata.st_mode):
            continue
        files_scanned += 1
        bytes_scanned += metadata.st_size
        if path.name == "auth.json":
            auth_documents += 1
        for name, needle in needles.items():
            if _contains(path, needle):
                hits[name] += 1
    report: dict[str, object] = {
        "schema_version": 1,
        "status": "ok" if not any(hits.values()) and auth_documents == 0 else "failed",
        "files_scanned": files_scanned,
        "bytes_scanned": bytes_scanned,
        "credential_value_hits": hits["credential_value"],
        "source_auth_path_hits": hits["source_auth_path"],
        "host_temp_auth_path_hits": hits["host_temp_auth_path"],
        "collected_auth_documents": auth_documents,
    }
    if report["status"] != "ok":
        raise JobAuditError("job credential audit failed; no credential content was printed")
    return report
