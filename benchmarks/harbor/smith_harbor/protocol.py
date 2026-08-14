"""Strict Smith schema-v3 parsing and redaction-safe ATIF conversion."""

from __future__ import annotations

import datetime as dt
import json
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from smith_harbor.constants import (
    ATIF_SCHEMA_VERSION,
    EFFORT,
    MODEL,
    OUTPUT_SCHEMA_VERSION,
    PROVIDER,
    RUNTIME_EVENT_SCHEMA_VERSION,
    SEGMENT_KINDS,
)

USAGE_KEYS = ("input_uncached", "input_cached", "cache_write", "output", "reasoning")
TERMINAL_STATUSES = {
    "ok",
    "approval_required",
    "interaction_required",
    "failed",
    "cancelled",
    "limit_reached",
}


class ProtocolError(ValueError):
    """Smith's machine stream violated the pinned integration contract."""


@dataclass(frozen=True)
class ParsedRun:
    """A validated Smith stream and its derived aggregate measurements."""

    events: tuple[dict[str, Any], ...]
    result: dict[str, Any]

    @property
    def usage(self) -> dict[str, int]:
        usage = _object(self.result.get("usage"), "Smith result usage")
        raw = _object(usage.get("current_turn"), "current-turn usage")
        return {key: _nonnegative_int(raw.get(key), default=0) for key in USAGE_KEYS}

    @property
    def usage_known(self) -> bool:
        usage = self.result.get("usage")
        return (
            isinstance(usage, dict) and usage.get("current_turn_provenance") == "provider_reported"
        )

    @property
    def harbor_input_tokens(self) -> int | None:
        if not self.usage_known:
            return None
        usage = self.usage
        return usage["input_uncached"] + usage["input_cached"] + usage["cache_write"]

    @property
    def harbor_cache_tokens(self) -> int | None:
        return self.usage["input_cached"] if self.usage_known else None

    @property
    def harbor_output_tokens(self) -> int | None:
        if not self.usage_known:
            return None
        usage = self.usage
        return usage["output"] + usage["reasoning"]


def _nonnegative_int(value: object, *, default: int | None = None) -> int:
    if value is None and default is not None:
        return default
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ProtocolError("Smith usage counters must be nonnegative integers")
    return value


def _object(value: object, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ProtocolError(f"{label} must be a JSON object")
    return value


def load_stream(path: Path, *, max_bytes: int = 128 * 1024 * 1024) -> ParsedRun:
    try:
        size = path.stat().st_size
    except OSError as exc:
        raise ProtocolError("Smith stream is unavailable") from exc
    if size > max_bytes:
        raise ProtocolError("Smith stream exceeds the 128 MiB conversion limit")
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError) as exc:
        raise ProtocolError("Smith stream is not readable UTF-8") from exc
    return parse_stream(lines)


