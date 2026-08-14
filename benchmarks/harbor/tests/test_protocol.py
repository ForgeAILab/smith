from __future__ import annotations

import json
from typing import Any

import pytest
from harbor.models.trajectories import Trajectory

from smith_harbor.constants import SEGMENT_KINDS
from smith_harbor.protocol import (
    ProtocolError,
    base_footprint_report,
    failure_atif,
    parse_stream,
    to_atif,
)


def _event(sequence: int, payload: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": 3,
        "type": "runtime_event",
        "event": {
            "schema_version": 13,
            "seq": sequence,
            "timestamp": 1_700_000_000_000 + sequence,
            "session": "session-1",
            "payload": payload,
        },
    }


def _stream(*, status: str = "ok") -> list[str]:
    usage = {
        "input_uncached": 100,
        "input_cached": 20,
        "cache_write": 5,
        "output": 10,
        "reasoning": 3,
    }
    events = [
        _event(
            1,
            {
                "event": "context_planned",
                "confidence": "estimated",
                "totals": {kind: index for index, kind in enumerate(SEGMENT_KINDS)},
                "input_tokens": 700,
                "input_budget_tokens": 900,
                "reserved_tokens": 200,
            },
        ),
        _event(
            2,
            {
                "event": "provider_attempt_started",
                "attempt": "attempt-1",
                "request": "request-1",
                "index": 0,
            },
        ),
        _event(
            3,
            {
                "event": "reasoning_delta",
                "attempt": "attempt-1",
                "text": "must-not-export",
                "redacted": True,
            },
        ),
        _event(
            4,
            {"event": "text_delta", "attempt": "attempt-1", "text": "done"},
        ),
        _event(
            5,
            {
                "event": "tool_call_requested",
                "call": "call-1",
                "name": "read_file",
                "argument_keys": ["path"],
                "argument_fingerprint": "sha256:redacted",
            },
        ),
        _event(
            6,
            {
                "event": "tool_call_completed",
                "call": "call-1",
                "name": "read_file",
                "is_error": False,
            },
        ),
        _event(
            7,
            {
                "event": "usage",
                "record": {
                    "source": "provider_attempt",
                    "provenance": {"attempt": "attempt-1"},
                    "delta": usage,
                },
            },
        ),
        _event(
            8,
            {"event": "provider_attempt_output_committed", "attempt": "attempt-1"},
        ),
        _event(
            9,
            {
                "event": "provider_attempt_finished",
                "attempt": "attempt-1",
                "finish": "stop",
            },
        ),
    ]
    result = {
        "schema_version": 3,
        "type": "result",
        "status": status,
        "provider": "chatgpt",
        "model": "gpt-5.6-luna",
        "reasoning": {"state": "on", "effort": "max"},
        "session_id": "session-1",
        "output": "done",
        "usage": {
            "current_turn": usage,
            "current_turn_provenance": "provider_reported",
        },
        "lifecycle": {
            "attempts_committed": 1,
            "attempts_discarded": 0,
            "children": [],
            "activation": {"capabilities": ["filesystem"]},
        },
    }
    return [json.dumps(document) for document in [*events, result]]


def test_stream_conversion_is_strict_redaction_safe_and_atif_valid() -> None:
    parsed = parse_stream(_stream())
    trajectory = to_atif(
        parsed,
        "do the task",
        smith_version="smith 0.1.0",
        smith_revision="a" * 40,
        artifact_sha256="b" * 64,
    )

    validated = Trajectory.model_validate(trajectory)
    assert validated.schema_version == "ATIF-v1.7"
    agent_step = trajectory["steps"][1]
    assert agent_step["tool_calls"][0]["arguments"] == {}
    assert agent_step["tool_calls"][0]["extra"]["arguments_withheld"] is True
    assert agent_step["observation"]["results"][0]["content"] is None
    assert "must-not-export" not in json.dumps(trajectory)
    assert agent_step["metrics"]["prompt_tokens"] == 125
    assert agent_step["metrics"]["cached_tokens"] == 20
    assert agent_step["metrics"]["completion_tokens"] == 13
    assert trajectory["final_metrics"]["total_prompt_tokens"] == 125
    assert trajectory["final_metrics"]["total_completion_tokens"] == 13


def test_failed_partial_stream_is_preserved_as_a_valid_trajectory() -> None:
    parsed = parse_stream(_stream(status="failed"))

    trajectory = to_atif(
        parsed,
        "do the task",
        smith_version="smith 0.1.0",
        smith_revision="a" * 40,
        artifact_sha256="b" * 64,
    )

    Trajectory.model_validate(trajectory)
    assert trajectory["extra"]["reported_success"] is False
    assert trajectory["steps"][1]["message"] == "done"


def test_unparseable_or_timed_out_run_has_non_invented_failure_atif() -> None:
    trajectory = failure_atif(
        "do the task",
        smith_version="smith 0.1.0",
        smith_revision="a" * 40,
        artifact_sha256="b" * 64,
        failure_kind="ProtocolError",
    )

    Trajectory.model_validate(trajectory)
    assert len(trajectory["steps"]) == 1
    assert trajectory["steps"][0]["source"] == "user"
    assert trajectory["extra"]["reported_success"] is False
    assert "agent message" in trajectory["notes"]


def test_parser_rejects_sequence_gaps_and_schema_drift() -> None:
    sequence_gap = _stream()
    wrapper = json.loads(sequence_gap[1])
    wrapper["event"]["seq"] = 99
    sequence_gap[1] = json.dumps(wrapper)
    with pytest.raises(ProtocolError, match="sequence gap"):
        parse_stream(sequence_gap)

    schema_drift = _stream()
    wrapper = json.loads(schema_drift[0])
    wrapper["event"]["schema_version"] = 12
    schema_drift[0] = json.dumps(wrapper)
    with pytest.raises(ProtocolError, match="schema must be 13"):
        parse_stream(schema_drift)


def test_base_probe_keeps_planned_and_observed_measurements_distinct() -> None:
    report = base_footprint_report(parse_stream(_stream()))

    assert report["planned_context"]["input_tokens"] == 700
    assert report["provider_observed_first_attempt"]["usage"]["input_uncached"] == 100
    assert "subtraction" in report["measurement_warning"]


def test_unknown_usage_provenance_is_not_reported_as_tokens() -> None:
    lines = _stream()
    result = json.loads(lines[-1])
    result["usage"]["current_turn_provenance"] = "estimated"
    lines[-1] = json.dumps(result)

    parsed = parse_stream(lines)

    assert parsed.harbor_input_tokens is None
    assert parsed.harbor_output_tokens is None
