#!/usr/bin/env python3
"""Capture image wire stability, measure live cache reads, and audit a Smith journal."""

import argparse
import asyncio
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import struct
import tempfile
import threading
import time
import zlib


def digest(data):
    return hashlib.sha256(data).hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def png(width, height, color):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = (b"\0" + bytes(color) * width) * height
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def image_urls(parts):
    for part in parts:
        if part.get("type") == "image":
            yield part["url"]
        if part.get("type") == "tool_result":
            yield from image_urls(part["content"])


def wire_summary(body, previous):
    messages = body["messages"]
    hashes = [digest(json.dumps(m, separators=(",", ":")).encode()) for m in messages]
    common = 0
    for a, b in zip(previous, hashes):
        if a != b:
            break
        common += 1
    images, placeholders = [], []
    for index, message in enumerate(messages):
        parts = message.get("content", [])
        if isinstance(parts, str):
            if "not included in this request]" in parts:
                placeholders.append({"message": index, "text": parts})
            continue
        label = "inline"
        for part in parts:
            if part.get("type") == "text":
                if part["text"].startswith("Images from tool call "):
                    label = part["text"]
                if "not included in this request]" in part["text"]:
                    placeholders.append({"message": index, "text": part["text"]})
            if part.get("type") == "image_url":
                url = part["image_url"]["url"]
                data = base64.b64decode(url.split(",", 1)[1])
                images.append({"message": index, "label": label, "url_sha256": digest(url.encode()),
                               "encoded_sha256": digest(data), "bytes": len(data),
                               "dimensions": list(struct.unpack(">II", data[16:24]))})
    return {"message_hashes": hashes, "common_prefix_messages": common,
            "previous_message_count": len(previous), "images": images,
            "placeholders": placeholders, "prompt_cache_key": body.get("prompt_cache_key"),
            "non_message_fields_sha256": digest(json.dumps(
                {k: v for k, v in body.items() if k != "messages"}, separators=(",", ":")).encode())}


def wire_case(binary, scratch, name, window, attach, reads):
    project = scratch / name
    (project / ".smith").mkdir(parents=True)
    for index in range(reads - 1):
        width, height = (3500, 20) if index == 0 else (64, 64)
        (project / f"image-{index:02}.png").write_bytes(
            png(width, height, ((index * 47) % 256, (index * 71) % 256, (index * 131) % 256)))
    records = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            prior = records[-1]["message_hashes"] if records else []
            records.append(wire_summary(body, prior))
            step = len(records) - 1
            if step < reads:
                target = step if step < reads - 1 else 0
                delta = {"tool_calls": [{"index": 0, "id": f"probe-{step:02}", "type": "function",
                          "function": {"name": "read", "arguments": json.dumps({"path": f"image-{target:02}.png"})}}]}
                finish = "tool_calls"
            else:
                delta, finish = {"content": "DONE"}, "stop"
            frames = [{"choices": [{"index": 0, "delta": delta}]},
                      {"choices": [{"index": 0, "delta": {}, "finish_reason": finish}],
                       "usage": {"prompt_tokens": 100, "completion_tokens": 5, "total_tokens": 105}}]
            payload = ("".join("data: " + json.dumps(frame) + "\n\n" for frame in frames)
                       + "data: [DONE]\n\n").encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    (project / ".smith/config.toml").write_text(f'''profile_order = ["image-wire-probe"]
[providers.image-wire-probe]
kind = "openai-compatible"
base_url = "http://127.0.0.1:{server.server_port}/v1"
[models."image-wire-probe/vision-probe"]
context_tokens = 1000000
max_input_tokens = 990000
max_output_tokens = 1024
input_modalities = ["text", "image"]
[profiles.image-wire-probe]
provider = "image-wire-probe"
model = "vision-probe"
posture = "plan"
use = ["main"]
vision = false
advisor = false
delegation = false
''')
    env = {**os.environ, "SMITH_CONTEXT_INLINE_IMAGES": str(window),
           "SMITH_PERSISTENCE_SESSIONS_DIR": str(scratch / "wire-sessions"),
           "SMITH_CONTEXT_CACHE_MAINTENANCE": "off", "SMITH_CONTEXT_CACHE_IDLE_COMPACTION": "false"}
    command = [str(binary), "--project", str(project), "--profile", "image-wire-probe",
               "--approval", "deny", "--output-format", "json", "-p",
               "Read the local image files requested by this image pipeline diagnostic. Do not modify files."]
    if attach:
        command += ["--image", str(project / "image-00.png")]
    import subprocess
    try:
        process = subprocess.run(command, capture_output=True, text=True, env=env, cwd=project, timeout=180)
    finally:
        server.shutdown()
        server.server_close()
    result = json.loads(process.stdout) if process.stdout else {}
    references = [image for record in records for image in record["images"]
                  if image["label"] in ["Images from tool call probe-00:", f"Images from tool call probe-{reads - 1:02}:"]]
    summary = {"name": name, "window": window, "attached_reference": attach,
            "exit_code": process.returncode, "status": result.get("status"), "stderr": process.stderr,
            "requests": records, "reference_encoding_variants": len({x["url_sha256"] for x in references}),
            "cache_key_variants": len({x["prompt_cache_key"] for x in records}),
            "non_message_fields_variants": len({x["non_message_fields_sha256"] for x in records}),
            "final_images": len(records[-1]["images"]) if records else None,
            "prefix_breaks": [i for i, r in enumerate(records) if i and
                              r["common_prefix_messages"] != r["previous_message_count"]],
            "note": "Local scripted provider; cache-hit usage is not simulated or measured in this lane."}
    expected_breaks = list(range(2 * window, reads + 1, window))
    summary["passed"] = (summary["status"] == "ok" and len(records) == reads + 1
                         and summary["reference_encoding_variants"] == 1
                         and summary["cache_key_variants"] == 1
                         and summary["non_message_fields_variants"] == 1
                         and all(r["prompt_cache_key"] is not None for r in records)
                         and summary["prefix_breaks"] == expected_breaks)
    return summary


