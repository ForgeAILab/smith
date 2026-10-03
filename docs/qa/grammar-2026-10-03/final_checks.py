#!/usr/bin/env python3
"""Final PTY checks for adopt-claude-code-grammar (tasks 6.1 and 6.3).

Captures every surface the change touched at 100x32, 80x24, and 44x16, and in
no-colour mode at 80x24, then re-runs the command and startup sweeps with the
expectations this change's grammar produces. It reuses the isolated harness in
../smith-structure-2026-10-02/sweeps/run_sweeps.py: a throwaway HOME, a
disposable Git project, fake providers, and a private tmux server, so it never
reads the owner's configuration or credentials.

    python3 final_checks.py [--binary PATH] [--out DIR]

Writes DIR/checks.json and one capture per surface and size to DIR/captures/.
"""

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "smith-structure-2026-10-02" / "sweeps"))
import run_sweeps as base  # noqa: E402

SIZES = ((100, 32), (80, 24), (44, 16))
SHELL_COMMAND = "printf 'one\\ntwo\\nthree\\nfour\\nfive\\nsix\\n'"


def open_pane(binary, home, project, width, height, no_color=False, args=()):
    env = {"NO_COLOR": "1"} if no_color else None
    extra = ("--no-color",) if no_color else ()
    return base.Pane(binary, home, project, width, height, extra_env=env,
                     args=("--approval", "ask", *extra, *args))


def wait(pane, predicate, timeout=8.0):
    deadline = time.time() + timeout
    frame = pane.frame()
    while time.time() < deadline:
        frame = pane.frame()
        if predicate(frame):
            return True, frame
        time.sleep(0.2)
    return False, frame


def run_command(pane, command, predicate, timeout=8.0):
    base.tmux("clear-history", "-t", pane.name)
    pane.send_text(command)
    time.sleep(0.3)
    pane.send_key("Enter")
    return wait(pane, predicate, timeout)


