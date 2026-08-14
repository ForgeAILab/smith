"""Owner-only Smith OAuth selection and isolated-copy helpers."""

from __future__ import annotations

import fcntl
import json
import os
import stat
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

from smith_harbor.constants import AUTH_FILE_MAX_BYTES


class AuthPreflightError(ValueError):
    """An auth file failed a bounded, content-safe preflight."""


@dataclass(frozen=True)
class SelectedAuth:
    """A validated selected credential, retained only in memory."""

    entry: str
    value: str

    def minimal_document(self) -> dict[str, object]:
        return {"schema_version": 1, "credentials": {self.entry: self.value}}


def default_auth_path() -> Path:
    """Resolve the supported host override without exposing it in Harbor config."""
    override = os.environ.get("SMITH_HARBOR_AUTH_FILE")
    return Path(override).expanduser() if override else Path.home() / ".smith" / "auth.json"


def _fail(reason: str) -> AuthPreflightError:
    return AuthPreflightError(f"Smith OAuth auth file {reason}; no credential content was read")


def _load_auth_document(path: Path) -> dict[str, object]:
    """Load a private schema-v1 document without including its contents in errors."""
    try:
        metadata = path.lstat()
    except OSError as exc:
        raise _fail("is unavailable") from exc
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise _fail("must be a regular non-symlink file")
    if hasattr(os, "getuid") and metadata.st_uid != os.getuid():
        raise _fail("must be owned by the current user")
    if stat.S_IMODE(metadata.st_mode) & 0o077:
        raise _fail("must not grant group or other permissions")
    if metadata.st_size > AUTH_FILE_MAX_BYTES:
        raise _fail("exceeds the 1 MiB size limit")

    try:
        with path.open("rb") as handle:
            payload = handle.read(AUTH_FILE_MAX_BYTES + 1)
    except OSError as exc:
        raise _fail("could not be read") from exc
    if len(payload) > AUTH_FILE_MAX_BYTES:
        raise _fail("exceeds the 1 MiB size limit")
    try:
        document = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise _fail("is not valid UTF-8 schema-v1 JSON") from exc
    if not isinstance(document, dict) or document.get("schema_version") != 1:
        raise _fail("does not use schema version 1")
    credentials = document.get("credentials")
    if not isinstance(credentials, dict):
        raise _fail("does not contain a credentials object")
    return document


def select_auth(path: Path, entry: str = "chatgpt") -> SelectedAuth:
    """Validate a Smith schema-v1 auth file and select exactly one string entry."""
    document = _load_auth_document(path)
    credentials = document["credentials"]
    assert isinstance(credentials, dict)
    value = credentials.get(entry)
    if not isinstance(value, str) or not value:
        raise AuthPreflightError(
            f"Smith OAuth entry {entry!r} is unavailable; no credential content was logged"
        )
    return SelectedAuth(entry=entry, value=value)


def _open_private_lock(path: Path) -> int:
    flags = os.O_RDWR | os.O_CREAT
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        descriptor = os.open(path, flags, 0o600)
    except OSError as exc:
        raise _fail("refresh lock is unavailable") from exc
    metadata = os.fstat(descriptor)
    if (
        not stat.S_ISREG(metadata.st_mode)
        or (hasattr(os, "getuid") and metadata.st_uid != os.getuid())
        or stat.S_IMODE(metadata.st_mode) & 0o077
    ):
        os.close(descriptor)
        raise _fail("refresh lock must be a private regular file")
    return descriptor


def merge_refreshed_auth(
    path: Path,
    refreshed: SelectedAuth,
    *,
    expected: SelectedAuth,
) -> bool:
    """Atomically merge one rotated OAuth entry while preserving the host document.

    The adjacent owner-only lock serializes harness writers. The expected value is
    checked under that lock so an uncooperative concurrent writer is never
    overwritten. Credential values are intentionally absent from every diagnostic.
    """
    if refreshed.entry != expected.entry:
        raise AuthPreflightError(
            "Smith OAuth refresh selected an unexpected entry; no credential content was logged"
        )

    lock_path = path.with_name(f".{path.name}.smith-harbor.lock")
    lock_descriptor = _open_private_lock(lock_path)
    temporary_path: Path | None = None
    try:
        fcntl.flock(lock_descriptor, fcntl.LOCK_EX)
        document = _load_auth_document(path)
        credentials = document["credentials"]
        assert isinstance(credentials, dict)
        current_value = credentials.get(expected.entry)
        if not isinstance(current_value, str) or not current_value:
            raise AuthPreflightError(
                "Smith OAuth refresh target is unavailable; no credential content was logged"
            )
        if current_value == refreshed.value:
            return False
        if current_value != expected.value:
            raise AuthPreflightError(
                "Smith OAuth refresh target changed concurrently; no credential content was logged"
            )
        if refreshed.value == expected.value:
            return False

        credentials[expected.entry] = refreshed.value
        descriptor, raw_path = tempfile.mkstemp(
            prefix=f".{path.name}.smith-harbor-",
            dir=path.parent,
        )
        temporary_path = Path(raw_path)
        try:
            rendered = (json.dumps(document, separators=(",", ":")) + "\n").encode()
            with os.fdopen(descriptor, "wb") as handle:
                handle.write(rendered)
                handle.flush()
                os.fsync(handle.fileno())
        except BaseException:
            temporary_path.unlink(missing_ok=True)
            raise
        os.replace(temporary_path, path)
        temporary_path = None
        directory_flags = os.O_RDONLY
        if hasattr(os, "O_DIRECTORY"):
            directory_flags |= os.O_DIRECTORY
        directory_descriptor = os.open(path.parent, directory_flags)
        try:
            os.fsync(directory_descriptor)
        finally:
            os.close(directory_descriptor)
        return True
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)
        fcntl.flock(lock_descriptor, fcntl.LOCK_UN)
        os.close(lock_descriptor)


@contextmanager
def minimal_auth_file(selected: SelectedAuth) -> Iterator[Path]:
    """Yield a private temporary auth file and remove it immediately afterward."""
    with tempfile.TemporaryDirectory(prefix="smith-harbor-auth-") as raw_dir:
        directory = Path(raw_dir)
        directory.chmod(0o700)
        path = directory / "auth.json"
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
                json.dump(selected.minimal_document(), handle, separators=(",", ":"))
                handle.write("\n")
        except BaseException:
            path.unlink(missing_ok=True)
            raise
        try:
            yield path
        finally:
            path.unlink(missing_ok=True)
