from __future__ import annotations

from pathlib import Path

import pytest

from smith_harbor.audit import JobAuditError, audit_job
from smith_harbor.auth import SelectedAuth


def test_job_audit_reports_safe_counts_without_credential_content(tmp_path: Path) -> None:
    job = tmp_path / "job"
    job.mkdir()
    (job / "result.json").write_text("{}", encoding="utf-8")
    (job / "trajectory.json").write_text("safe", encoding="utf-8")

    report = audit_job(
        job,
        selected=SelectedAuth("chatgpt", "oauth-secret"),
        source_auth_path=tmp_path / "source-auth.json",
    )

    assert report["status"] == "ok"
    assert report["credential_value_hits"] == 0
    assert report["collected_auth_documents"] == 0


@pytest.mark.parametrize(
    ("filename", "content"),
    [("leak.txt", "oauth-secret"), ("auth.json", "not-the-secret")],
)
def test_job_audit_fails_closed_on_oauth_material(
    tmp_path: Path, filename: str, content: str
) -> None:
    job = tmp_path / "job"
    job.mkdir()
    (job / "result.json").write_text("{}", encoding="utf-8")
    (job / filename).write_text(content, encoding="utf-8")

    with pytest.raises(JobAuditError, match="no credential content was printed"):
        audit_job(
            job,
            selected=SelectedAuth("chatgpt", "oauth-secret"),
            source_auth_path=tmp_path / "source-auth.json",
        )
