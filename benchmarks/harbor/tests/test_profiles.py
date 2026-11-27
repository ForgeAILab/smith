from __future__ import annotations

import json
import signal
from pathlib import Path

import pytest

from smith_harbor.profiles import (
    ProfileError,
    _run_owned_process,
    job_config,
    load_profile,
    run_profile,
)


def test_frozen_profiles_have_expected_counts_and_serial_defaults(tmp_path: Path) -> None:
    expected = {"smoke": (3, 1), "context-probe": (6, 1), "dev": (26, 1), "full": (82, 3)}
    for name, (tasks, rollouts) in expected.items():
        profile = load_profile(name)
        assert len(profile.tasks) == tasks
        assert profile.rollouts == rollouts
        assert profile.concurrency == 1

        config = job_config(profile, job_name=f"smith-{name}", jobs_dir=tmp_path)
        rendered = json.dumps(config)
        assert "auth.json" not in rendered
        assert "oauth-secret" not in rendered
        assert config["artifacts"] == []
        assert config["n_concurrent_trials"] == 1


def test_context_probe_supports_glm_persistence_input_cap_and_concurrency(tmp_path: Path) -> None:
    from smith_harbor.constants import load_serving

    profile = load_profile("context-probe")
    glm = load_serving("glm-5.3")
    config = job_config(
        profile,
        job_name="smith-glm-context-probe",
        jobs_dir=tmp_path,
        concurrency=3,
        serving=glm,
        persistence=True,
        max_input_tokens=64_000,
    )
    assert config["n_concurrent_trials"] == 3
    (agent,) = config["agents"]
    kwargs = agent["kwargs"]
    assert kwargs["persistence"] is True
    assert kwargs["max_input_tokens"] == 64_000
    invariants = kwargs["run_invariants"]
    assert invariants["persistence"] is True
    assert invariants["serving_limits"]["max_input_tokens"] == 64_000
    assert invariants["serving_limits"]["context_tokens"] == 64_000 + glm.max_output_tokens
    assert (
        run_profile(
            "context-probe",
            job_name="smith-glm-context-probe",
            jobs_dir=tmp_path,
            concurrency=3,
            print_config=True,
            serving_name="glm-5.3",
            persistence=True,
            max_input_tokens=64_000,
        )
        == 0
    )


def test_max_input_tokens_override_must_exceed_capability_budget(tmp_path: Path) -> None:
    from smith_harbor.constants import load_serving

    profile = load_profile("context-probe")
    with pytest.raises(ProfileError, match="max_input_tokens"):
        job_config(
            profile,
            job_name="smith-glm-tiny",
            jobs_dir=tmp_path,
            serving=load_serving("glm-5.3"),
            max_input_tokens=12_000,
        )


def test_unsafe_oauth_concurrency_requires_a_visible_job_label(tmp_path: Path) -> None:
    with pytest.raises(ProfileError, match="unsafe-oauth"):
        run_profile(
            "smoke",
            job_name="unlabelled",
            jobs_dir=tmp_path,
            unsafe_concurrency=2,
            print_config=True,
        )

    assert (
        run_profile(
            "smoke",
            job_name="experiment-unsafe-oauth",
            jobs_dir=tmp_path,
            unsafe_concurrency=2,
            print_config=True,
        )
        == 0
    )


def test_owned_harbor_process_group_is_interrupted_and_reaped(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    calls: list[tuple[str, object]] = []
    captured_popen_kwargs: dict[str, object] = {}

    class InterruptingProcess:
        pid = 4242

        def wait(self, timeout: float | None = None) -> int:
            calls.append(("wait", timeout))
            if timeout is None:
                raise KeyboardInterrupt
            return 130

    def fake_popen(*args: object, **kwargs: object) -> InterruptingProcess:
        captured_popen_kwargs.update(kwargs)
        calls.append(("popen", (args, kwargs)))
        return InterruptingProcess()

    monkeypatch.setattr("smith_harbor.profiles.subprocess.Popen", fake_popen)
    monkeypatch.setattr(
        "smith_harbor.profiles.os.killpg",
        lambda pid, sig: calls.append(("signal", (pid, sig))),
    )

    with pytest.raises(KeyboardInterrupt):
        _run_owned_process(["harbor", "run"], cwd=tmp_path, env={})

    assert captured_popen_kwargs["start_new_session"] is True
    assert ("signal", (4242, signal.SIGINT)) in calls
    assert not any(call == ("signal", (4242, signal.SIGTERM)) for call in calls)
