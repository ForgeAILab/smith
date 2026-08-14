"""Run a live Smith OAuth canary or base-footprint probe in an isolated home."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

from smith_harbor.auth import (
    default_auth_path,
    merge_refreshed_auth,
    minimal_auth_file,
    select_auth,
)
from smith_harbor.constants import EFFORT, MODEL, PROVIDER, REPOSITORY_ROOT, smith_config_toml
from smith_harbor.protocol import ParsedRun, base_footprint_report, parse_stream
from smith_harbor.variants import load_variant, variant_provenance

CANARY_PROMPT = "Reply with exactly SMITH_LUNA_MAX_CANARY_OK and nothing else."
BASE_PROBE_PROMPT = "Reply with exactly OK."


@dataclass(frozen=True)
class IsolatedRun:
    parsed: ParsedRun
    stdout: str
    stderr: str
    return_code: int


def default_local_binary() -> Path:
    return REPOSITORY_ROOT / "target" / "debug" / "smith"


def run_isolated(
    binary: Path,
    prompt: str,
    *,
    request_output_tokens: int = 512,
    timeout_seconds: int = 300,
    variant_name: str = "current",
) -> IsolatedRun:
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError("local Smith binary is unavailable or not executable")
    auth_path = default_auth_path()
    selected = select_auth(auth_path, "chatgpt")
    with tempfile.TemporaryDirectory(prefix="smith-harbor-canary-") as raw_root:
        root = Path(raw_root)
        home = root / "home"
        project = root / "project"
        smith_home = home / ".smith"
        smith_home.mkdir(parents=True, mode=0o700)
        project.mkdir(mode=0o700)
        variant = load_variant(variant_name)
        (smith_home / "config.toml").write_text(
            smith_config_toml(
                request_output_tokens,
                profile_instructions=variant.instructions,
                delegation=variant.delegation,
            ),
            encoding="utf-8",
        )
        (smith_home / "config.toml").chmod(0o600)
        with minimal_auth_file(selected) as source_auth:
            target_auth = smith_home / "auth.json"
            shutil.copyfile(source_auth, target_auth)
            target_auth.chmod(0o600)

        environment = {
            key: value for key, value in os.environ.items() if not key.upper().startswith("SMITH_")
        }
        environment.update({"HOME": str(home), "SMITH_PERSISTENCE_ENABLED": "false"})
        try:
            completed = subprocess.run(
                [
                    binary,
                    "--project",
                    project,
                    "-p",
                    prompt,
                    "--provider",
                    PROVIDER,
                    "--model",
                    MODEL,
                    "--effort",
                    EFFORT,
                    "--approval",
                    "allow-all",
                    "--output-format",
                    "stream-json",
                ],
                stdin=subprocess.DEVNULL,
                capture_output=True,
                text=True,
                env=environment,
                timeout=timeout_seconds,
                check=False,
            )
        finally:
            refreshed = select_auth(target_auth, selected.entry)
            merge_refreshed_auth(auth_path, refreshed, expected=selected)
        credential_values = {selected.value, refreshed.value}
        if any(
            value in completed.stdout or value in completed.stderr for value in credential_values
        ):
            raise RuntimeError("Smith canary output contained credential material")
        parsed = parse_stream(completed.stdout.splitlines())
        return IsolatedRun(
            parsed=parsed,
            stdout=completed.stdout,
            stderr=completed.stderr,
            return_code=completed.returncode,
        )


def canary_report(run: IsolatedRun, *, variant_name: str = "current") -> dict[str, object]:
    parsed = run.parsed
    if run.return_code != 0 or parsed.result["status"] != "ok":
        raise RuntimeError(f"Luna Max canary failed with Smith status {parsed.result['status']!r}")
    if not parsed.usage_known or parsed.harbor_input_tokens is None:
        raise RuntimeError("Luna Max canary did not report provider-attributed usage")
    output = str(parsed.result.get("output", ""))
    return {
        "schema_version": 1,
        "status": "ok",
        "provider": parsed.result["provider"],
        "model": parsed.result["model"],
        "reasoning": parsed.result["reasoning"],
        "usage": parsed.usage,
        "usage_provenance": parsed.result["usage"]["current_turn_provenance"],
        "output_sha256": hashlib.sha256(output.encode()).hexdigest(),
        "output_bytes": len(output.encode()),
        "cost_usd": None,
        "cost_provenance": "unknown_subscription_oauth",
        **variant_provenance(load_variant(variant_name)),
    }


def write_report(report: dict[str, object], output: Path | None) -> None:
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if output is None:
        print(rendered, end="")
        return
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_suffix(output.suffix + ".tmp")
    temporary.write_text(rendered, encoding="utf-8")
    os.replace(temporary, output)


def run_canary(
    binary: Path, output: Path | None = None, *, variant_name: str = "current"
) -> dict[str, object]:
    report = canary_report(
        run_isolated(binary, CANARY_PROMPT, variant_name=variant_name),
        variant_name=variant_name,
    )
    write_report(report, output)
    return report


def run_base_probe(binary: Path, output: Path | None = None) -> dict[str, object]:
    run = run_isolated(binary, BASE_PROBE_PROMPT)
    if run.return_code != 0:
        raise RuntimeError("base-footprint probe did not complete successfully")
    report = base_footprint_report(run.parsed)
    write_report(report, output)
    return report