def parse_stream(
    lines: Iterable[str],
    *,
    expected_provider: str = PROVIDER,
    expected_model: str = MODEL,
    expected_effort: str = EFFORT,
) -> ParsedRun:
    documents: list[dict[str, Any]] = []
    for ordinal, line in enumerate(lines, start=1):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ProtocolError(f"Smith stream line {ordinal} is not JSON") from exc
        documents.append(_object(value, f"Smith stream line {ordinal}"))
    if not documents:
        raise ProtocolError("Smith stream is empty")

    terminal = documents[-1]
    if terminal.get("schema_version") != OUTPUT_SCHEMA_VERSION or terminal.get("type") != "result":
        raise ProtocolError("Smith stream must end with one schema-v3 result")
    if any(document.get("type") == "result" for document in documents[:-1]):
        raise ProtocolError("Smith stream contains a nonterminal result")
    status = terminal.get("status")
    if status not in TERMINAL_STATUSES:
        raise ProtocolError(f"Smith result has unsupported status {status!r}")
    if terminal.get("provider") != expected_provider:
        raise ProtocolError("Smith result provider does not match the pinned profile")
    if terminal.get("model") != expected_model:
        raise ProtocolError("Smith result model does not match the pinned profile")
    reasoning = _object(terminal.get("reasoning"), "Smith result reasoning")
    if reasoning.get("effort") != expected_effort or reasoning.get("state") != "on":
        raise ProtocolError("Smith result did not apply the pinned reasoning effort")

    events: list[dict[str, Any]] = []
    prior_sequence: int | None = None
    session_id = terminal.get("session_id")
    if not isinstance(session_id, str) or not session_id:
        raise ProtocolError("Smith result lacks a session identity")
    for ordinal, wrapper in enumerate(documents[:-1], start=1):
        if (
            wrapper.get("schema_version") != OUTPUT_SCHEMA_VERSION
            or wrapper.get("type") != "runtime_event"
        ):
            raise ProtocolError(f"Smith stream line {ordinal} is not a schema-v3 runtime event")
        event = _object(wrapper.get("event"), f"Smith stream event {ordinal}")
        if event.get("schema_version") != RUNTIME_EVENT_SCHEMA_VERSION:
            raise ProtocolError(
                f"Smith runtime event schema must be {RUNTIME_EVENT_SCHEMA_VERSION}"
            )
        sequence = event.get("seq")
        if isinstance(sequence, bool) or not isinstance(sequence, int) or sequence < 0:
            raise ProtocolError("Smith runtime event sequence is invalid")
        if prior_sequence is not None and sequence != prior_sequence + 1:
            raise ProtocolError("Smith runtime event stream has a sequence gap")
        prior_sequence = sequence
        if event.get("session") != session_id:
            raise ProtocolError("Smith runtime event session does not match the result")
        _object(event.get("payload"), f"Smith runtime event payload {ordinal}")
        events.append(event)

    result_usage = _object(terminal.get("usage"), "Smith result usage")
    usage = _object(result_usage.get("current_turn"), "current-turn usage")
    for key, value in usage.items():
        if key in USAGE_KEYS:
            _nonnegative_int(value)
    return ParsedRun(events=tuple(events), result=terminal)


def _iso_timestamp(milliseconds: object) -> str | None:
    if isinstance(milliseconds, bool) or not isinstance(milliseconds, int) or milliseconds < 0:
        return None
    return (
        dt.datetime.fromtimestamp(milliseconds / 1000, tz=dt.UTC).isoformat().replace("+00:00", "Z")
    )


def _usage_metrics(delta: Mapping[str, object]) -> dict[str, Any] | None:
    counters = {key: _nonnegative_int(delta.get(key), default=0) for key in USAGE_KEYS}
    if not any(counters.values()):
        return None
    return {
        "prompt_tokens": counters["input_uncached"]
        + counters["input_cached"]
        + counters["cache_write"],
        "completion_tokens": counters["output"] + counters["reasoning"],
        "cached_tokens": counters["input_cached"],
        "extra": {"smith_usage": counters, "provenance": "provider_reported"},
    }


