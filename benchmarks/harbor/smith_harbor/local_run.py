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
    resolve_provider_api_key,
    select_auth,
)
from smith_harbor.constants import (
    REPOSITORY_ROOT,
    Serving,
    load_serving,
    smith_config_toml,
)
from smith_harbor.protocol import ParsedRun, base_footprint_report, parse_stream
from smith_harbor.variants import load_variant, variant_provenance

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
    serving_name: str | None = None,
) -> IsolatedRun:
    serving = load_serving(serving_name)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError("local Smith binary is unavailable or not executable")
    api_key: str | None = None
    credential_values: set[str] = set()
    auth_path_source = default_auth_path()
    if serving.oauth:
        selected = select_auth(auth_path_source, "chatgpt")
        credential_values.add(selected.value)
    else:
        if serving.credential_env_var is None:
            raise RuntimeError("API-key serving lacks a credential environment variable")
        api_key = resolve_provider_api_key(serving.credential_env_var, serving.provider)
        credential_values.add(api_key)
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
                serving=serving,
            ),
            encoding="utf-8",
        )
        (smith_home / "config.toml").chmod(0o600)

        def _merge_refreshed() -> None:
            if serving.oauth:
                refreshed = select_auth(target_auth, selected.entry)
                merge_refreshed_auth(auth_path_source, refreshed, expected=selected)
                credential_values.add(refreshed.value)

        target_auth = smith_home / "auth.json"
        if serving.oauth:
            with minimal_auth_file(selected) as source_auth:
                shutil.copyfile(source_auth, target_auth)
                target_auth.chmod(0o600)

        environment = {
            key: value for key, value in os.environ.items() if not key.upper().startswith("SMITH_")
        }
        environment.update({"HOME": str(home), "SMITH_PERSISTENCE_ENABLED": "false"})
        if api_key is not None and serving.credential_env_var is not None:
            environment[serving.credential_env_var] = api_key
        try:
            completed = subprocess.run(
                [
                    binary,
                    "--project",
                    project,
                    "-p",
                    prompt,
                    "--provider",
                    serving.provider,
                    "--model",
                    serving.model,
                    *(["--effort", serving.effort] if serving.effort else []),
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
            _merge_refreshed()
        if any(
            value in completed.stdout or value in completed.stderr for value in credential_values
        ):
            raise RuntimeError("Smith canary output contained credential material")
        parsed = parse_stream(completed.stdout.splitlines(), serving=serving)
        return IsolatedRun(
            parsed=parsed,
            stdout=completed.stdout,
            stderr=completed.stderr,
            return_code=completed.returncode,
        )


def canary_report(
    run: IsolatedRun,
    *,
    variant_name: str = "current",
    serving: Serving | None = None,
) -> dict[str, object]:
    selected_serving = serving or load_serving()
    parsed = run.parsed
    if run.return_code != 0 or parsed.result["status"] != "ok":
        raise RuntimeError(
            f"{selected_serving.name} canary failed with Smith status {parsed.result['status']!r}"
        )
    if parsed.result["provider"] != selected_serving.provider:
        raise RuntimeError(
            f"{selected_serving.name} canary reported provider {parsed.result['provider']!r}"
        )
    if parsed.result["model"] != selected_serving.model:
        raise RuntimeError(
            f"{selected_serving.name} canary reported model {parsed.result['model']!r}"
        )
    if not parsed.usage_known or parsed.harbor_input_tokens is None:
        raise RuntimeError(
            f"{selected_serving.name} canary did not report provider-attributed usage"
        )
    output = str(parsed.result.get("output", ""))
    return {
        "schema_version": 1,
        "status": "ok",
        "serving": selected_serving.name,
        "provider": parsed.result["provider"],
        "model": parsed.result["model"],
        "reasoning": parsed.result["reasoning"],
        "usage": parsed.usage,
        "usage_provenance": parsed.result["usage"]["current_turn_provenance"],
        "output_sha256": hashlib.sha256(output.encode()).hexdigest(),
        "output_bytes": len(output.encode()),
        "cost_usd": None,
        "cost_provenance": (
            "unknown_subscription_oauth"
            if selected_serving.oauth
            else "per_token_api_key_metered"
        ),
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
    binary: Path,
    output: Path | None = None,
    *,
    variant_name: str = "current",
    serving_name: str | None = None,
) -> dict[str, object]:
    serving = load_serving(serving_name)
    run = run_isolated(
        binary,
        serving.canary_prompt,
        variant_name=variant_name,
        serving_name=serving.name,
    )
    report = canary_report(run, variant_name=variant_name, serving=serving)
    write_report(report, output)
    return report


def run_base_probe(
    binary: Path, output: Path | None = None, *, serving_name: str | None = None
) -> dict[str, object]:
    run = run_isolated(binary, BASE_PROBE_PROMPT, serving_name=serving_name)
    if run.return_code != 0:
        raise RuntimeError("base-footprint probe did not complete successfully")
    report = base_footprint_report(run.parsed)
    write_report(report, output)
    return report
