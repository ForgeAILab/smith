from __future__ import annotations

import os
import subprocess
from pathlib import Path

from smith_harbor.smith_agent import SmithAgent


def test_nonzero_exit_description_classifies_native_signal_without_raw_output() -> None:
    description = SmithAgent._exit_description(139)

    assert description == ("Smith returned bounded non-success exit code 139 (signal 11 SIGSEGV)")
    assert "prompt" not in description
    assert "command" not in description


def test_non_signal_exit_description_stays_numeric() -> None:
    assert SmithAgent._exit_description(1) == ("Smith returned bounded non-success exit code 1")


def test_task_command_uses_bash_login_environment_when_available() -> None:
    command = SmithAgent._task_login_command("printf '%s' \"$0\"")

    result = subprocess.run(
        ["sh", "-c", command],
        check=True,
        capture_output=True,
        text=True,
    )

    assert result.stdout.endswith("bash")


def test_task_command_quotes_untrusted_instruction_content() -> None:
    command = SmithAgent._task_login_command("printf '%s' 'safe; still one command'")

    result = subprocess.run(
        ["sh", "-c", command],
        check=True,
        capture_output=True,
        text=True,
    )

    assert result.stdout == "safe; still one command"


def test_task_command_falls_back_to_posix_shell_without_bash(tmp_path: Path) -> None:
    shell = tmp_path / "sh"
    shell.write_text('#!/bin/sh\nexec /bin/sh "$@"\n', encoding="utf-8")
    shell.chmod(0o755)
    command = SmithAgent._task_login_command("printf '%s' fallback")

    result = subprocess.run(
        ["/bin/sh", "-c", command],
        check=True,
        capture_output=True,
        text=True,
        env={**os.environ, "PATH": str(tmp_path)},
    )

    assert result.stdout == "fallback"


def test_smith_agent_propagates_persistence_and_max_input_tokens(tmp_path: Path) -> None:
    default_agent = SmithAgent(
        tmp_path / "default",
        model_name="zai/glm-5.3",
        serving_name="glm-5.3",
    )
    assert default_agent.persistence is False
    assert default_agent.max_input_tokens is None
    assert default_agent._run_environment()["SMITH_PERSISTENCE_ENABLED"] == "false"

    persisted_agent = SmithAgent(
        tmp_path / "persisted",
        model_name="zai/glm-5.3",
        serving_name="glm-5.3",
        persistence=True,
        max_input_tokens=64_000,
    )
    assert persisted_agent.persistence is True
    assert persisted_agent.max_input_tokens == 64_000
    assert persisted_agent._run_environment()["SMITH_PERSISTENCE_ENABLED"] == "true"