def _attempts(parsed: ParsedRun) -> list[dict[str, Any]]:
    attempts: list[dict[str, Any]] = []
    by_id: dict[str, dict[str, Any]] = {}
    calls: dict[str, tuple[dict[str, Any], dict[str, Any]]] = {}
    current: dict[str, Any] | None = None
    for event in parsed.events:
        payload = event["payload"]
        name = payload.get("event")
        if name == "provider_attempt_started":
            attempt_id = payload.get("attempt")
            request_id = payload.get("request")
            if not isinstance(attempt_id, str) or not isinstance(request_id, str):
                raise ProtocolError("provider_attempt_started lacks request/attempt identity")
            current = {
                "attempt": attempt_id,
                "request": request_id,
                "index": payload.get("index"),
                "timestamp": _iso_timestamp(event.get("timestamp")),
                "text": [],
                "reasoning": [],
                "redacted_reasoning_fragments": 0,
                "tool_calls": [],
                "observations": [],
                "usage": {key: 0 for key in USAGE_KEYS},
                "disposition": "unfinished",
                "finish": None,
            }
            attempts.append(current)
            by_id[attempt_id] = current
        elif name in {"text_delta", "reasoning_delta"}:
            attempt_id = payload.get("attempt")
            attempt = by_id.get(attempt_id) if isinstance(attempt_id, str) else None
            if attempt is None:
                raise ProtocolError(f"{name} references an unknown provider attempt")
            text = payload.get("text")
            if not isinstance(text, str):
                raise ProtocolError(f"{name} lacks text")
            if name == "reasoning_delta" and payload.get("redacted") is True:
                attempt["redacted_reasoning_fragments"] += 1
            else:
                attempt["reasoning" if name == "reasoning_delta" else "text"].append(text)
        elif name == "usage":
            record = _object(payload.get("record"), "Smith usage event record")
            provenance = _object(record.get("provenance"), "Smith usage provenance")
            attempt_id = provenance.get("attempt")
            attempt = by_id.get(attempt_id) if isinstance(attempt_id, str) else None
            if attempt is not None:
                delta = _object(record.get("delta"), "Smith usage delta")
                for key in USAGE_KEYS:
                    attempt["usage"][key] += _nonnegative_int(delta.get(key), default=0)
        elif name in {"provider_attempt_output_committed", "provider_attempt_output_discarded"}:
            attempt_id = payload.get("attempt")
            attempt = by_id.get(attempt_id) if isinstance(attempt_id, str) else None
            if attempt is None:
                raise ProtocolError(f"{name} references an unknown provider attempt")
            attempt["disposition"] = "committed" if name.endswith("committed") else "discarded"
        elif name == "provider_attempt_finished":
            attempt_id = payload.get("attempt")
            attempt = by_id.get(attempt_id) if isinstance(attempt_id, str) else None
            if attempt is None:
                raise ProtocolError("provider_attempt_finished references an unknown attempt")
            attempt["finish"] = payload.get("finish")
        elif name == "tool_call_requested":
            if current is None:
                raise ProtocolError("tool call appeared before a provider attempt")
            call_id = payload.get("call")
            tool_name = payload.get("name")
            if not isinstance(call_id, str) or not isinstance(tool_name, str):
                raise ProtocolError("tool call lacks stable identity or name")
            raw_arguments = payload.get("arguments")
            arguments = raw_arguments if isinstance(raw_arguments, dict) else {}
            call = {
                "tool_call_id": call_id,
                "function_name": tool_name,
                "arguments": arguments,
                "extra": {
                    "argument_keys": payload.get("argument_keys", []),
                    "argument_fingerprint": payload.get("argument_fingerprint"),
                    "arguments_withheld": not isinstance(raw_arguments, dict),
                },
            }
            observation = {
                "source_call_id": call_id,
                "content": None,
                "extra": {"observation_withheld": True, "completed": False},
            }
            current["tool_calls"].append(call)
            current["observations"].append(observation)
            calls[call_id] = (call, observation)
        elif name == "tool_call_completed":
            pair = calls.get(payload.get("call"))
            if pair is None:
                raise ProtocolError("tool completion references an unknown tool call")
            _, observation = pair
            observation["extra"] = {
                "observation_withheld": True,
                "completed": True,
                "is_error": payload.get("is_error") is True,
                "tool_name": payload.get("name"),
            }
    return attempts


