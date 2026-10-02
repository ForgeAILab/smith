#!/usr/bin/env python3
"""Re-run the 2026-09-21 PTY sweeps against a Smith binary.

The 26-check command sweep and the 40-check startup sweep were first run on
2026-09-21 (docs/qa/smith-ux-2026-09-21/command-sweep-after.json and
startup-ux-checks-after.json); their drivers were never checked in. This
driver rebuilds both from those result lists and the captures beside them.

Each run uses an isolated HOME, a disposable Git project, fake providers, and
a private tmux server, so it never reads the owner's configuration or
credentials. Usage:

    python3 run_sweeps.py [--binary PATH] [--out DIR]

Results are written to DIR/command-sweep.json and DIR/startup-checks.json, and
every check's final frame to DIR/captures/.
"""

import argparse
import http.server
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

TMUX_SOCKET = "smith-sweep"

FIXTURE_CONFIG = """
default_profile = "build"

[profiles.build]
description = "Make and test changes"
posture = "build"
use = ["main", "child"]
provider = "local"
model = "example-model"

[profiles.plan]
description = "Plan before changing files"
posture = "plan"
use = ["main", "child"]
provider = "local"
model = "example-model"

[profiles.review]
description = "Read-only change review"
posture = "review"
use = ["main", "child"]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[providers.other]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[models."local/small-model"]
context_tokens = 32768
max_input_tokens = 28672
max_output_tokens = 4096

[models."other/alternative-model"]
context_tokens = 64000
max_input_tokens = 60000
max_output_tokens = 4096
"""


class Pane:
    """One Smith process in a private tmux session."""

    def __init__(self, binary, home, project, width, height, extra_env=None, args=()):
        self.name = f"s{os.getpid()}{int(time.time() * 1000) % 100000}"
        env = {
            "HOME": str(home),
            "TMPDIR": "/private/tmp",
            "TERM": "xterm-256color",
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        }
        env.update(extra_env or {})
        env_args = []
        for key, value in env.items():
            env_args += ["-e", f"{key}={value}"]
        command = " ".join([str(binary), *args]) + "; echo SMITH-EXIT=$?; sleep 300"
        tmux(
            "new-session", "-d", "-s", self.name, "-x", str(width), "-y", str(height),
            "-c", str(project), *env_args, command,
        )

    def send_text(self, text):
        tmux("send-keys", "-t", self.name, "-l", text)

    def send_key(self, *keys):
        tmux("send-keys", "-t", self.name, *keys)

    def frame(self):
        return tmux("capture-pane", "-p", "-t", self.name)

    def wait_for(self, needle, timeout=10.0, echo=None):
        """Waits for `needle` on a line other than the echoed `echo` command.

        Many expected words are also part of the command that was typed
        (`/goal` -> `goal`), so the echo line alone must not count, and when
        the command is echoed into the transcript only the lines after that
        echo count, so an earlier command's output cannot satisfy a later one.
        """
        deadline = time.time() + timeout
        frame = ""
        while time.time() < deadline:
            frame = self.frame()
            lines = frame.splitlines()
            if echo is not None:
                echoes = [
                    index for index, line in enumerate(lines)
                    if line.strip().lstrip("›").strip() == echo
                ]
                if echoes:
                    lines = lines[echoes[-1] + 1:]
            if any(
                needle in line and line.strip().lstrip("›").strip() != echo
                for line in lines
            ):
                return True, frame
            time.sleep(0.2)
        return False, frame

    def settle(self, delay=0.6):
        time.sleep(delay)
        return self.frame()

    def close(self):
        subprocess.run(
            ["tmux", "-L", TMUX_SOCKET, "kill-session", "-t", self.name],
            capture_output=True,
        )


def tmux(*args):
    result = subprocess.run(
        ["tmux", "-L", TMUX_SOCKET, *args], capture_output=True, text=True
    )
    if result.returncode != 0 and args[0] != "kill-session":
        raise RuntimeError(f"tmux {' '.join(args)}: {result.stderr.strip()}")
    return result.stdout


class Recorder:
    def __init__(self, out):
        self.out = out
        self.captures = out / "captures"
        self.captures.mkdir(parents=True, exist_ok=True)
        self.results = []

    def record(self, name, passed, frame, detail=""):
        slug = "".join(ch if ch.isalnum() else "-" for ch in name).strip("-")
        (self.captures / f"sweep-{slug}.txt").write_text(frame)
        entry = {"check": name, "pass": bool(passed)}
        if detail:
            entry["detail"] = detail
        self.results.append(entry)
        mark = "PASS" if passed else "FAIL"
        print(f"{mark}  {name}" + (f"  ({detail})" if detail and not passed else ""))


def make_project(root):
    project = root / "project"
    project.mkdir(parents=True)
    subprocess.run(["git", "init", "-q", "-b", "main"], cwd=project, check=True)
    (project / "README.md").write_text("sweep fixture\n")
    subprocess.run(["git", "add", "."], cwd=project, check=True)
    subprocess.run(
        ["git", "-c", "user.name=sweep", "-c", "user.email=sweep@example.test",
         "commit", "-q", "-m", "fixture"],
        cwd=project, check=True,
    )
    return project


