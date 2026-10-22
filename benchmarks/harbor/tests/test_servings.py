from __future__ import annotations

import json
import tomllib
from pathlib import Path

import pytest

from smith_harbor.constants import (
    MAX_INPUT_TOKENS,
    SERVINGS,
    ZAI_CREDENTIAL_ENV_VAR,
    load_serving,
    smith_config_toml,
)


def test_serving_table_pins_both_servings_and_shared_token_budgets() -> None:
    luna = load_serving("luna-max")
    glm = load_serving("glm-5.3")

    assert luna.provider == "chatgpt"
    assert luna.model == "gpt-5.6-luna"
    assert luna.effort == "max"
    assert luna.oauth is True
    assert luna.credential == "authfile:chatgpt"
    assert luna.allowed_hosts == ("auth.openai.com", "chatgpt.com")

    assert glm.provider == "zai"
    assert glm.model == "glm-5.3"
    assert glm.effort is None
    assert glm.oauth is False
    assert glm.credential == f"env:{ZAI_CREDENTIAL_ENV_VAR}"
    assert glm.allowed_hosts == ("api.z.ai",)
    assert glm.credential_env_var == ZAI_CREDENTIAL_ENV_VAR

    # Identical input budgets; output ceilings are each model's native limit
    # and context is exactly input plus the output reserve.
    assert luna.max_input_tokens == glm.max_input_tokens == MAX_INPUT_TOKENS
    assert luna.max_output_tokens == 16_384
    assert glm.max_output_tokens == 131_072
    assert luna.context_tokens == luna.max_input_tokens + luna.max_output_tokens
    assert glm.context_tokens == glm.max_input_tokens + glm.max_output_tokens

    assert load_serving() is luna
    assert set(SERVINGS) == {"luna-max", "glm-5.3"}


def test_unknown_serving_name_fails_closed() -> None:
    with pytest.raises(ValueError, match="unknown Smith Harbor serving"):
        load_serving("gpt-5.6-terra")


def test_default_serving_config_matches_the_pinned_luna_document() -> None:
    config = tomllib.loads(smith_config_toml())

    assert config["profiles"]["harbor"]["provider"] == "chatgpt"
    assert config["profiles"]["harbor"]["model"] == "gpt-5.6-luna"
    provider = config["providers"]["chatgpt"]
    assert provider["kind"] == "chatgpt-responses"
    assert provider["credential"] == "authfile:chatgpt"
    assert "api_key" not in provider
    model = config["models"]["chatgpt/gpt-5.6-luna"]
    assert model["context_tokens"] == 272_000
    assert model["max_input_tokens"] == MAX_INPUT_TOKENS
    assert model["max_output_tokens"] == 16_384
    assert config["models"]["chatgpt/gpt-5.6-luna"]["reasoning"]["mandatory"] is True
    assert "response" not in provider


def test_glm_serving_config_uses_env_credential_and_text_reasoning() -> None:
    rendered = smith_config_toml(serving=load_serving("glm-5.3"))

    assert "api_key" not in rendered
    config = tomllib.loads(rendered)
    assert config["profiles"]["harbor"]["provider"] == "zai"
    assert config["profiles"]["harbor"]["model"] == "glm-5.3"
    provider = config["providers"]["zai"]
    assert provider["kind"] == "openai-compatible"
    assert provider["base_url"] == "https://api.z.ai/api/coding/paas/v4"
    assert provider["credential"] == f"env:{ZAI_CREDENTIAL_ENV_VAR}"
    assert provider["response"]["reasoning_only"] == "text"
    model = config["models"]["zai/glm-5.3"]
    assert model["context_tokens"] == 386_688
    assert model["max_input_tokens"] == MAX_INPUT_TOKENS
    assert model["max_output_tokens"] == 131_072
    assert config["profiles"]["harbor"]["max_output_tokens"] == 131_072
    assert config["context"]["output_reserve"] == 131_072
    assert "reasoning" not in model


def test_variant_lines_stay_inside_the_profile_block_for_either_serving() -> None:
    for name in SERVINGS:
        config = tomllib.loads(
            smith_config_toml(
                profile_instructions="Finish early.",
                delegation=False,
                serving=load_serving(name),
            )
        )
        profile = config["profiles"]["harbor"]
        assert profile["instructions"] == "Finish early."
        assert profile["delegation"] is False


def test_job_config_records_serving_invariants_and_hosts(tmp_path: Path) -> None:
    from smith_harbor.profiles import job_config, load_profile

    profile = load_profile("smoke")
    config = job_config(
        profile,
        job_name="smith-glm-smoke",
        jobs_dir=tmp_path,
        serving=load_serving("glm-5.3"),
    )
    rendered = json.dumps(config)
    assert ZAI_CREDENTIAL_ENV_VAR in rendered  # the reference, never the value
    assert "api.z.ai" in rendered
    (agent,) = config["agents"]
    assert agent["model_name"] == "zai/glm-5.3"
    assert agent["extra_allowed_hosts"] == ["api.z.ai"]
    assert agent["kwargs"]["serving_name"] == "glm-5.3"
    invariants = agent["kwargs"]["run_invariants"]
    assert invariants["serving"] == "glm-5.3"
    assert invariants["provider"] == "zai"
    assert invariants["model"] == "glm-5.3"
    assert invariants["effort"] == "none"
    assert invariants["credential_policy"] == f"env_{ZAI_CREDENTIAL_ENV_VAR}"
    assert invariants["cost_provenance"] == "per_token_api_key_metered"
    assert invariants["network_policy"]["agent_extra_allowed_hosts"] == ["api.z.ai"]