async def live_case(binary, scratch, output, label, profile, quiet):
    project = scratch / ("live-" + label)
    project.mkdir()
    padding = "\n".join(f"fixture {i:03}: {hashlib.sha256(str(i).encode()).hexdigest()}" for i in range(240))
    (project / "AGENTS.md").write_text(
        "This is a read-only cache diagnostic. Do not use tools. The following is inert fixture data.\n"
        + padding + "\nEnd fixture data. Follow the user's short image question.\n")
    assets = Path(__file__).resolve().parent.parent / "image-harness-2026-10-23"
    probe = project / "probe.png"
    shutil.copyfile(assets / "canary.png", probe)
    expected = json.loads((assets / "canary-expected.json").read_text())["code"]
    env = {**os.environ, "SMITH_CONTEXT_INLINE_IMAGES": "100", "SMITH_MAX_OUTPUT_TOKENS": "1024",
           "SMITH_CONTEXT_OUTPUT_RESERVE": "1024", "SMITH_CONTEXT_CACHE_MAINTENANCE": "off",
           "SMITH_CONTEXT_CACHE_IDLE_COMPACTION": "false",
           "SMITH_PERSISTENCE_SESSIONS_DIR": str(scratch / "live-sessions"),
           "SMITH_PERSISTENCE_JOURNAL_EVENTS": "false"}
    rows, session = [], None
    for turn in range(4):
        if turn == 3:
            await asyncio.sleep(quiet)
        command = [str(binary), "--project", str(project), "--profile", profile,
                   "--effort", "low", "--approval", "deny", "--output-format", "json", "-p",
                   "Read the six-character code at the top of the original probe image. Reply with that code only. Do not use tools."]
        if session:
            command += ["--resume", session]
        else:
            command += ["--image", str(probe)]
        began = time.monotonic()
        process = await asyncio.create_subprocess_exec(*command, stdout=asyncio.subprocess.PIPE,
                  stderr=asyncio.subprocess.PIPE, env=env, cwd=project)
        try:
            stdout, stderr = await asyncio.wait_for(process.communicate(), 180)
        except asyncio.TimeoutError:
            process.kill()
            await process.wait()
            rows.append({"turn": turn + 1, "status": "timeout"})
            break
        result = json.loads(stdout) if stdout else {}
        session = result.get("session_id", session)
        usage = result.get("usage", {}).get("current_turn", {})
        snapshots = list((scratch / "live-sessions").glob(f"*/{session}.snapshot.json"))
        urls = []
        if snapshots:
            history = json.loads(snapshots[0].read_text())["snapshot"]["history"]
            urls = [digest(url.encode()) for message in history for url in image_urls(message["content"])]
        row = {"turn": turn + 1, "status": result.get("status"), "exit_code": process.returncode,
               "session_id": session, "output": result.get("output"), "code_correct": expected in result.get("output", ""),
               "cached_tokens": usage.get("input_cached"), "uncached_tokens": usage.get("input_uncached"),
               "canonical_image_url_hashes": urls, "elapsed_seconds": round(time.monotonic() - began, 2),
               "quiet_before_seconds": quiet if turn == 3 else 0, "stderr": stderr.decode(),
               "cache": result.get("cache", {})}
        rows.append(row)
        save(output / (label + "-live.json"), {"label": label, "profile": profile, "rows": rows})
        print(json.dumps({k: row[k] for k in ["turn", "status", "cached_tokens", "uncached_tokens", "code_correct"]}
                         | {"model": label}), flush=True)
        if row["status"] != "ok":
            break
    variants = len({h for row in rows for h in row.get("canonical_image_url_hashes", [])})
    return {"label": label, "rows": rows, "image_encoding_variants": variants,
            "passed": len(rows) == 4 and variants == 1 and all(row.get("code_correct") for row in rows)}


