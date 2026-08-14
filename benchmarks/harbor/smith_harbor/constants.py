"""Stable identities and limits for the first Smith Harbor evaluation."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

BRIDGE_VERSION = "0.1.1"
HARBOR_VERSION = "0.20.0"
DATASET_NAME = "harbor-index/harbor-index-1.0"
DATASET_REF = "sha256:9d4514cb93f6fafd9cf8ff352c784495ab675176c7f09671db523bd19b663584"
DATASET_TASK_COUNT = 82
PROVIDER = "chatgpt"
PROVIDER_KIND = "chatgpt-responses"
MODEL = "gpt-5.6-luna"
HARBOR_MODEL = f"{PROVIDER}/{MODEL}"
EFFORT = "max"
APPROVAL = "allow-all"
OUTPUT_SCHEMA_VERSION = 3
RUNTIME_EVENT_SCHEMA_VERSION = 13
ATIF_SCHEMA_VERSION = "ATIF-v1.7"

CONTEXT_TOKENS = 272_000
MAX_INPUT_TOKENS = 255_616
MAX_OUTPUT_TOKENS = 16_384
AUTH_FILE_MAX_BYTES = 1024 * 1024

PACKAGE_ROOT = Path(__file__).resolve().parent.parent
REPOSITORY_ROOT = PACKAGE_ROOT.parent.parent
ARTIFACTS_DIR = PACKAGE_ROOT / "artifacts"
ARTIFACT_MANIFEST_PATH = ARTIFACTS_DIR / "manifest.json"
PROFILES_DIR = PACKAGE_ROOT / "profiles"
DATASET_MANIFEST_PATH = PROFILES_DIR / "harbor-index-1.0.json"


def default_jobs_dir() -> Path:
    """Choose a Docker Desktop-shared job root unless the operator overrides it."""
    override = os.environ.get("SMITH_HARBOR_JOBS_DIR")
    if override:
        return Path(override).expanduser()
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Caches" / "smith-harbor" / "jobs"
    return PACKAGE_ROOT / "jobs"


SEGMENT_KINDS = (
    "system_instruction",
    "developer_instruction",
    "ability_instruction",
    "tool_schema",
    "memory",
    "retrieval",
    "history",
    "tool_result",
    "user_input",
    "continuation",
    "summary",
)

SUPPORTED_TARGETS = {
    "x86_64-unknown-linux-musl": {
        "docker_platform": "linux/amd64",
        "uname_machine": ("x86_64", "amd64"),
        "elf_machine": 62,
    },
    "aarch64-unknown-linux-musl": {
        "docker_platform": "linux/arm64",
        "uname_machine": ("aarch64", "arm64"),
        "elf_machine": 183,
    },
}


def smith_config_toml(
    request_output_tokens: int = MAX_OUTPUT_TOKENS,
    *,
    profile_instructions: str | None = None,
    delegation: bool = True,
) -> str:
    """Return the benchmark-local Smith configuration without credential values."""
    if not 1 <= request_output_tokens <= MAX_OUTPUT_TOKENS:
        raise ValueError("request output tokens exceed the benchmark model limit")
    instruction_line = (
        f"instructions = {json.dumps(profile_instructions)}\n"
        if profile_instructions is not None
        else ""
    )
    delegation_line = f"delegation = {str(delegation).lower()}\n"
    return f'''default_profile = "harbor"
profile_order = ["harbor"]

[profiles.harbor]
provider = "{PROVIDER}"
model = "{MODEL}"
max_output_tokens = {request_output_tokens}
{delegation_line}{instruction_line}

[providers.{PROVIDER}]
kind = "{PROVIDER_KIND}"
base_url = "https://chatgpt.com/backend-api/codex"
credential = "authfile:chatgpt"

[models."{PROVIDER}/{MODEL}"]
context_tokens = {CONTEXT_TOKENS}
max_input_tokens = {MAX_INPUT_TOKENS}
max_output_tokens = {MAX_OUTPUT_TOKENS}

[models."{PROVIDER}/{MODEL}".reasoning]
mandatory = true
efforts = ["none", "low", "medium", "high", "xhigh", "max"]
default_enabled = true
default_effort = "medium"
dialect = "openai-effort"

[context]
output_reserve = {request_output_tokens}
reasoning_reserve = 0
capability_budget = 12000

[limits]
max_retries = 2
max_tool_steps = 0
turn_time_limit_ms = 0
tool_output_limit_bytes = 65536

[persistence]
enabled = false
'''