def to_atif(
    parsed: ParsedRun,
    instruction: str,
    *,
    smith_version: str,
    smith_revision: str,
    artifact_sha256: str,
    run_invariants: Mapping[str, object] | None = None,
) -> dict[str, Any]:
    """Convert one validated stream to an ATIF-v1.7 JSON-compatible object."""
    steps: list[dict[str, Any]] = [{"step_id": 1, "source": "user", "message": instruction}]
    attempts = _attempts(parsed)
    for attempt in attempts:
        extra = {
            "request_id": attempt["request"],
            "attempt_id": attempt["attempt"],
            "attempt_index": attempt["index"],
            "disposition": attempt["disposition"],
            "finish": attempt["finish"],
            "redacted_reasoning_fragments": attempt["redacted_reasoning_fragments"],
        }
        step: dict[str, Any] = {
            "step_id": len(steps) + 1,
            "timestamp": attempt["timestamp"],
            "source": "agent",
            "model_name": parsed.result["model"],
            "reasoning_effort": parsed.result["reasoning"]["effort"],
            "message": "".join(attempt["text"]),
            "reasoning_content": "".join(attempt["reasoning"]) or None,
            "tool_calls": attempt["tool_calls"] or None,
            "observation": (
                {"results": attempt["observations"]} if attempt["observations"] else None
            ),
            "metrics": _usage_metrics(attempt["usage"]),
            "llm_call_count": 1,
            "extra": extra,
        }
        steps.append({key: value for key, value in step.items() if value is not None})
    if len(steps) == 1:
        steps.append(
            {
                "step_id": 2,
                "source": "agent",
                "model_name": parsed.result["model"],
                "reasoning_effort": parsed.result["reasoning"]["effort"],
                "message": parsed.result.get("output", ""),
                "llm_call_count": 1,
                "extra": {"synthetic_terminal_projection": True},
            }
        )

    lifecycle = _object(parsed.result.get("lifecycle"), "Smith result lifecycle")
    event_names = [event["payload"].get("event") for event in parsed.events]
    usage = parsed.usage
    tool_calls = sum(1 for name in event_names if name == "tool_call_requested")
    tool_errors = sum(
        1
        for event in parsed.events
        if event["payload"].get("event") == "tool_call_completed"
        and event["payload"].get("is_error") is True
    )
    raw_children = lifecycle.get("children")
    children = raw_children if isinstance(raw_children, list) else []
    activation = lifecycle.get("activation")
    raw_capabilities = activation.get("capabilities") if isinstance(activation, dict) else None
    capabilities = raw_capabilities if isinstance(raw_capabilities, list) else []
    metadata = {
        "smith_usage": usage,
        "usage_provenance": _object(parsed.result.get("usage"), "Smith result usage").get(
            "current_turn_provenance"
        ),
        "provider_requests": len({attempt["request"] for attempt in attempts}),
        "attempts_committed": lifecycle.get("attempts_committed", 0),
        "attempts_discarded": lifecycle.get("attempts_discarded", 0),
        "tool_calls": tool_calls,
        "tool_errors": tool_errors,
        "activated_capabilities": capabilities,
        "child_calls": len(children),
        "compactions": event_names.count("context_compacted"),
        "reported_status": parsed.result["status"],
        "reported_success": parsed.result["status"] == "ok",
        "provider": parsed.result["provider"],
        "model": parsed.result["model"],
        "effort": parsed.result["reasoning"]["effort"],
        "smith_revision": smith_revision,
        "artifact_sha256": artifact_sha256,
        "run_invariants": dict(run_invariants or {}),
    }
    final_metrics: dict[str, Any] = {
        "total_steps": len(steps),
        "extra": metadata,
    }
    if parsed.harbor_input_tokens is not None:
        final_metrics["total_prompt_tokens"] = parsed.harbor_input_tokens
        final_metrics["total_cached_tokens"] = parsed.harbor_cache_tokens
        final_metrics["total_completion_tokens"] = parsed.harbor_output_tokens

    return {
        "schema_version": ATIF_SCHEMA_VERSION,
        "session_id": parsed.result["session_id"],
        "agent": {
            "name": "smith",
            "version": smith_version.removeprefix("smith "),
            "model_name": parsed.result["model"],
            "extra": {
                "provider": parsed.result["provider"],
                "reasoning": parsed.result["reasoning"],
                "smith_revision": smith_revision,
                "artifact_sha256": artifact_sha256,
            },
        },
        "steps": steps,
        "notes": (
            "Smith runtime events intentionally withhold raw tool arguments and observations; "
            "ATIF entries preserve that boundary."
        ),
        "final_metrics": final_metrics,
        "extra": metadata,
    }


