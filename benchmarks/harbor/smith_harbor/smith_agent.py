"""Harbor installed-agent bridge for a static Smith binary."""

from __future__ import annotations

import asyncio
import json
import shlex
import signal
import tempfile
from collections.abc import Mapping
from pathlib import Path, PurePosixPath
from typing import Any, override

from harbor.agents.installed.base import BaseInstalledAgent, NonZeroAgentExitCodeError
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext
from harbor.models.trajectories import Trajectory
from harbor.utils.trajectory_utils import format_trajectory_json

from smith_harbor.artifacts import (
    ArtifactError,
    VerifiedArtifact,
    target_for_machine,
    verify_artifact,
)
from smith_harbor.auth import (
    SelectedAuth,
    default_auth_path,
    merge_refreshed_auth,
    minimal_auth_file,
    select_auth,
)
from smith_harbor.constants import (
    APPROVAL,
    ARTIFACT_MANIFEST_PATH,
    BRIDGE_VERSION,
    EFFORT,
    HARBOR_MODEL,
    MODEL,
    PROVIDER,
    smith_config_toml,
)
from smith_harbor.protocol import ProtocolError, failure_atif, load_stream, to_atif
from smith_harbor.variants import load_variant, variant_provenance


class SmithAgent(BaseInstalledAgent):
    """Run Smith via schema-v3 headless output inside a Harbor task sandbox."""

    SUPPORTS_ATIF = True
    SUPPORTS_RESUME = False
    SUPPORTS_WINDOWS = False

    _REMOTE_BINARY = PurePosixPath("/installed-agent/smith")
    _REMOTE_STREAM = PurePosixPath("/logs/agent/smith-stream.jsonl")
    _REMOTE_STDERR = PurePosixPath("/logs/agent/smith-stderr.log")
    _REMOTE_EXIT = PurePosixPath("/logs/agent/smith-exit-code.txt")

    def __init__(
        self,
        logs_dir: Path,
        *,
        profile_name: str = "smoke",
        variant_name: str = "current",
        run_invariants: Mapping[str, object] | None = None,
        allow_concurrent_oauth: bool = False,
        **kwargs: Any,
    ) -> None:
        super().__init__(logs_dir, version=BRIDGE_VERSION, **kwargs)
        if self.model_name != HARBOR_MODEL:
            raise ValueError(f"Smith Harbor requires model_name={HARBOR_MODEL!r}")
        self.profile_name = profile_name
        self.variant = load_variant(variant_name)
        self.run_invariants = dict(run_invariants or {})
        self.allow_concurrent_oauth = allow_concurrent_oauth
        self._artifact: VerifiedArtifact | None = None
        self._selected_auth: SelectedAuth | None = None
        self._remote_auth_path: PurePosixPath | None = None

    @staticmethod
    @override
    def name() -> str:
        return "smith-harbor"

    @staticmethod
    def _exit_description(return_code: int) -> str:
        description = f"Smith returned bounded non-success exit code {return_code}"
        if 128 < return_code <= 255:
            signal_number = return_code - 128
            try:
                signal_name = signal.Signals(signal_number).name
            except ValueError:
                signal_name = "unknown"
            description += f" (signal {signal_number} {signal_name})"
        return description

    @staticmethod
    def _task_login_command(command: str) -> str:
        """Run Smith with the task image's login environment when available.

        Harbor task images commonly activate their project interpreter from a
        Bash login profile. Smith's shell tool intentionally uses ``sh -c`` and
        inherits Smith's process environment, so launching Smith directly can
        otherwise hide task-provided commands such as pytest. Keep a POSIX
        fallback for minimal images without Bash.
        """
        quoted = shlex.quote(command)
        return (
            "if command -v bash >/dev/null 2>&1; then "
            f"exec bash -lc {quoted}; "
            "else "
            f"exec sh -c {quoted}; "
            "fi"
        )

    async def _remote_identity(self, environment: BaseEnvironment) -> tuple[str, int, int]:
        result = await environment.exec(
            command="printf '%s\\n' \"$HOME\"; id -u; id -g",
            user=environment.default_user,
            timeout_sec=15,
        )
        if result.return_code != 0:
            raise RuntimeError("could not resolve the Harbor agent user's private home")
        lines = (result.stdout or "").splitlines()
        if len(lines) != 3 or not lines[0].startswith("/"):
            raise RuntimeError("Harbor agent user returned an invalid private home")
        try:
            uid, gid = int(lines[1]), int(lines[2])
        except ValueError as exc:
            raise RuntimeError("Harbor agent user returned an invalid identity") from exc
        return lines[0], uid, gid

    async def _ensure_runtime_dependencies(self, environment: BaseEnvironment) -> None:
        check = await environment.exec(
            command="test -r /etc/ssl/certs/ca-certificates.crt && command -v sha256sum >/dev/null",
            user="root",
            timeout_sec=15,
        )
        if check.return_code == 0:
            return
        command = (
            "if command -v apk >/dev/null 2>&1; then "
            "apk add --no-cache ca-certificates coreutils >/dev/null; "
            "elif command -v apt-get >/dev/null 2>&1; then "
            "apt-get update -qq && DEBIAN_FRONTEND=noninteractive "
            "apt-get install -y -qq ca-certificates coreutils >/dev/null; "
            "elif command -v dnf >/dev/null 2>&1; then "
            "dnf install -y ca-certificates coreutils >/dev/null; "
            "elif command -v yum >/dev/null 2>&1; then "
            "yum install -y ca-certificates coreutils >/dev/null; "
            "else exit 42; fi"
        )
        result = await environment.exec(command=command, user="root", timeout_sec=180)
        if result.return_code != 0:
            raise RuntimeError("sandbox lacks bounded TLS/digest runtime dependencies")

    async def _upload_private_config_and_auth(
        self,
        environment: BaseEnvironment,
        *,
        remote_home: str,
        uid: int,
        gid: int,
    ) -> SelectedAuth:
        selected = select_auth(default_auth_path(), "chatgpt")
        with tempfile.TemporaryDirectory(prefix="smith-harbor-config-") as raw_dir:
            directory = Path(raw_dir)
            directory.chmod(0o700)
            config_path = directory / "config.toml"
            config_path.write_text(
                smith_config_toml(
                    profile_instructions=self.variant.instructions,
                    delegation=self.variant.delegation,
                ),
                encoding="utf-8",
            )
            config_path.chmod(0o600)
            with minimal_auth_file(selected) as auth_path:
                await environment.upload_file(config_path, "/tmp/smith-harbor-config.toml")
                await environment.upload_file(auth_path, "/tmp/smith-harbor-auth.json")

        remote_smith = PurePosixPath(remote_home) / ".smith"
        install = (
            f"install -d -m 0700 -o {uid} -g {gid} {shlex.quote(str(remote_smith))}; "
            f"install -m 0600 -o {uid} -g {gid} /tmp/smith-harbor-config.toml "
            f"{shlex.quote(str(remote_smith / 'config.toml'))}; "
            f"install -m 0600 -o {uid} -g {gid} /tmp/smith-harbor-auth.json "
            f"{shlex.quote(str(remote_smith / 'auth.json'))}; "
            "rm -f /tmp/smith-harbor-config.toml /tmp/smith-harbor-auth.json"
        )
        result = await environment.exec(command=install, user="root", timeout_sec=30)
        if result.return_code != 0:
            raise RuntimeError("could not install Smith's private trial configuration")
        self._remote_auth_path = remote_smith / "auth.json"
        return selected

    async def _capture_refreshed_auth(self, environment: BaseEnvironment) -> None:
        """Return only the selected rotated entry to Smith's supported auth store."""
        if self._selected_auth is None or self._remote_auth_path is None:
            raise RuntimeError("Smith OAuth refresh handoff was not initialized")
        source_path = default_auth_path()
        with tempfile.TemporaryDirectory(prefix="smith-harbor-refresh-") as raw_dir:
            directory = Path(raw_dir)
            directory.chmod(0o700)
            downloaded = directory / "auth.json"
            try:
                await environment.download_file(
                    source_path=str(self._remote_auth_path),
                    target_path=downloaded,
                )
                downloaded.chmod(0o600)
                refreshed = select_auth(downloaded, self._selected_auth.entry)
                merge_refreshed_auth(
                    source_path,
                    refreshed,
                    expected=self._selected_auth,
                )
            except Exception:
                raise RuntimeError(
                    "Smith OAuth refresh handoff failed without logging credential content"
                ) from None
        self._selected_auth = refreshed

    async def _write_safe_provenance(self, environment: BaseEnvironment) -> None:
        assert self._artifact is not None
        safe = {
            "schema_version": 1,
            "bridge_version": BRIDGE_VERSION,
            "profile": self.profile_name,
            **variant_provenance(self.variant),
            "provider": PROVIDER,
            "model": MODEL,
            "effort": EFFORT,
            "approval": APPROVAL,
            "oauth_copy_policy": (
                "unsupported-risk-concurrent-refresh-handoff"
                if self.allow_concurrent_oauth
                else "serial-selected-entry-refresh-handoff"
            ),
            "smith_revision": self._artifact.revision,
            "smith_dirty": self._artifact.dirty,
            "artifact_target": self._artifact.target,
            "artifact_sha256": self._artifact.sha256,
            "artifact_version": self._artifact.version,
            "run_invariants": self.run_invariants,
            "cost_usd": None,
            "cost_provenance": "unknown_subscription_oauth",
        }
        with tempfile.TemporaryDirectory(prefix="smith-harbor-provenance-") as raw_dir:
            path = Path(raw_dir) / "provenance.json"
            path.write_text(json.dumps(safe, indent=2, sort_keys=True) + "\n", encoding="utf-8")
            await environment.upload_file(path, "/logs/agent/provenance.json")

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        system = await environment.exec(
            command="uname -s",
            user=environment.default_user,
            timeout_sec=10,
        )
        machine = await environment.exec(
            command="uname -m",
            user=environment.default_user,
            timeout_sec=10,
        )
        if system.return_code != 0 or (system.stdout or "").strip() != "Linux":
            raise RuntimeError("Smith Harbor supports Linux task sandboxes only")
        if machine.return_code != 0:
            raise RuntimeError("could not determine the Linux sandbox architecture")
        try:
            target = target_for_machine(machine.stdout or "")
            artifact = verify_artifact(ARTIFACT_MANIFEST_PATH, target)
        except ArtifactError as exc:
            raise RuntimeError(str(exc)) from exc
        self._artifact = artifact

        await self._ensure_runtime_dependencies(environment)
        await environment.upload_file(artifact.path, "/tmp/smith-harbor-binary")
        install_binary = await environment.exec(
            command=(
                "install -m 0755 /tmp/smith-harbor-binary /installed-agent/smith "
                "&& rm -f /tmp/smith-harbor-binary"
            ),
            user="root",
            timeout_sec=30,
        )
        if install_binary.return_code != 0:
            raise RuntimeError("could not install the verified Smith binary")
        verify = await environment.exec(
            command=(
                "actual=$(sha256sum /installed-agent/smith | cut -d ' ' -f 1); "
                f'test "$actual" = {shlex.quote(artifact.sha256)} && '
                "/installed-agent/smith --version"
            ),
            user=environment.default_user,
            timeout_sec=30,
        )
        if verify.return_code != 0 or (verify.stdout or "").strip() != artifact.version:
            raise RuntimeError("installed Smith digest/version verification failed")

        remote_home, uid, gid = await self._remote_identity(environment)
        self._selected_auth = await self._upload_private_config_and_auth(
            environment,
            remote_home=remote_home,
            uid=uid,
            gid=gid,
        )
        await self._write_safe_provenance(environment)

    @override
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        if self._artifact is None:
            raise RuntimeError("Smith artifact was not installed")
        with tempfile.TemporaryDirectory(prefix="smith-harbor-instruction-") as raw_dir:
            instruction_path = Path(raw_dir) / "instruction.txt"
            instruction_path.write_text(instruction, encoding="utf-8")
            await environment.upload_file(instruction_path, "/logs/agent/instruction.txt")
        smith_command = (
            f"{self._REMOTE_BINARY} --project . -p {shlex.quote(instruction)} "
            f"--provider {PROVIDER} --model {MODEL} --effort {EFFORT} "
            f"--approval {APPROVAL} --output-format stream-json "
            f">{self._REMOTE_STREAM} 2>{self._REMOTE_STDERR}; "
            "code=$?; "
            f"printf '%s\\n' \"$code\" >{self._REMOTE_EXIT}; "
            'if [ "$code" -eq 0 ]; then '
            f"terminal=$(tail -n 1 {self._REMOTE_STREAM}); "
            'case "$terminal" in *\'"schema_version":3\'*\'"type":"result"\'*'
            f'\'"status":"ok"\'*\'"provider":"{PROVIDER}"\'*'
            f'\'"model":"{MODEL}"\'*\'"effort":"{EFFORT}"\'*) ;; '
            "*) code=86; printf '%s\\n' "
            "'Smith terminal result violated pinned run invariants' >&2 ;; esac; fi; "
            'exit "$code"'
        )
        command = self._task_login_command(smith_command)
        try:
            result = await environment.exec(
                command=command,
                user=environment.default_user,
                env={"SMITH_PERSISTENCE_ENABLED": "false"},
            )
        except BaseException:
            try:
                await asyncio.shield(self._capture_refreshed_auth(environment))
            except Exception:
                pass
            raise
        await asyncio.shield(self._capture_refreshed_auth(environment))
        if result.return_code != 0:
            raise NonZeroAgentExitCodeError(
                f"{self._exit_description(result.return_code)}; "
                "inspect smith-stderr.log and converter-diagnostics.json"
            )

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        diagnostics: dict[str, object] = {"schema_version": 1, "status": "conversion_failed"}
        instruction_path = self.logs_dir / "instruction.txt"
        instruction = ""
        try:
            if instruction_path.exists():
                instruction = instruction_path.read_text(encoding="utf-8")
        except OSError:
            pass
        try:
            if self._artifact is None:
                raise ProtocolError("Smith artifact provenance is unavailable")
            stream_path = self.logs_dir / self._REMOTE_STREAM.name
            parsed = load_stream(stream_path)
            trajectory_dict = to_atif(
                parsed,
                instruction,
                smith_version=self._artifact.version,
                smith_revision=self._artifact.revision,
                artifact_sha256=self._artifact.sha256,
                run_invariants=self.run_invariants,
            )
            trajectory = Trajectory.model_validate(trajectory_dict)
            (self.logs_dir / "trajectory.json").write_text(
                format_trajectory_json(trajectory.to_json_dict()),
                encoding="utf-8",
            )
            context.n_input_tokens = parsed.harbor_input_tokens
            context.n_cache_tokens = parsed.harbor_cache_tokens
            context.n_output_tokens = parsed.harbor_output_tokens
            context.cost_usd = None
            context.metadata = trajectory.extra
            diagnostics = {
                "schema_version": 1,
                "status": "ok",
                "smith_status": parsed.result["status"],
                "cost_usd": None,
                "cost_provenance": "unknown_subscription_oauth",
            }
        except (OSError, ProtocolError, ValueError) as exc:
            diagnostics["error_type"] = type(exc).__name__
            diagnostics["message"] = str(exc)[:500]
            if self._artifact is not None:
                fallback = failure_atif(
                    instruction,
                    smith_version=self._artifact.version,
                    smith_revision=self._artifact.revision,
                    artifact_sha256=self._artifact.sha256,
                    failure_kind=type(exc).__name__,
                )
                trajectory = Trajectory.model_validate(fallback)
                (self.logs_dir / "trajectory.json").write_text(
                    format_trajectory_json(trajectory.to_json_dict()),
                    encoding="utf-8",
                )
                context.metadata = trajectory.extra
        (self.logs_dir / "converter-diagnostics.json").write_text(
            json.dumps(diagnostics, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
