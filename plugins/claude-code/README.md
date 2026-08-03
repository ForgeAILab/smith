# Smith plugin for Claude Code

Lets Claude Code hand a coding task to a local Smith agent, running on your own
configured provider and model, and return Smith's answer verbatim.

## Install

```sh
/plugin marketplace add /Volumes/Data/codes/ai/tui/plugins/claude-code
/plugin install smith@smith
```

Use the path to your own checkout. Smith must already be on `PATH` and
configured — `smith setup` if it is not.

## Use

```
/smith:task    why does the retry path drop the cancellation?
/smith:task    --write fix the retry path so cancellation propagates
/smith:resume  session-336d1d86-... now add a regression test
/smith:sessions
```

Or let Claude pick it up on its own: the `smith-delegate` subagent is described
for exactly the cases where a second implementation or diagnosis pass by a
different agent is worth more than another turn on the main thread.

## Read-only by default

A dispatch runs with `--approval deny`, so Smith can read and search but every
mutating tool fails closed at its approval boundary. `--write` selects
`--approval allow-all` for that one run.

This matters more than it looks. `smith -p` is headless — there is no prompt to
answer — so an unauthorized action cannot pause and wait for you. The default
makes "Smith read your repository" and "Smith edited your repository unattended"
two different commands rather than two outcomes of the same one.

When a read-only dispatch needs to write, the tool call is declined and Smith
carries on and tells you so — the run still exits `0` with `status: "ok"`, and
the refusal is described in its answer. Verified: asked to overwrite a file
under `--approval deny`, Smith read it, prepared the overwrite, reported that
the action was declined, and left the file untouched.

That means you read the answer rather than the status. The subagent reports it
verbatim and names the `--write` form; it does not retry with wider authority
on its own.

For a stronger guarantee than denial, configure a read-only profile and pass
`--profile <name>`. A read-only posture never registers `edit` or `shell` in the
first place, so there is nothing to deny.

## Sessions

Every dispatch returns a `session_id`, and the subagent reports it. Passing it
back through `/smith:resume` gives Smith its prior conversation, which is both
cheaper and more faithful than restating the problem — Smith's own context, not
your reconstruction of it.

## Layout

```
plugins/smith/
  .claude-plugin/plugin.json
  agents/smith-delegate.md          thin forwarder; no repository access
  commands/{task,resume,sessions}.md
  skills/smith-cli-runtime/SKILL.md  the CLI contract and result envelope
```

The subagent deliberately holds only `Bash`. It forwards and reports; it does
not read files, plan, or decide what Smith should conclude. Investigating before
dispatching would pay for the same reads twice and risks handing Smith a stale
picture of a repository it is about to read itself.
