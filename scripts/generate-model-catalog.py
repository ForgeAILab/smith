#!/usr/bin/env python3
"""Generate Smith's bounded Models.dev seed from a reviewed API snapshot."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys
import urllib.request
from typing import Any

SOURCE_URL = "https://models.dev/api.json"
SCHEMA_REVISION = 1
SUPPORTED_PROVIDERS = ("openrouter", "zai-coding-plan")
MAX_U32 = 2**32 - 1
MAX_MODELS_PER_PROVIDER = 10_000
KNOWN_MODALITIES = {
    "text": "text",
    "image": "image",
    "audio": "audio",
    "video": "video",
    "pdf": "document",
    "document": "document",
}
MODALITY_ORDER = ("text", "image", "audio", "video", "document")


def sha256(data: bytes) -> str:
    return f"sha256:{hashlib.sha256(data).hexdigest()}"


def canonical(value: Any) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode()


def bounded_text(value: Any, field: str, maximum: int) -> str:
    if (
        not isinstance(value, str)
        or not value
        or len(value.encode()) > maximum
        or any(ord(character) < 0x20 for character in value)
    ):
        raise ValueError(f"{field} must be non-empty bounded text")
    return value


def optional_bool(entry: dict[str, Any], field: str) -> tuple[bool, str | None]:
    value = entry.get(field, False)
    if isinstance(value, bool):
        return value, None
    return False, f"catalog field `{field}` is not boolean"


def modalities(
    entry: dict[str, Any],
) -> tuple[list[str], list[str], str | None]:
    raw = entry.get("modalities")
    if not isinstance(raw, dict):
        return [], [], "catalog model has no valid modality declaration"
    result: list[list[str]] = []
    for direction in ("input", "output"):
        values = raw.get(direction)
        if not isinstance(values, list) or len(values) > 16:
            return [], [], f"catalog `{direction}` modalities are invalid"
        normalized: set[str] = set()
        for value in values:
            if not isinstance(value, str) or value not in KNOWN_MODALITIES:
                return [], [], f"catalog `{direction}` modality is unsupported"
            normalized.add(KNOWN_MODALITIES[value])
        result.append([value for value in MODALITY_ORDER if value in normalized])
    return result[0], result[1], None


def positive_u32(value: Any) -> int | None:
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return value if 0 < value <= MAX_U32 else None


def limits(entry: dict[str, Any]) -> tuple[dict[str, int] | None, str | None]:
    raw = entry.get("limit")
    if not isinstance(raw, dict):
        return None, "catalog model has no valid limit declaration"
    context = positive_u32(raw.get("context"))
    output = positive_u32(raw.get("output"))
    separate_input = raw.get("input")
    input_limit = (
        context if separate_input is None else positive_u32(separate_input)
    )
    if context is None or output is None or input_limit is None:
        return None, "catalog model has a zero, missing, or out-of-range limit"
    if output > context:
        return None, "catalog output limit exceeds its context window"
    if input_limit > context:
        return None, "catalog input limit exceeds its context window"
    return {
        "context_tokens": context,
        "max_input_tokens": input_limit,
        "max_output_tokens": output,
    }, None


def normalize_model(key: str, raw: Any) -> dict[str, Any] | None:
    bounded_text(key, "model key", 512)
    if not isinstance(raw, dict):
        raise ValueError(f"model `{key}` is not an object")
    if bounded_text(raw.get("id"), f"model `{key}` id", 512) != key:
        raise ValueError(f"model `{key}` id does not match its map key")
    name = bounded_text(raw.get("name"), f"model `{key}` name", 256)

    status = raw.get("status")
    if status == "deprecated":
        return None
    disabled_reason: str | None = None
    if status is not None:
        disabled_reason = "catalog model has an unsupported status"

    normalized_limits, limit_error = limits(raw)
    input_modalities, output_modalities, modality_error = modalities(raw)
    tool_call, tool_error = optional_bool(raw, "tool_call")
    reasoning, reasoning_error = optional_bool(raw, "reasoning")
    structured_output, structured_error = optional_bool(raw, "structured_output")
    disabled_reason = disabled_reason or limit_error or modality_error
    disabled_reason = (
        disabled_reason or tool_error or reasoning_error or structured_error
    )

    result: dict[str, Any] = {
        "id": key,
        "name": name,
        "input_modalities": input_modalities,
        "output_modalities": output_modalities,
        "tool_call": tool_call,
        "reasoning": reasoning,
        "structured_output": structured_output,
    }
    if normalized_limits is not None:
        result["limits"] = normalized_limits
    if disabled_reason is not None:
        result["disabled_reason"] = disabled_reason
    return result


def normalize_provider(key: str, raw: Any) -> dict[str, Any]:
    if not isinstance(raw, dict):
        raise ValueError(f"provider `{key}` is not an object")
    if bounded_text(raw.get("id"), f"provider `{key}` id", 128) != key:
        raise ValueError(f"provider `{key}` id does not match its map key")
    name = bounded_text(raw.get("name"), f"provider `{key}` name", 256)
    models_value = raw.get("models")
    if not isinstance(models_value, dict) or len(models_value) > MAX_MODELS_PER_PROVIDER:
        raise ValueError(f"provider `{key}` has an invalid model map")

    models: dict[str, Any] = {}
    for model_id in sorted(models_value):
        model = normalize_model(model_id, models_value[model_id])
        if model is not None:
            models[model_id] = model
    return {"id": key, "name": name, "models": models}


def generate(
    source_bytes: bytes, retrieved_at_ms: int, source_revision: str | None
) -> dict[str, Any]:
    source = json.loads(source_bytes)
    if not isinstance(source, dict):
        raise ValueError("Models.dev response is not a provider-keyed object")
    providers = {
        provider: normalize_provider(provider, source.get(provider))
        for provider in SUPPORTED_PROVIDERS
    }
    source_digest = sha256(source_bytes)
    return {
        "schema_revision": SCHEMA_REVISION,
        "source_url": SOURCE_URL,
        "source_digest": source_digest,
        "content_digest": sha256(canonical(providers)),
        "source_revision": source_revision or source_digest,
        "retrieved_at_ms": retrieved_at_ms,
        "providers": providers,
    }


def fetch() -> bytes:
    request = urllib.request.Request(
        SOURCE_URL, headers={"User-Agent": "smith-model-catalog-generator"}
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        if response.geturl() != SOURCE_URL:
            raise ValueError("Models.dev redirected away from the canonical source")
        return response.read(8 * 1024 * 1024 + 1)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--retrieved-at-ms", type=int, required=True)
    parser.add_argument("--source-revision")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    source_bytes = args.input.read_bytes() if args.input else fetch()
    if len(source_bytes) > 8 * 1024 * 1024:
        raise ValueError("Models.dev response exceeds the 8 MiB generator limit")
    document = generate(
        source_bytes, args.retrieved_at_ms, args.source_revision
    )
    rendered = (
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    ).encode()

    if args.check:
        if not args.output.exists() or args.output.read_bytes() != rendered:
            print(f"{args.output} is not reproducible from the supplied source", file=sys.stderr)
            return 1
        return 0

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
