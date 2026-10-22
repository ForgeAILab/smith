"""Stable identities and limits for the first Smith Harbor evaluation."""

from __future__ import annotations

import dataclasses
import json
import os
import sys
from pathlib import Path

BRIDGE_VERSION = "0.1.1"
HARBOR_VERSION = "0.20.0"
DATASET_NAME = "harbor-index/harbor-index-1.0"
DATASET_REF = "sha256:9d4514cb93f6fafd9cf8ff352c784495ab675176c7f09671db523bd19b663584"
DATASET_TASK_COUNT = 82
APPROVAL = "allow-all"
OUTPUT_SCHEMA_VERSION = 3
RUNTIME_EVENT_SCHEMA_VERSION = 16
ATIF_SCHEMA_VERSION = "ATIF-v1.7"

CONTEXT_TOKENS = 272_000
MAX_INPUT_TOKENS = 255_616
MAX_OUTPUT_TOKENS = 16_384
AUTH_FILE_MAX_BYTES = 1024 * 1024

ZAI_BASE_URL = "https://api.z.ai/api/coding/paas/v4"
ZAI_CREDENTIAL_ENV_VAR = "ZAI_API_KEY"
CHATGPT_BASE_URL = "https://chatgpt.com/backend-api/codex"


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


@dataclasses.dataclass(frozen=True)
class Serving:
    """A pinned model serving identity for benchmark runs.

    A serving names everything a run needs besides the task set: provider,
    credential reference, model limits, reasoning policy, and egress hosts.
    Credential references are always references; values never appear here.
    """

    name: str
    provider: str
    provider_kind: str
    base_url: str
    model: str
    credential: str
    effort: str | None
    allowed_hosts: tuple[str, ...]
    oauth: bool
    canary_prompt: str
    canary_output: str
    context_tokens: int = CONTEXT_TOKENS
    max_input_tokens: int = MAX_INPUT_TOKENS
    max_output_tokens: int = MAX_OUTPUT_TOKENS
    credential_env_var: str | None = None
    reasoning_only_text: bool = False

    @property
    def harbor_model(self) -> str:
        return f"{self.provider}/{self.model}"

    @property
    def credential_policy(self) -> str:
        if self.oauth:
            return "oauth_authfile_selected_entry"
        return f"env_{self.credential_env_var}"

    def config_toml(self, request_output_tokens: int | None = None) -> str:
        """Return the benchmark-local Smith configuration without credential values."""
        requested = (
            self.max_output_tokens if request_output_tokens is None else request_output_tokens
        )
        if not 1 <= requested <= self.max_output_tokens:
            raise ValueError("request output tokens exceed the serving model limit")
        response_block = (
            f'\n[providers.{self.provider}.response]\nreasoning_only = "text"\n'
            if self.reasoning_only_text
            else ""
        )
        reasoning_block = (
            f"""
[models."{self.harbor_model}".reasoning]
mandatory = true
efforts = ["none", "low", "medium", "high", "xhigh", "max"]
default_enabled = true
default_effort = "medium"
dialect = "openai-effort"
"""
            if self.effort is not None
            else ""
        )
        return f'''default_profile = "harbor"
profile_order = ["harbor"]

[profiles.harbor]
provider = "{self.provider}"
model = "{self.model}"
max_output_tokens = {request_output_tokens}

[providers.{self.provider}]
kind = "{self.provider_kind}"
base_url = "{self.base_url}"
credential = "{self.credential}"{response_block}
[models."{self.harbor_model}"]
context_tokens = {self.context_tokens}
max_input_tokens = {self.max_input_tokens}
max_output_tokens = {self.max_output_tokens}
{reasoning_block}
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


SERVINGS: dict[str, Serving] = {
    "luna-max": Serving(
        name="luna-max",
        provider="chatgpt",
        provider_kind="chatgpt-responses",
        base_url=CHATGPT_BASE_URL,
        model="gpt-5.6-luna",
        credential="authfile:chatgpt",
        effort="max",
        allowed_hosts=("auth.openai.com", "chatgpt.com"),
        oauth=True,
        canary_prompt="Reply with exactly SMITH_LUNA_MAX_CANARY_OK and nothing else.",
        canary_output="SMITH_LUNA_MAX_CANARY_OK",
    ),
    "glm-5.3": Serving(
        name="glm-5.3",
        provider="zai",
        provider_kind="openai-compatible",
        base_url=ZAI_BASE_URL,
        model="glm-5.3",
        credential=f"env:{ZAI_CREDENTIAL_ENV_VAR}",
        effort=None,
        allowed_hosts=("api.z.ai",),
        oauth=False,
        canary_prompt="Reply with exactly SMITH_GLM_53_CANARY_OK and nothing else.",
        canary_output="SMITH_GLM_53_CANARY_OK",
        # Identical input budget to luna-max; the output ceiling is GLM's
        # native limit because its text-mode reasoning consumes it, and context
        # is exactly input plus output reserve, mirroring luna's arithmetic.
        context_tokens=386_688,
        max_input_tokens=MAX_INPUT_TOKENS,
        max_output_tokens=131_072,
        credential_env_var=ZAI_CREDENTIAL_ENV_VAR,
        reasoning_only_text=True,
    ),
}

DEFAULT_SERVING_NAME = "luna-max"

# Backwards-compatible module identities: the default serving's pinned values.
PROVIDER = SERVINGS[DEFAULT_SERVING_NAME].provider
PROVIDER_KIND = SERVINGS[DEFAULT_SERVING_NAME].provider_kind
MODEL = SERVINGS[DEFAULT_SERVING_NAME].model
HARBOR_MODEL = SERVINGS[DEFAULT_SERVING_NAME].harbor_model
EFFORT = SERVINGS[DEFAULT_SERVING_NAME].effort


def load_serving(name: str | None = None) -> Serving:
    """Resolve a serving by name, failing closed on unknown names."""
    try:
        return SERVINGS[name or DEFAULT_SERVING_NAME]
    except KeyError:
        raise ValueError(f"unknown Smith Harbor serving {name!r}") from None


def smith_config_toml(
    request_output_tokens: int | None = None,
    *,
    profile_instructions: str | None = None,
    delegation: bool = True,
    serving: Serving | None = None,
) -> str:
    """Return the benchmark-local Smith configuration without credential values."""
    selected = serving or load_serving()
    requested = (
        selected.max_output_tokens
        if request_output_tokens is None
        else request_output_tokens
    )
    config = selected.config_toml(requested)
    instruction_line = (
        f"instructions = {json.dumps(profile_instructions)}\n"
        if profile_instructions is not None
        else ""
    )
    delegation_line = f"delegation = {str(delegation).lower()}\n"
    anchor = f"max_output_tokens = {requested}\n"
    return config.replace(anchor, anchor + delegation_line + instruction_line, 1)