def make_home(root, configured=True):
    home = root / ("home" if configured else "home-empty")
    (home / ".smith").mkdir(parents=True)
    if configured:
        (home / ".smith" / "config.toml").write_text(FIXTURE_CONFIG)
    return home


def wait_ready(pane):
    return pane.wait_for("Ask Smith", timeout=20)


def dismiss(pane):
    pane.send_key("Escape")
    time.sleep(0.4)
    pane.send_key("C-u")
    time.sleep(0.2)


COMMAND_CHECKS = [
    ("/status", "session"),
    ("/diagnostics", "cache maintenance"),
    ("/context", "system"),
    ("/goal", "goal"),
    ("/details", "detail"),
    # The 09-21 run expected "timeline"; the empty state now reads
    # "No turns, children, or recovery actions yet."
    ("/timeline", "No turns"),
    ("/agent", "agent"),
    ("/mcp", "MCP"),
    ("/skills", "smith.security"),
    ("/diff", "changes"),
    ("/review", "review"),
    ("/undo", "no Smith turn has attributable changes"),
    ("/redo", "redo"),
    ("/revert", "revert"),
    ("/account", "credential"),
    ("/think", "thinking"),
    ("/effort", "effort"),
    ("/model", "Choose model"),
    ("/provider", "Choose provider"),
    ("/profile", "Choose profile"),
    ("/connect", "Connect provider"),
    ("/disconnect", "Disconnect provider"),
    ("/resume", "Resume session"),
    ("first prompt", "Worked for"),
    ("/model local/small-model", "local/small-model"),
    ("/profile plan", "plan"),
]


def command_sweep(binary, root, recorder):
    project = make_project(root)
    home = make_home(root)
    pane = Pane(binary, home, project, 100, 32, args=("--approval", "ask"))
    try:
        ready, frame = wait_ready(pane)
        if not ready:
            recorder.record("command sweep startup", False, frame, "composer never appeared")
            return
        for command, expected in COMMAND_CHECKS:
            tmux("clear-history", "-t", pane.name)
            if command == "first prompt":
                pane.send_text("reply with ok")
                pane.send_key("Enter")
                passed, frame = pane.wait_for(expected, timeout=20)
            else:
                pane.send_text(command)
                time.sleep(0.3)
                pane.send_key("Enter")
                passed, frame = pane.wait_for(expected, timeout=6, echo=command)
            recorder.record(command, passed, frame, f"expected {expected!r}")
            # Pickers and confirmations close on Esc; plain output ignores it.
            dismiss(pane)
    finally:
        pane.close()


def startup_checks(binary, root, recorder, width, height):
    tag = f"{width}x{height}"
    project = make_project(root / tag)
    home = make_home(root / tag)
    pane = Pane(binary, home, project, width, height)
    try:
        ready, frame = wait_ready(pane)
        recorder.record(
            f"guide {tag}",
            ready and "/model" in frame and "/help" in frame,
            frame,
        )

        pane.send_text("/")
        frame = pane.settle()
        menu = [line for line in frame.splitlines() if line.lstrip().startswith(("/", "› /"))]
        recorder.record(
            f"bounded command menu {tag}",
            len(menu) >= 2 and "Ask Smith" not in frame and len(frame.splitlines()) <= height,
            frame,
        )

        pane.send_text("stat")
        time.sleep(0.4)
        pane.send_key("Enter")
        passed, frame = pane.wait_for("goal:", timeout=6)
        recorder.record(f"highlighted status executes {tag}", passed, frame)
        dismiss(pane)

        pane.send_text("/help")
        time.sleep(0.3)
        pane.send_key("Enter")
        passed, start = pane.wait_for("/status", timeout=6)
        recorder.record(f"help opens at start {tag}", passed, start)

        # Help opens at its first line; paging reaches the end of the list.
        scrolled = start
        pages = 0
        while "/quit" not in scrolled and pages < 12:
            pane.send_key("PageDown")
            scrolled = pane.settle(0.4)
            pages += 1
        recorder.record(
            f"help scrolling {tag}",
            scrolled != start and "/quit" in scrolled,
            scrolled,
            f"{pages} pages",
        )
        pane.send_key("End")
        dismiss(pane)

        pane.send_text("/model")
        time.sleep(0.3)
        pane.send_key("Enter")
        passed, picker = pane.wait_for("Choose model", timeout=6)
        recorder.record(
            f"model current state {tag}",
            passed and "current" in picker and "example-model" in picker,
            picker,
        )
        recorder.record(
            f"model action hints {tag}",
            passed and "esc" in picker.lower() and ("enter" in picker.lower()),
            picker,
        )

        pane.send_text("zz-no-such-model")
        miss = pane.settle()
        recorder.record(
            f"filter miss recovery {tag}",
            "No matches" in miss and "clear" in miss.lower(),
            miss,
        )
        recorder.record(
            f"filter miss does not suggest setup {tag}",
            "No matches" in miss and "/connect" not in miss.split("No matches", 1)[1]
            and "setup" not in miss.split("No matches", 1)[1].lower(),
            miss,
        )

        pane.send_key("C-u")
        cleared = pane.settle()
        recorder.record(
            f"clear filter without choosing {tag}",
            "Choose model" in cleared and "example-model" in cleared and "No matches" not in cleared,
            cleared,
        )
        dismiss(pane)
        after = pane.settle()
        if "local/example-model" not in after:
            recorder.results[-1]["pass"] = False
            recorder.results[-1]["detail"] = "current model changed after clearing the filter"

        pane.send_text("/switch")
        intent = pane.settle()
        recorder.record(
            f"description search {tag}",
            "/model" in intent and "/profile" in intent,
            intent,
        )
        pane.send_key("Enter")
        passed, opened = pane.wait_for("Choose model", timeout=6)
        recorder.record(f"intent selection opens model {tag}", passed, opened)
        dismiss(pane)
    finally:
        pane.close()


