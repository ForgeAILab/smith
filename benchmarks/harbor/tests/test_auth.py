from __future__ import annotations

import json
import stat
from pathlib import Path

import pytest

from smith_harbor.auth import (
    AuthPreflightError,
    SelectedAuth,
    merge_refreshed_auth,
    minimal_auth_file,
    select_auth,
)


def _write_auth(path: Path, *, mode: int = 0o600) -> None:
    path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "credentials": {"chatgpt": "oauth-secret", "unused": "must-not-copy"},
                "unrelated": {"state": "must-not-copy"},
            }
        ),
        encoding="utf-8",
    )
    path.chmod(mode)


def test_minimal_auth_copy_contains_only_selected_entry(tmp_path: Path) -> None:
    source = tmp_path / "auth.json"
    _write_auth(source)

    selected = select_auth(source)
    with minimal_auth_file(selected) as isolated:
        document = json.loads(isolated.read_text(encoding="utf-8"))
        assert document == {
            "schema_version": 1,
            "credentials": {"chatgpt": "oauth-secret"},
        }
        assert stat.S_IMODE(isolated.stat().st_mode) == 0o600
        assert stat.S_IMODE(isolated.parent.stat().st_mode) == 0o700
        isolated_path = isolated

    assert not isolated_path.exists()


@pytest.mark.parametrize("mode", [0o604, 0o640, 0o666])
def test_auth_preflight_rejects_group_or_other_access(tmp_path: Path, mode: int) -> None:
    source = tmp_path / "auth.json"
    _write_auth(source, mode=mode)

    with pytest.raises(AuthPreflightError, match="group or other"):
        select_auth(source)


def test_auth_preflight_rejects_symlink_without_reading_target(tmp_path: Path) -> None:
    target = tmp_path / "target.json"
    _write_auth(target)
    link = tmp_path / "auth.json"
    link.symlink_to(target)

    with pytest.raises(AuthPreflightError, match="non-symlink"):
        select_auth(link)


def test_auth_preflight_rejects_oversized_file(tmp_path: Path) -> None:
    source = tmp_path / "auth.json"
    source.write_bytes(b"x" * (1024 * 1024 + 1))
    source.chmod(0o600)

    with pytest.raises(AuthPreflightError, match="1 MiB"):
        select_auth(source)


def test_refresh_merge_updates_only_selected_entry_atomically(tmp_path: Path) -> None:
    source = tmp_path / "auth.json"
    _write_auth(source)
    expected = select_auth(source)

    changed = merge_refreshed_auth(
        source,
        SelectedAuth(entry="chatgpt", value="rotated-secret"),
        expected=expected,
    )

    assert changed is True
    document = json.loads(source.read_text(encoding="utf-8"))
    assert document == {
        "schema_version": 1,
        "credentials": {
            "chatgpt": "rotated-secret",
            "unused": "must-not-copy",
        },
        "unrelated": {"state": "must-not-copy"},
    }
    assert stat.S_IMODE(source.stat().st_mode) == 0o600
    lock = tmp_path / ".auth.json.smith-harbor.lock"
    assert stat.S_IMODE(lock.stat().st_mode) == 0o600


def test_refresh_merge_rejects_concurrent_selected_entry_change(tmp_path: Path) -> None:
    source = tmp_path / "auth.json"
    _write_auth(source)
    expected = select_auth(source)
    document = json.loads(source.read_text(encoding="utf-8"))
    document["credentials"]["chatgpt"] = "concurrent-secret"
    source.write_text(json.dumps(document), encoding="utf-8")
    source.chmod(0o600)

    with pytest.raises(AuthPreflightError, match="changed concurrently"):
        merge_refreshed_auth(
            source,
            SelectedAuth(entry="chatgpt", value="rotated-secret"),
            expected=expected,
        )

    assert select_auth(source).value == "concurrent-secret"


def test_refresh_merge_is_noop_when_bundle_is_unchanged(tmp_path: Path) -> None:
    source = tmp_path / "auth.json"
    _write_auth(source)
    selected = select_auth(source)

    assert merge_refreshed_auth(source, selected, expected=selected) is False
