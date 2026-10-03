#!/usr/bin/env python3
"""Compare prompt-cache reads between two Smith builds on live providers.

Repeats the v0.3.0 method (docs/qa/advisor-2026-10-03/live-walkthrough.md):
three headless turns on a scratch Git project holding a ~50 KB file, read in
turn 1 and followed up twice with `--resume`, for each model and each build,
on identical prompts. Uses the owner's real configuration and credentials;
nothing in it is edited.

    python3 cache_ab.py --old ~/.local/bin/smith --new target/release/smith
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

MODELS = (("zai", "glm-5.3"), ("google", "gemini-3.8-flash"), ("xai", "grok-4.3"))
PROMPTS = (
    "Read big.py and tell me in one sentence what the file contains. Do not edit anything.",
    "Without reading the file again, which function in big.py did you see last? One sentence.",
    "Without reading the file again, name one function from the middle of big.py. One sentence.",
)


def make_project(root):
    project = root / "project"
    project.mkdir(parents=True)
    body = []
    for index in range(600):
        body.append(
            f"def function_{index:03d}(value):\n"
            f"    \"\"\"Return value scaled by {index}.\"\"\"\n"
            f"    return value * {index} + {index % 7}\n\n"
        )
    (project / "big.py").write_text("".join(body))
    subprocess.run(["git", "init", "-q", "-b", "main"], cwd=project, check=True)
    subprocess.run(["git", "add", "."], cwd=project, check=True)
    subprocess.run(
        ["git", "-c", "user.name=qa", "-c", "user.email=qa@example.test", "commit", "-q", "-m", "fixture"],
        cwd=project, check=True,
    )
    return project


def turn(binary, project, provider, model, prompt, session=None):
    args = [str(binary), "-p", prompt, "--output-format", "stream-json",
            "--provider", provider, "--model", model]
    if session:
        args += ["--resume", session]
    run = subprocess.run(args, cwd=project, capture_output=True, text=True, timeout=600,
                         env={**os.environ, "TMPDIR": "/private/tmp"})
    result = None
    for line in run.stdout.splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("type") == "result":
            result = record
    if result is None:
        return {"error": (run.stderr or run.stdout)[-400:]}
    usage = result.get("usage", {}).get("current_turn", {})
    cache = result.get("cache", {})
    return {
        "session": result.get("session_id"),
        "status": result.get("status"),
        "cached": usage.get("input_cached", 0),
        "uncached": usage.get("input_uncached", 0),
        "state": cache.get("state"),
        "misses": cache.get("miss_count"),
        "rebilled": cache.get("rebilled_tokens"),
        "preserved": cache.get("preserved_prefix_tokens"),
        "invalidated": cache.get("invalidated_prefix_tokens"),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--old", required=True)
    parser.add_argument("--new", required=True)
    parser.add_argument("--out", default=str(Path(__file__).resolve().parent / "cache-ab.json"))
    args = parser.parse_args()
    builds = {}
    for label, binary in (("old", args.old), ("new", args.new)):
        path = Path(binary).expanduser().resolve()
        version = subprocess.run([str(path), "--version"], capture_output=True, text=True).stdout.strip()
        builds[label] = (path, version)
        print(f"{label}: {path} ({version})")

    results = []
    with tempfile.TemporaryDirectory(dir="/private/tmp", prefix="smith-cache-ab-") as tmp:
        for provider, model in MODELS:
            for label, (binary, version) in builds.items():
                project = make_project(Path(tmp) / f"{provider}-{label}")
                session = None
                for index, prompt in enumerate(PROMPTS, start=1):
                    row = turn(binary, project, provider, model, prompt, session)
                    session = row.get("session") or session
                    row.update({"model": f"{provider}/{model}", "build": f"{label} {version}", "turn": index})
                    results.append(row)
                    print(json.dumps(row), flush=True)
                    if "error" in row:
                        break
    Path(args.out).write_text(json.dumps(results, indent=2) + "\n")
    failed = any("error" in row or row.get("status") != "ok" for row in results)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