def surfaces(binary, root, recorder, width, height, no_color=False):
    tag = f"{width}x{height}" + ("-nocolor" if no_color else "")
    project = base.make_project(root / tag)
    home = base.make_home(root / tag)
    pane = open_pane(binary, home, project, width, height, no_color)
    try:
        ready, frame = wait(pane, lambda f: "? for shortcuts" in f, timeout=20)
        recorder.record(f"startup {tag}", ready and "/help" in frame, frame)
        if not ready:
            return

        pane.send_text("/")
        passed, frame = wait(pane, lambda f: "/status" in f)
        recorder.record(f"command menu {tag}", passed and len(frame.splitlines()) <= height, frame)
        # The archived sweep waited for `goal:`; aligned columns have no colon.
        pane.send_text("stat")
        time.sleep(0.4)
        pane.send_key("Enter")
        passed, frame = wait(pane, lambda f: "● /status" in f and "permission" in f)
        recorder.record(f"highlighted status executes {tag}", passed, frame)
        base.dismiss(pane)

        pane.send_text("?")
        passed, frame = wait(pane, lambda f: "Shortcuts" in f and "any key closes" in f)
        recorder.record(f"shortcuts panel {tag}", passed, frame)
        pane.send_key("Escape")
        time.sleep(0.4)

        # `!` is a mode, not draft text: Backspace on the bare marker leaves it.
        pane.send_text("!")
        passed, frame = wait(pane, lambda f: "bash mode" in f)
        recorder.record(f"bash mode {tag}", passed, frame)
        pane.send_key("BSpace")
        # A long /status above can pause following, which replaces the idle hint.
        left, frame = wait(pane, lambda f: "> Ask Smith" in f and "bash mode" not in f)
        recorder.record(f"backspace leaves bash mode {tag}", left, frame)

        for command, needle in (
            ("/help", "Start here"),
            ("/status", "session"),
            ("/context", "context"),
            ("/diagnostics", "Session"),
        ):
            passed, frame = run_command(pane, command, lambda f, n=needle, c=command: f"● {c}" in f and n in f)
            extra = ""
            if command == "/diagnostics":
                passed = passed and "?/?/?" not in frame and " ? " not in frame
                extra = "headings, no ? values"
            recorder.record(f"{command} {tag}", passed, frame, extra)
            pane.send_key("End")
            base.dismiss(pane)

        for command, title in (("/model", "Choose model"), ("/profile", "Choose profile")):
            passed, frame = run_command(pane, command, lambda f, t=title: t in f)
            recorder.record(f"{command} picker {tag}", passed and "current" in frame, frame)
            base.dismiss(pane)

        passed, frame = run_command(pane, "reply with ok", lambda f: "Worked for" in f, timeout=20)
        recorder.record(f"first turn {tag}", passed, frame)

        passed, frame = run_command(pane, f"!{SHELL_COMMAND}", lambda f: "+2 lines" in f or "+ 2 lines" in f)
        recorder.record(
            f"shell echo {tag}",
            passed and frame.count("! printf") == 1,
            frame,
            "one echo row, folded result",
        )

        pane.send_key("C-c")
        time.sleep(0.3)
        pane.send_key("C-c")
        exited, frame = wait(pane, lambda f: "SMITH-EXIT" in f, timeout=10)
        # /status shortens the id at narrow widths; the sidecar names it whole.
        sidecars = sorted(home.rglob("*.shell.jsonl"))
        if not (exited and len(sidecars) == 1):
            recorder.record(f"resumed shell echo {tag}", False, frame, "no exit or no saved shortcut")
            return
        session = sidecars[0].name.removesuffix(".shell.jsonl")
    finally:
        pane.close()

    resumed = open_pane(binary, home, project, width, height, no_color, args=("--resume", session))
    try:
        passed, frame = wait(resumed, lambda f: "! printf" in f, timeout=20)
        lines = frame.splitlines()
        # The fake provider's answer to the earlier prompt stays on screen at
        # every size; the prompt itself scrolls off at 44x16.
        answer = next((i for i, line in enumerate(lines) if "fake provider" in line), None)
        echo = next((i for i, line in enumerate(lines) if "! printf" in line), None)
        in_place = answer is not None and echo is not None and answer < echo
        recorder.record(
            f"resumed shell echo {tag}",
            passed and in_place and frame.count("! printf") == 1,
            frame,
            "echo restored once, after the turn that preceded it",
        )
    finally:
        resumed.close()


def setup_surface(binary, root, recorder, width, height):
    tag = f"{width}x{height}"
    project = base.make_project(root / f"setup-{tag}")
    home = base.make_home(root / f"setup-{tag}", configured=False)
    pane = base.Pane(binary, home, project, width, height)
    try:
        passed, frame = wait(pane, lambda f: "Smith setup" in f, timeout=20)
        recorder.record(f"setup {tag}", passed and len(frame.splitlines()) <= height, frame)
    finally:
        pane.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", default="target/debug/smith")
    parser.add_argument("--out", default=str(Path(__file__).resolve().parent / "final"))
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    if not binary.exists():
        sys.exit(f"no Smith binary at {binary}")
    out = Path(args.out)
    if out.exists():
        shutil.rmtree(out)
    version = subprocess.run([str(binary), "--version"], capture_output=True, text=True).stdout.strip()
    print(f"binary: {binary} ({version})")

    recorder = base.Recorder(out)
    with tempfile.TemporaryDirectory(dir="/private/tmp", prefix="smith-final-") as tmp:
        root = Path(tmp)
        for width, height in SIZES:
            surfaces(binary, root, recorder, width, height)
        surfaces(binary, root, recorder, 80, 24, no_color=True)
        for width, height in SIZES:
            setup_surface(binary, root, recorder, width, height)
    subprocess.run(["tmux", "-L", base.TMUX_SOCKET, "kill-server"], capture_output=True)

    (out / "checks.json").write_text(json.dumps(recorder.results, indent=2) + "\n")
    passed = sum(1 for result in recorder.results if result["pass"])
    print(f"final checks: {passed}/{len(recorder.results)} passed")
    sys.exit(0 if passed == len(recorder.results) else 1)


if __name__ == "__main__":
    main()
