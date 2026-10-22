"""Credential-content audit for a completed local Harbor job."""

from __future__ import annotations

import stat
from pathlib import Path

from smith_harbor.auth import (
    SelectedAuth,
    default_auth_path,
    default_config_path,
    resolve_provider_api_key,
    select_auth,
)
from smith_harbor.constants import load_serving


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
    serving_name: str | None = None,
) -> dict[str, object]:
    """Scan regular job files without printing credential values or source paths."""
    root = job_path.expanduser().resolve()
    if not root.is_dir() or not (root / "result.json").is_file():
        raise JobAuditError("job audit target is not a completed Harbor job directory")
    serving = load_serving(serving_name)
    needles: dict[str, bytes] = {}
    if serving.oauth:
        source = source_auth_path or default_auth_path()
        credential = selected or select_auth(source, "chatgpt")
        needles = {
            "credential_value": credential.value.encode(),
            "source_auth_path": str(source.expanduser()).encode(),
            "host_temp_auth_path": b"smith-harbor-auth-",
        }
    else:
        if serving.credential_env_var is None:
            raise JobAuditError("API-key serving lacks a credential environment variable")
        api_key = resolve_provider_api_key(serving.credential_env_var, serving.provider)
        needles = {
            "credential_value": api_key.encode(),
            "source_config_path": str(default_config_path().expanduser()).encode(),
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
        "serving": serving.name,
        "status": "ok" if not any(hits.values()) and auth_documents == 0 else "failed",
        "files_scanned": files_scanned,
        "bytes_scanned": bytes_scanned,
        "credential_value_hits": hits["credential_value"],
        "collected_auth_documents": auth_documents,
    }
    if serving.oauth:
        report["source_auth_path_hits"] = hits["source_auth_path"]
        report["host_temp_auth_path_hits"] = hits["host_temp_auth_path"]
    else:
        report["source_config_path_hits"] = hits["source_config_path"]
    if report["status"] != "ok":
        raise JobAuditError("job credential audit failed; no credential content was printed")
    return report
