#!/usr/bin/env python3
"""Run the same image-to-HTML task through Smith and Codex CLI."""

import asyncio
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import time

ROOT = Path(__file__).resolve().parent


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


async def run_case(label, environment):
    project = Path(environment["scratch"]) / label
    harness, model_label = label.split("-", 1)
    model = "gpt-6.1-sol" if model_label == "sol" else "gpt-6-astra"
    shutil.copyfile(ROOT / "reference.png", project / "reference.png")
    instructions = (
        "This directory is an isolated screenshot-reconstruction benchmark. "
        "Use only the attached reference and local files. Work only here, do not delegate, "
        "and do not inspect parent/sibling directories. Use self-contained vanilla HTML, CSS, "
        "and JavaScript. Do not use reference.png as a rendered page asset. "
        "Document inferred design tokens in DESIGN.md before writing UI code.\n"
    )
    (project / "AGENTS.md").write_text(instructions)
    env = {**os.environ, "TMPDIR": "/private/tmp"}
    prompt = (ROOT / "prompt.txt").read_text()
    if harness == "smith":
        (project / ".smith").mkdir()
        (project / ".smith/config.toml").write_text(f'''profile_order = ["web-benchmark"]
[profiles.web-benchmark]
provider = "chatgpt"
model = "{model}"
posture = "build"
use = ["main"]
delegation = false
advisor = false
vision = false
max_output_tokens = 16384
''')
        env.update({"SMITH_PERSISTENCE_SESSIONS_DIR": str(Path(environment["scratch"]) / "sessions"),
                    "SMITH_CONTEXT_CACHE_MAINTENANCE": "off", "SMITH_CONTEXT_CACHE_IDLE_COMPACTION": "false"})
        command = [environment["smith_binary"], "--project", str(project), "--profile", "web-benchmark",
                   "--effort", "medium", "--approval", "allow-all", "--output-format", "stream-json",
                   "--image", str(project / "reference.png"), "-p", "-"]
    else:
        command = [environment["codex_binary"], "exec", "--ignore-user-config", "--ignore-rules",
                   "--ephemeral", "--skip-git-repo-check", "--cd", str(project),
                   "--sandbox", "workspace-write", "-c", 'approval_policy="never"',
                   "-c", 'model_reasoning_effort="medium"', "--model", model,
                   "--image", str(project / "reference.png"), "--json", "-"]
    started = time.monotonic()
    record = {"case": label, "harness": harness, "model": model, "status": "running",
              "started_at_epoch": time.time(), "command": command, "project": str(project),
              "image_sha256": hashlib.sha256((project / "reference.png").read_bytes()).hexdigest(),
              "prompt_sha256": hashlib.sha256(prompt.encode()).hexdigest()}
    save(ROOT / (label + "-run.json"), record)
    print(json.dumps({"case": label, "phase": "started", "model": model}), flush=True)
    with (ROOT / (label + "-events.jsonl")).open("wb") as stdout, (ROOT / (label + ".stderr")).open("wb") as stderr:
        process = await asyncio.create_subprocess_exec(*command, stdin=asyncio.subprocess.PIPE,
                  stdout=stdout, stderr=stderr, cwd=project, env=env, start_new_session=True)
        try:
            await asyncio.wait_for(process.communicate(prompt.encode()), environment["timeout_seconds"])
            record.update({"status": "completed" if process.returncode == 0 else "failed",
                           "exit_code": process.returncode})
        except asyncio.TimeoutError:
            os.killpg(process.pid, signal.SIGKILL)
            await process.wait()
            record.update({"status": "timeout", "exit_code": process.returncode})
    record["elapsed_seconds"] = round(time.monotonic() - started, 2)
    record["index_html_exists"] = (project / "index.html").exists()
    record["observations_exists"] = (project / "observations.json").exists()
    target = ROOT / "outputs" / label
    target.mkdir(parents=True, exist_ok=True)
    record["generated_files"] = []
    for path in project.rglob("*"):
        relative = path.relative_to(project)
        if path.is_file() and relative.parts[0] not in [".smith", ".git"] and relative.name not in ["reference.png", "AGENTS.md"]:
            destination = target / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, destination)
            record["generated_files"].append(str(relative))
    save(ROOT / (label + "-run.json"), record)
    print(json.dumps({k: record[k] for k in ["case", "status", "elapsed_seconds", "index_html_exists", "observations_exists"]}), flush=True)
    return record


async def main():
    environment = json.loads((ROOT / "run-environment.json").read_text())
    results = []
    for wave in [["codex-sol", "smith-astra"], ["smith-sol", "codex-astra"]]:
        results.extend(await asyncio.gather(*(run_case(label, environment) for label in wave)))
        save(ROOT / "runs.json", results)


if __name__ == "__main__":
    asyncio.run(main())