def test_luna_job_config_keeps_oauth_invariants(tmp_path: Path) -> None:
    from smith_harbor.profiles import job_config, load_profile

    config = job_config(
        load_profile("smoke"), job_name="smith-luna-smoke", jobs_dir=tmp_path
    )
    (agent,) = config["agents"]
    invariants = agent["kwargs"]["run_invariants"]
    assert invariants["serving"] == "luna-max"
    assert invariants["effort"] == "max"
    assert invariants["credential_policy"] == "oauth_authfile_selected_entry"
    assert invariants["cost_provenance"] == "unknown_subscription_oauth"
    assert agent["extra_allowed_hosts"] == ["auth.openai.com", "chatgpt.com"]


def test_api_key_resolution_prefers_environment_and_never_logs_content(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from smith_harbor.auth import resolve_provider_api_key

    monkeypatch.setenv("ZAI_API_KEY", "env-key-value")
    assert resolve_provider_api_key("ZAI_API_KEY", "zai") == "env-key-value"


def test_api_key_resolution_falls_back_to_operator_config(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from smith_harbor.auth import resolve_provider_api_key

    config = tmp_path / "config.toml"
    config.write_text(
        '[providers.zai]\nkind = "openai-compatible"\n'
        'base_url = "https://api.z.ai/api/coding/paas/v4"\n'
        'api_key = "config-key-value"\n',
        encoding="utf-8",
    )
    monkeypatch.delenv("ZAI_API_KEY", raising=False)
    monkeypatch.setenv("SMITH_HARBOR_CONFIG_FILE", str(config))

    assert resolve_provider_api_key("ZAI_API_KEY", "zai") == "config-key-value"


@pytest.mark.parametrize("config_text", ["", "[providers.other]\napi_key = \"x\"\n"])
def test_api_key_resolution_fails_closed_without_content(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, config_text: str
) -> None:
    from smith_harbor.auth import ApiKeyPreflightError, resolve_provider_api_key

    config = tmp_path / "config.toml"
    config.write_text(config_text, encoding="utf-8")
    monkeypatch.delenv("ZAI_API_KEY", raising=False)
    monkeypatch.setenv("SMITH_HARBOR_CONFIG_FILE", str(config))

    with pytest.raises(ApiKeyPreflightError) as excinfo:
        resolve_provider_api_key("ZAI_API_KEY", "zai")
    assert "ZAI_API_KEY" in str(excinfo.value)
    assert "config-key-value" not in str(excinfo.value)


def test_preflight_glm_requires_resolvable_key(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    from smith_harbor.profiles import ProfileError, load_profile, preflight

    monkeypatch.delenv("ZAI_API_KEY", raising=False)
    monkeypatch.setenv("SMITH_HARBOR_CONFIG_FILE", str(tmp_path / "missing.toml"))

    with pytest.raises(ProfileError, match="ZAI_API_KEY"):
        preflight(load_profile("smoke"), serving_name="glm-5.3")


def test_smith_command_matches_serving_identity_without_credential_material() -> None:
    from smith_harbor.smith_agent import SmithAgent

    luna = SmithAgent._smith_command("finish the task", load_serving("luna-max"))
    assert "--provider chatgpt --model gpt-5.6-luna --effort max " in luna
    assert '\'\"effort\":\"max"\'*)' in luna

    glm = SmithAgent._smith_command("finish the task", load_serving("glm-5.3"))
    assert "--provider zai --model glm-5.3 --approval" in glm
    assert "--effort" not in glm
    assert "effort" not in glm
    assert ZAI_CREDENTIAL_ENV_VAR not in glm
    assert '"provider":"zai"' in glm
    assert '"model":"glm-5.3"' in glm


def test_agent_model_name_must_match_the_selected_serving(tmp_path: Path) -> None:
    from smith_harbor.smith_agent import SmithAgent

    with pytest.raises(ValueError, match="model_name='zai/glm-5.3'"):
        SmithAgent(
            tmp_path,
            model_name="chatgpt/gpt-5.6-luna",
            serving_name="glm-5.3",
        )


def test_paired_comparison_refuses_across_servings() -> None:
    from smith_harbor.analysis import AnalysisError, LoadedJob, TrialObservation, compare_jobs

    def job(serving: str) -> LoadedJob:
        trial = TrialObservation(
            task="task-0",
            reward=1.0,
            tokens=100.0,
            latency_seconds=10.0,
            reported_success=True,
            verifier_success=True,
            failed=False,
        )
        return LoadedJob(
            Path(serving),
            serving,
            {
                "dataset": "frozen",
                "serving": serving,
                "provider": serving,
                "model": serving,
                "effort": "none",
                "credential_policy": f"env_{serving}",
            },
            (trial,),
        )

    with pytest.raises(AnalysisError, match="invariants differ"):
        compare_jobs(job("luna-max"), job("glm-5.3"))

    report = compare_jobs(job("luna-max"), job("glm-5.3"), descriptive_only=True)
    assert report["comparison_type"] == "descriptive_only_unpaired"


def test_audit_scans_for_resolved_api_key_on_env_servings(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from smith_harbor.audit import JobAuditError, audit_job

    job = tmp_path / "job"
    job.mkdir()
    (job / "result.json").write_text("{}", encoding="utf-8")
    (job / "trajectory.json").write_text("safe", encoding="utf-8")

    monkeypatch.setenv(ZAI_CREDENTIAL_ENV_VAR, "zai-secret-value")
    report = audit_job(job, serving_name="glm-5.3")
    assert report["status"] == "ok"
    assert report["credential_value_hits"] == 0
    assert report["serving"] == "glm-5.3"

    (job / "leak.txt").write_text("contained zai-secret-value", encoding="utf-8")
    with pytest.raises(JobAuditError, match="no credential content was printed"):
        audit_job(job, serving_name="glm-5.3")
