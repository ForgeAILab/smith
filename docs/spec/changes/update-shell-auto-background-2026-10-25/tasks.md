---
created_at: 2026-10-25T20:16:29Z
updated_at: 2026-10-25T23:18:06Z
completed_at: 2026-10-25T23:18:06Z
---

## 1. Output still held after the shell exits (shipped as a bug fix)

- [x] 1.1 Bound the post-exit drain in `shell.rs` at 2 s and honor
      cancellation during it; after a kill, wait for the pipe to close and
      escalate to SIGKILL instead of waiting for it unbounded
- [x] 1.2 Hand a process still holding the pipe to the background host; stop
      it with its group when no host can take it
- [x] 1.3 Keep a background task whose shell exited first stoppable by
      `task_stop`, its deadline, and session shutdown
- [x] 1.4 Tests for each of the above

## 2. Auto-background at 5 minutes

- [x] 2.1 Add the remaining deadline to `BackgroundTaskHost::adopt`; manual
      backgrounding passes none
- [x] 2.2 Race a 300000 ms timer in the foreground loop when `timeout_ms` is
      longer; when it fires, adopt the call with the remaining deadline
- [x] 2.3 Return the task ID, the output so far, and a statement that the
      command has not completed and will still be killed at its timeout;
      mark the result as moved automatically, not by the user
- [x] 2.4 Render the result in the TUI and headless output through the
      existing backgrounded-call path (no code change: both show the
      result text, and the task appears with the session's running tasks)
- [x] 2.5 Update the `shell` tool's `timeout_ms` and `run_in_background`
      descriptions
- [x] 2.6 Tests with an injected short threshold: the handoff fires, the
      adopted task is still killed at its deadline, and a timeout at or below
      the threshold still kills in the foreground
- [x] 2.7 When a handoff cannot be adopted, stop the process group instead
      of leaving it running (also covers ctrl+b, which used to leave it)

## 3. Verify

- [x] 3.1 Full gate, then push the branch for CI before main