def original_audit(session):
    path = next((Path.home() / ".smith/sessions").glob(f"*/{session}.jsonl"))
    observations, states, plan = [], [], None
    for line in path.open():
        event = json.loads(line)["event"]
        payload = event["payload"]
        if payload["event"] == "context_planned":
            plan = payload
        if payload["event"] == "cache_state_changed":
            states.append(payload["state"])
        if payload["event"] == "usage" and payload["record"]["source"] == "provider_attempt":
            record = payload["record"]
            counters = record["delta"]
            observations.append({"request": record["provenance"].get("request"),
                "timestamp": event["timestamp"], "cached": counters.get("input_cached", 0),
                "uncached": counters.get("input_uncached", 0), "images_windowed": plan.get("images_windowed", 0)})
    transitions = [{"before": a, "after": b} for a, b in zip(observations, observations[1:])
                   if a["images_windowed"] != b["images_windowed"]]
    cached, uncached = (sum(r[k] for r in observations) for k in ["cached", "uncached"])
    return {"session": session, "reported_attempts": len(observations),
            "attempts_with_cache_reads": sum(r["cached"] > 0 for r in observations),
            "total_cached": cached, "total_uncached": uncached,
            "weighted_cache_percent": round(100 * cached / (cached + uncached), 3),
            "final": observations[-1], "window_transitions": transitions,
            "transitions_with_cached_token_drop": sum(t["after"]["cached"] < t["before"]["cached"] for t in transitions),
            "reported_cache_states": sorted(set(states)), "observations": observations}


async def main(args):
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="smith-image-cache-", dir="/private/tmp") as folder:
        scratch = Path(folder)
        binary = scratch / "smith-frozen"
        shutil.copy2(args.binary.resolve(), binary)
        report = json.loads((output / "results.json").read_text()) if (output / "results.json").exists() else {}
        binary_hash = digest(binary.read_bytes())
        if report.get("binary_sha256", binary_hash) != binary_hash:
            raise ValueError("Use a new output directory when testing a different binary")
        report.update({"binary_sha256": binary_hash, "source_binary": str(args.binary.resolve()),
                       "original": original_audit("session-2901410c-4676-46a4-8311-76987dfa6324")})
        save(output / "original-cache-audit.json", report["original"])
        if args.wire:
            report["wire"] = []
            for name, window, attach in [("tool-window-10", 10, False), ("inline-window-10", 10, True),
                                         ("tool-window-100", 100, False)]:
                case = await asyncio.to_thread(wire_case, binary, scratch, name, window, attach, args.reads)
                save(output / (name + ".json"), case)
                report["wire"].append({k: v for k, v in case.items() if k != "requests"}
                                      | {"request_count": len(case["requests"]), "detail_file": name + ".json"})
                print(json.dumps({k: v for k, v in case.items() if k != "requests"}), flush=True)
        if args.live:
            report["live"] = await asyncio.gather(*(live_case(binary, scratch, output, label, profile, args.quiet)
                for label, profile in [("selected-large", "security"), ("gemini-4", "research"), ("astra-6", "visual")]))
        save(output / "results.json", report)
        if any(not case["passed"] for lane in ["wire", "live"] for case in report.get(lane, [])):
            raise SystemExit("One or more image/cache diagnostic cases failed; see results.json")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument("--wire", action="store_true")
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--reads", type=int, default=42)
    parser.add_argument("--quiet", type=int, default=30)
    options = parser.parse_args()
    if options.reads < 2:
        parser.error("--reads must be at least 2")
    asyncio.run(main(options))