def failure_atif(
    instruction: str,
    *,
    smith_version: str,
    smith_revision: str,
    artifact_sha256: str,
    failure_kind: str,
) -> dict[str, Any]:
    """Create a minimal valid ATIF record without inventing missing agent output."""
    failure = {
        "conversion_status": "failed",
        "failure_kind": failure_kind,
        "reported_success": False,
    }
    return {
        "schema_version": ATIF_SCHEMA_VERSION,
        "agent": {
            "name": "smith",
            "version": smith_version.removeprefix("smith "),
            "model_name": MODEL,
            "extra": {
                "provider": PROVIDER,
                "reasoning_effort": EFFORT,
                "smith_revision": smith_revision,
                "artifact_sha256": artifact_sha256,
            },
        },
        "steps": [{"step_id": 1, "source": "user", "message": instruction}],
        "notes": (
            "Smith output could not be converted. No agent message, tool arguments, "
            "observations, or token counts were inferred."
        ),
        "final_metrics": {"total_steps": 1, "extra": failure},
        "extra": {
            **failure,
            "provider": PROVIDER,
            "model": MODEL,
            "effort": EFFORT,
            "smith_revision": smith_revision,
            "artifact_sha256": artifact_sha256,
        },
    }


def base_footprint_report(parsed: ParsedRun) -> dict[str, Any]:
    """Keep planned component counts distinct from observed first-attempt usage."""
    context_event = next(
        (event for event in parsed.events if event["payload"].get("event") == "context_planned"),
        None,
    )
    if context_event is None:
        raise ProtocolError("base probe did not emit context_planned")
    payload = context_event["payload"]
    raw_totals = _object(payload.get("totals"), "context-planned totals")
    planned = {kind: _nonnegative_int(raw_totals.get(kind), default=0) for kind in SEGMENT_KINDS}

    first_usage: dict[str, int] | None = None
    first_attempt: str | None = None
    for event in parsed.events:
        event_payload = event["payload"]
        if event_payload.get("event") != "usage":
            continue
        record = _object(event_payload.get("record"), "base-probe usage record")
        if record.get("source") != "provider_attempt":
            continue
        provenance = _object(record.get("provenance"), "base-probe usage provenance")
        attempt_id = provenance.get("attempt")
        first_attempt = attempt_id if isinstance(attempt_id, str) else None
        delta = _object(record.get("delta"), "base-probe usage delta")
        first_usage = {key: _nonnegative_int(delta.get(key), default=0) for key in USAGE_KEYS}
        break
    if first_usage is None:
        raise ProtocolError("base probe did not emit provider-attributed usage")
    return {
        "schema_version": 1,
        "measurement_warning": (
            "planned segment counts and provider-observed usage are separate measurements; "
            "no subtraction-based base-token claim is made"
        ),
        "planned_context": {
            "provenance": payload.get("confidence", "estimated"),
            "segment_tokens": planned,
            "input_tokens": _nonnegative_int(payload.get("input_tokens")),
            "input_budget_tokens": _nonnegative_int(payload.get("input_budget_tokens")),
            "reserved_tokens": _nonnegative_int(payload.get("reserved_tokens")),
        },
        "provider_observed_first_attempt": {
            "provenance": "provider_reported",
            "attempt_id": first_attempt,
            "usage": first_usage,
        },
        "provider": parsed.result["provider"],
        "model": parsed.result["model"],
        "effort": parsed.result["reasoning"]["effort"],
    }