def no_color_check(binary, root, recorder):
    project = make_project(root / "no-color")
    home = make_home(root / "no-color")
    pane = Pane(binary, home, project, 44, 16, extra_env={"NO_COLOR": "1"})
    try:
        wait_ready(pane)
        pane.send_text("/model")
        time.sleep(0.3)
        pane.send_key("Enter")
        passed, frame = pane.wait_for("Choose model", timeout=6)
        recorder.record(
            "no-color picker keeps state and actions",
            passed and "current" in frame and "esc" in frame.lower(),
            frame,
        )
    finally:
        pane.close()


class ModelsHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps({"object": "list", "data": [{"id": "fixture-model", "object": "model"}]})
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body.encode())

    def log_message(self, *_):
        pass


def setup_checks(binary, root, recorder):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ModelsHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    endpoint = f"http://127.0.0.1:{server.server_address[1]}/v1"
    project = make_project(root / "setup")
    home = make_home(root / "setup", configured=False)
    pane = Pane(binary, home, project, 80, 24)
    try:
        ready, frame = pane.wait_for("Smith setup", timeout=20)
        if not ready:
            for name in ("Back retains endpoint", "Back retains provider", "cancel setup writes no config"):
                recorder.record(name, False, frame, "setup never appeared")
            return
        pane.send_text("Custom")
        time.sleep(0.4)
        pane.send_key("Enter")
        pane.wait_for("Provider name", timeout=6)
        pane.send_text("back-fixture")
        pane.send_key("Enter")
        pane.wait_for("API base URL", timeout=6)
        pane.send_text(endpoint)
        pane.send_key("Enter")
        time.sleep(1.5)
        pane.send_key("BTab")
        passed, frame = pane.wait_for(endpoint, timeout=6)
        recorder.record(
            "Back retains endpoint", passed and "API base URL" in frame, frame
        )
        pane.send_key("BTab")
        passed, frame = pane.wait_for("back-fixture", timeout=6)
        recorder.record(
            "Back retains provider", passed and "Provider name" in frame, frame
        )
        pane.send_key("Escape")
        time.sleep(0.6)
        frame = pane.frame()
        if "SMITH-EXIT" not in frame:
            # A cancel confirmation, if any, is accepted with Enter or y.
            pane.send_key("Escape")
            time.sleep(0.6)
            frame = pane.frame()
        exited, frame = pane.wait_for("SMITH-EXIT", timeout=6)
        config = home / ".smith" / "config.toml"
        recorder.record(
            "cancel setup writes no config",
            exited and not config.exists(),
            frame,
            "" if not config.exists() else "config.toml was written",
        )
    finally:
        pane.close()
        server.shutdown()


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", default="target/debug/smith")
    parser.add_argument("--out", default=str(Path(__file__).resolve().parent / "results"))
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    if not binary.exists():
        sys.exit(f"no Smith binary at {binary}")
    out = Path(args.out)
    if out.exists():
        shutil.rmtree(out)
    version = subprocess.run([str(binary), "--version"], capture_output=True, text=True).stdout.strip()
    print(f"binary: {binary} ({version})")

    with tempfile.TemporaryDirectory(dir="/private/tmp", prefix="smith-sweep-") as tmp:
        root = Path(tmp)
        commands = Recorder(out)
        command_sweep(binary, root / "commands", commands)
        startup = Recorder(out)
        for width, height in ((100, 32), (80, 24), (44, 16)):
            startup_checks(binary, root / "startup", startup, width, height)
        no_color_check(binary, root, startup)
        setup_checks(binary, root, startup)
    subprocess.run(["tmux", "-L", TMUX_SOCKET, "kill-server"], capture_output=True)

    (out / "command-sweep.json").write_text(json.dumps(commands.results, indent=2) + "\n")
    (out / "startup-checks.json").write_text(json.dumps(startup.results, indent=2) + "\n")
    for label, results in (("command sweep", commands.results), ("startup checks", startup.results)):
        passed = sum(1 for result in results if result["pass"])
        print(f"{label}: {passed}/{len(results)} passed")
    failed = any(not result["pass"] for result in commands.results + startup.results)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
