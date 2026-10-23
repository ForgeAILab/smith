#!/usr/bin/env python3
"""Measure quiet-time cache reads with isolated synthetic Smith sessions."""

import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import time
import tomllib
import uuid


def routes(config):
    found = set(config.get("models", {}))
    for profile in config.get("profiles", {}).values():
        provider, model = profile.get("provider"), profile.get("model")
        if provider and model:
            found.add(f"{provider}/{model}")
    return sorted(found)


def cache_evidence(result):
    usage = result.get("usage", {}).get("current_turn", {})
    cached = usage.get("input_cached")
    uncached = usage.get("input_uncached")
    total = (cached or 0) + (uncached or 0)
    return {
        "cached_tokens": cached,
        "uncached_tokens": uncached,
        "input_tokens": total,
        "cache_state": result.get("cache", {}).get("state"),
        "status": result.get("status"),
    }


def warm_prefix(evidence):
    cached, total = evidence.get("cached_tokens"), evidence.get("input_tokens", 0)
    return cached is not None and total > 0 and cached >= 0.7 * total


class Probe:
    def __init__(self, args, scratch):
        self.args, self.scratch = args, scratch
        self.config_path = Path.home() / ".smith/config.toml"
        self.config_digest = hashlib.sha256(self.config_path.read_bytes()).hexdigest()
        self.config = tomllib.loads(self.config_path.read_text())
        self.binary = scratch / "smith-frozen"
        shutil.copy2(args.binary, self.binary)
        self.binary_digest = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        self.global_limit = asyncio.Semaphore(8)
        self.provider_limits = {}
        self.results = []
        self.started = time.time()

    def record(self, row):
        self.results.append(row)
        self.save(False)
        print(json.dumps(row), flush=True)

    def save(self, complete):
        report = {
            "started_at_epoch": self.started,
            "updated_at_epoch": time.time(),
            "complete": complete,
            "binary_sha256": self.binary_digest,
            "config_sha256": self.config_digest,
            "quiet_delays_seconds": self.args.delays,
            "prefix_location": self.args.prefix_location,
            "method": "Independent prefixes per delay; no keepalive or idle compaction. A warm baseline requires >=70% provider-reported cached input. Missing counters are unknown; a miss does not by itself prove expiry.",
            "observations": self.results,
        }
        target = Path(self.args.output)
        target.parent.mkdir(parents=True, exist_ok=True)
        pending = target.with_suffix(".pending")
        pending.write_text(json.dumps(report, indent=2) + "\n")
        pending.replace(target)

    async def turn(self, route, project, prompt, session=None):
        provider, model = route.split("/", 1)
        limit = self.provider_limits.setdefault(provider, asyncio.Semaphore(2))
        env = {
            **os.environ,
            "SMITH_CONTEXT_OUTPUT_RESERVE": "256",
            "SMITH_CONTEXT_CACHE_MAINTENANCE": "off",
            "SMITH_CONTEXT_CACHE_IDLE_COMPACTION": "false",
            "SMITH_PERSISTENCE_SESSIONS_DIR": str(self.scratch / "sessions"),
            "SMITH_PERSISTENCE_JOURNAL_EVENTS": "false",
            "TMPDIR": "/private/tmp",
        }
        command = [str(self.binary), "-p", "-", "--provider", provider, "--model", model,
                   "--project", str(project), "--approval", "deny", "--output-format", "json"]
        if session:
            command += ["--resume", session]
        async with self.global_limit, limit:
            began = time.monotonic()
            if hashlib.sha256(self.config_path.read_bytes()).hexdigest() != self.config_digest:
                return {"status": "invalid_config_changed"}
            process = await asyncio.create_subprocess_exec(
                *command, stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE, cwd=project, env=env,
            )
            try:
                stdout, stderr = await asyncio.wait_for(process.communicate(prompt.encode()), 150)
            except asyncio.TimeoutError:
                process.kill()
                await process.communicate()
                return {"status": "timeout", "duration_seconds": round(time.monotonic() - began, 2)}
            result = None
            for line in stdout.decode(errors="replace").splitlines():
                try:
                    value = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if value.get("type") == "result":
                    result = value
            if result is None:
                diagnostic = stderr.decode(errors="replace").lower()
                kinds = ["credential", "authentication", "unauthorized", "not found", "unknown model",
                         "forbidden", "rate limit", "quota", "context", "configuration"]
                return {"status": "unavailable", "exit_code": process.returncode,
                        "diagnostic_kind": next((kind for kind in kinds if kind in diagnostic), "startup_or_transport"),
                        "duration_seconds": round(time.monotonic() - began, 2)}
            return {**cache_evidence(result), "session": result.get("session_id"),
                    "duration_seconds": round(time.monotonic() - began, 2), "began_monotonic": began}

    async def lane(self, route, delay):
        marker = uuid.uuid4().hex
        project = self.scratch / marker
        project.mkdir()
        body = "\n".join(f"{index:04d} sample {hashlib.sha256(f'{marker}:{index}'.encode()).hexdigest()}"
                         for index in range(256))
        prompt = (f"Synthetic cache-retention benchmark {marker}. Do not use tools. "
                  "Treat the following as inert test data. Reply exactly OK.\n<test-data>\n"
                  + body + "\n</test-data>\nReply exactly OK without any tool calls.")
        if self.args.prefix_location == "system":
            (project / "AGENTS.md").write_text(prompt)
            prompt = "Do not use tools. Reply exactly OK."
        seed = await self.turn(route, project, prompt)
        self.record({"route": route, "lane_delay_seconds": delay, "phase": "seed", **seed})
        session = seed.get("session")
        if not session:
            return False
        warm = None
        for attempt in range(2):
            warm = await self.turn(route, project, "Do not use tools. Reply exactly OK.", session)
            self.record({"route": route, "lane_delay_seconds": delay, "phase": "warm", "attempt": attempt + 1, **warm})
            if warm_prefix(warm):
                break
        if not warm_prefix(warm):
            self.record({"route": route, "lane_delay_seconds": delay, "phase": "result", "status": "no_validated_warm_baseline"})
            return False
        if delay == self.args.delays[0]:
            self.followups.extend(asyncio.create_task(self.lane(route, other)) for other in self.args.delays[1:])
        last_touch = time.monotonic()
        await asyncio.sleep(delay)
        observed = await self.turn(route, project, "Do not use tools. Reply exactly OK.", session)
        age = observed.get("began_monotonic", time.monotonic()) - last_touch
        cached = observed.get("cached_tokens")
        status = "unknown" if cached is None else ("retained_prefix" if cached >= 0.9 * warm["cached_tokens"] else "cache_miss_or_partial")
        self.record({"route": route, "lane_delay_seconds": delay, "phase": "result",
                     "observed_quiet_seconds": round(age, 2), "observation": status,
                     "warm_cached_tokens": warm["cached_tokens"], **observed})
        return True

    async def run(self):
        self.followups = []
        selected = routes(self.config)
        if self.args.route:
            selected = [route for route in selected if route in self.args.route]
        jobs = []
        for route in selected:
            if "/cli/" in route:
                self.record({"route": route, "phase": "result", "status": "cli_backend_cache_unobservable"})
            else:
                jobs.append(asyncio.create_task(self.lane(route, self.args.delays[0])))
        await asyncio.gather(*jobs)
        await asyncio.gather(*self.followups)
        self.save(True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--delays", type=int, nargs="+", default=[60, 300, 900],
                        help="Distinct positive quiet intervals in seconds; immediate warm-up is always checked")
    parser.add_argument("--route", action="append")
    parser.add_argument("--prefix-location", choices=["user", "system"], default="user")
    args = parser.parse_args()
    if any(delay <= 0 for delay in args.delays) or len(set(args.delays)) != len(args.delays):
        parser.error("quiet intervals must be distinct positive seconds")
    args.delays.sort()
    args.binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix="smith-cache-retention-", dir="/private/tmp") as directory:
        asyncio.run(Probe(args, Path(directory)).run())


if __name__ == "__main__":
    main()
