# Smith TUI Design

The visual and interaction contract for the `smith` terminal client. Task 1.4
requires this document to be approved before TUI implementation continues; it
defines what the code in `crates/smith-tui` is allowed to assume.

Smith is an **operational coding surface**, not a dashboard. The transcript is
the product. Everything else — status, approvals, background work — earns its
space by being unavoidable, and gives the space back when it is not.

The text hierarchy follows the observable grammar of OpenAI Codex CLI 0.145.0,
captured in a real cmux terminal and checked against the matching open-source
tag. Smith keeps its own name and runtime concepts, but uses the same quiet
bullet-led transcript, hanging indents, semantic color roles, command labels,
status-card structure, and compact footer.

## 1. Principles

1. **The transcript owns the screen.** There is no permanent top header. The
   composer and one compact status footer are the only persistent chrome.
2. **State is legible without color.** Color is a second channel, never the
   only one. Every state that matters is also carried by a glyph or a word,
   because terminals get themed, piped, and screenshotted in monochrome.
3. **Uncertainty is shown, not smoothed.** An estimated token count reads
   `~12.4k`, an unknown cost reads `cost ?`. Smith never renders a guess with
   the same weight as a provider-reported fact.
4. **Nothing moves that the user did not cause.** Streaming text appends;
   layout does not reflow, jump, or animate underneath a reader.
5. **The keyboard is the only required input.** A mouse may work; it is never
   necessary.

## 2. Layout

```text
┌────────────────────────────────────────────────────────────────────────┐
│                                                                        │
│ › explain the retry policy                                             │
│                                                                        │
│ • The retry policy classifies provider failures into three groups…     │  transcript
│                                                                        │  (flex, scrolls)
│ • Read(src/retry.rs) · ok                                              │
│ • Shell(.) · ok                                                        │
│                                                                        │
├────────────────────────────────────────────────────────────────────────┤
│                                                                        │
│ › fix the flaky test in retry.rs▏                                      │  composer (1–8 rows + inset)
│                                                                        │
├────────────────────────────────────────────────────────────────────────┤
│   openai/gpt-5.3 · ~/work/api                                          │  footer (1 row)
└────────────────────────────────────────────────────────────────────────┘
```

Regions, top to bottom:

| Region | Height | Rule |
| --- | --- | --- |
| Transcript | flex | Minimum 3 rows; below that Smith renders a size warning only. |
| Composer | 3–10 | One-row vertical inset around 1–8 rows of input. |
| Footer | 1 | Model and path while idle; controls only while an interactive overlay is active. |

Typing `/` opens command completion as a compact bottom-pane list directly
beneath the composer, following the same selected-row grammar as Codex. Modal
overlays are reserved for consequential interaction: approval, provider-spend
confirmation, undo/revert confirmation, and exit confirmation. They are
centered, max 72 columns wide and 60% of height, and drawn over the transcript.
Read-only command information never opens a modal; it appends to the
transcript. Only one interactive surface exists at a time; a second request
replaces the first rather than stacking.

### Narrow and short terminals

- Below 60 columns the footer keeps the provider/model and drops path, then
  secondary state.
- Below 10 rows or 40 columns Smith renders only `terminal too small (need
  40×10)` — a half-rendered coding surface is worse than an honest refusal.

### Setup before the coding surface

A genuinely empty interactive launch opens `Smith setup` before a runtime,
session, tool registry, approval channel, journal, or provider transport is
constructed. Partial or malformed configuration is an error, not an excuse to
replace user state. Non-interactive and machine-output launches never open
setup.

Setup is a keyboard-first sequence of one choice or one field per screen:
action, provider, authentication, model, explicit limits, response
compatibility, default selection, and review. The review names every
non-secret value, the exact user-config destination, and the pending local
preflight. API-key text is rendered only as masking glyphs. `Shift+Tab` goes
back and `Esc` cancels without writes; a denied credential service returns to
authentication with the environment-reference option still available.

Publication is transactional. Smith enrolls the reviewed credential, writes a
same-directory atomic user-config edit, then exercises the shared runtime
factory's derivation-only preflight. Failure restores the exact prior config
bytes and prior credential. Preflight sends no provider request and constructs
no session state. Only a successful automatic first-run continues into the
ordinary coding surface; explicit `smith setup` commands exit after success.

## 3. Type and glyphs

Terminal typography is the user's, not ours. Smith commits only to a fixed
column grid, and assumes nothing about font family, ligatures, or size.

**Width safety.** Every glyph Smith renders is either ASCII or a single-width
codepoint verified against `unicode-width`. No emoji: they are double-width on
some terminals and single on others, which corrupts alignment. Right-aligned
columns are computed from display width, never `str::len`.

Role markers, one per transcript block:

| Marker | Meaning |
| --- | --- |
| `›` | User message |
| `•` | Assistant message, reasoning, tool call, or informational notice |
| `└` | Detail or output belonging to the row above |
| `│` | Wrapped command continuation |
| `■` | Error or interrupted operation |
| `⚠` | Warning or degraded state |
| `?` | Approval request |

The first line of a block owns its marker. Continuation lines use a two-column
hanging indent, so wrapped content never looks like a new event. The `•` itself
is normally dim; a completed tool colors only that bullet green or red and
keeps the action verb bold. User text remains the terminal foreground rather
than turning the whole prompt cyan.

Reasoning is progress, not a second assistant reply. From `turn_started` until
the turn ends, the transcript appends one animated, dim
`Working… · 12s` row rather than rendering the provider's raw reasoning text.
Its duration comes from a local monotonic clock. At the turn boundary the
working row disappears and a quiet notice freezes the outcome and elapsed
time, such as `turn · completed in 12s` or `turn · interrupted after 12s`.
Canonical session history and the journal still retain reasoning for model
continuity, replay, and diagnostics; historical durations are not invented
when replay has no local timing record.

## 4. Color

Sixteen-color ANSI only, by name — never by RGB or 256-color index. Smith
cannot know the user's palette, so it must ask for "red" and let the terminal
decide what red is. This is also what makes light and dark themes work without
Smith detecting either.

| Token | ANSI | Used for |
| --- | --- | --- |
| `default` | terminal default | Assistant text, transcript body |
| `dim` | dim modifier | Timestamps, hints, tool argument detail |
| `accent` | cyan, bold | Active selection, inline code, focused control |
| `command` | magenta | Local slash-command labels such as `/status` |
| `success` | green | Successful tool bullet, cache hit, confirmed state |
| `warning` | yellow | Estimated values, degraded capability, unread inbox |
| `danger` | red | Errors, denials, destructive approval targets |
| `reasoning` | default, dim, italic | Compact in-flight model progress |
| `link` | cyan, underline | Rendered links |
| `status-model` | cyan | Active model in the footer |
| `status-path` | green | Working directory in the footer |

Rules:

- **Never color-only.** `success` always accompanies the word `ok`; `danger`
  always accompanies `!` or `failed`.
- **No fixed background fills.** Codex derives a subtle composer/user surface
  from the detected terminal background. Smith leaves that surface at the
  terminal default until it has equivalent bounded palette detection; a fixed
  dark fill would break light terminals.
- **Bold marks structure** — action verbs, headings, strong Markdown, modal
  titles, and the active selection. It never colors an entire paragraph.
- Assistant Markdown follows the reference hierarchy: H1 bold+underline, H2
  bold, H3 bold+italic, lower headings italic, inline code cyan, emphasis
  italic, and strong text bold.
- A `--no-color` flag and `NO_COLOR` env var drop hue while retaining the
  typographic structure carried by dim, bold, italic, and underline. The glyph
  channel from §3 makes the result fully usable.

## 5. Composer, commands, and keyboard

The composer is the only persistent focus target. Transcript scrolling is
global and never requires a focus mode. There is no hidden modal state: when a
modal or temporary view is open, it owns input and names its controls in the
hint row.

| Key | Action |
| --- | --- |
| `Enter` | Send the composer |
| `Shift+Enter` / `Alt+Enter` | Newline in the composer |
| `Esc` | Interrupt the running turn; if idle, clear the composer |
| `Ctrl+C` | Quit (confirm if work is live); twice within 1s forces quit |
| `Ctrl+P` | Open command completion using the shared command registry |
| `Tab` / `Shift+Tab` | Complete or move command selection; never change regions |
| `PageUp` / `PageDown` / `Home` / `End` | Scroll transcript or jump to either edge |
| `Ctrl+L` | Jump to newest and re-enable follow |
| `y` / `n` / `a` | Approval: allow once / deny / allow for session |

Approval keys are deliberately *not* `Enter`-defaulted. An approval modal has
no default action, because a stray `Enter` from the composer must never grant a
shell command.

Typing `/` at the start of a composer draft opens a filtered completion menu.
Each result has a command name, one-line description, and argument hint.
`Tab` completes the selected command without executing it; `Shift+Tab` moves
selection backward; `Enter` executes; and `Esc` dismisses the menu while
preserving the draft. `Ctrl+P` opens the same registry and parser. `//` sends a
literal leading slash to the provider.

The initial command set is deliberately bounded:

| Command | Result |
| --- | --- |
| `/help` | List every implemented command, grouped by primary and advanced use. |
| `/status` | Show resolved runtime, context window, session, permission, Git, child, and attribution state locally. |
| `/context` | Visualize the latest model-facing context plan, free input space, reserves, and compaction state locally. |
| `/new` | Save the current session and create a fresh identity. |
| `/resume [ID]` | With no ID, choose a project session locally; otherwise validate and resume `ID`. |
| `/profile [NAME]` | With no name, choose a configured profile; apply it while clearing narrower overrides. |
| `/provider [NAME]` | With no name, choose a configured provider; cascade to its model choices when needed. |
| `/model [PROVIDER/MODEL]` | With no pair, choose from provider-qualified models; apply provider and model atomically. |
| `/agent [ID]` | List children or show one child's latest state/result. |
| `/diff [SCOPE]` | Inspect all, last-turn, staged, unstaged, untracked, file, or hunk changes. |
| `/review [SCOPE]` | Confirm and launch a provider-backed read-only review. |
| `/undo` | Preview the last fully attributable Smith turn and require explicit confirmation. |
| `/revert [FILE]` | Select one current file or hunk, preview it, and require explicit confirmation. |
| `/quit` | Exit under the active-work policy. |

### Inline local results

Read-only local commands append an attributed transcript block. The command
itself is a magenta transcript line rather than a generic result marker:

```text
/status
╭──────────────────────────────╮
│ >_ Smith                     │
│                              │
│ session:   session-…         │
│ provider:  openai            │
│ model:     gpt-5.3           │
│ context window:  96% input left (2.7k used / 68.9k budget) │
│ model window:    200k total · 131k reserved                │
╰──────────────────────────────╯
```

`/status` uses a dim, content-sized border; labels are dim and values use the
normal foreground. Its context section follows Codex's percent-left grammar,
but names Smith's enforced **input budget** separately from the model's total
window and reserved output/reasoning space. It shows the latest request plan
and its exact/estimated provenance; cumulative provider input is separately
labelled as session usage because it is not the active context size. Before the
first plan it says `not planned yet` instead of showing zero. Other local
results stay unboxed: help section names are bold, command names and inline
code are cyan, diff additions are green, removals are red, and hunk headers
are cyan. Empty results use a dim `•`; errors use a red `■`. Unavailable and
error results state their condition in words, not color alone. Content wraps
at terminal width and is bounded before it enters the transcript, with an
explicit truncation line rather than silent clipping.

`/context` is the focused, Claude-style context view. It stays unboxed and
starts with the active model plus latest-plan input use and percent left. A
fixed 5×10 map uses both distinct single-width glyphs and named ANSI colors to
show system instructions, tool schemas, history, summaries, current user
input, free input space, and reserved output/reasoning capacity. The
accompanying legend repeats each glyph, count, and percentage so color is never
the only channel. Below it, Smith names exact versus estimated counting,
segment count, provider-reported cumulative session input, cache reads, and
whether compaction is waiting or has applied a summary. The map represents the
latest request Smith actually planned; before the first turn it renders
capacity and reserve with `usage unavailable until the first turn` instead of
inventing segment usage. It does not retain or reveal raw context content.

`/help`, `/status`, `/context`, `/agent`, and every `/diff` scope use this
primitive.
Consecutive results append in order and remain visible while the composer stays
active. They are TUI-local display records: they are never represented as user
or assistant messages, never sent to the provider, and are intentionally
dropped when the transcript is rebuilt from canonical history on resume.

Commands that require an idle runtime fail locally while a turn is active;
they are not queued and are never sent to the provider. Runtime-selection
commands restore the normal screen, shut down and save the current runtime,
then rebuild through the same Smith factory and explicitly resume when
appropriate. Configuration, credential, or compatibility failures therefore
appear outside raw terminal mode. A provider/model change is called out in the
transcript, clears cache evidence, and labels prior aggregate context as
estimated.

Omitted selector arguments open the same reusable resource-picker grammar:
type to filter bounded local metadata, `Up`/`Down` to move, `Enter` to choose,
and `Esc` to restore the untouched composer draft. Active choices are labelled
`current`; incompatible or incomplete entries remain visible with an
`unavailable` reason but cannot be selected. Empty model and provider views
point to `smith setup`; an empty session view says there is nothing to resume.
Filtering and selection resolve no credential, read no model history, make no
network request, and spend no provider tokens.

For configured OpenRouter and Z.AI Coding Plan endpoints, model rows may come
from Smith's frozen Models.dev snapshot as well as explicit TOML. The exact
endpoint binding is a trust boundary; a matching provider name alone is not.
Catalog rows retain the configured provider alias, show the catalog display
name and provider-qualified ID, summarize limits and coding capabilities, and
label the source revision and age as `advertised`. That word is deliberate:
the row does not claim that the current account, plan, region, or credential
is entitled to use the model.

Deprecated catalog entries are absent. Entries lacking text output, tool
calling, complete valid limits, or input space after effective reserves remain
filterable but dimmed with a bounded `unavailable` reason. Provider row counts
include only selectable models. Catalog display name, ID, provider, and
capability detail all participate in local filtering, including catalogs with
hundreds of entries; rendering still emits only the rows the viewport can
show.

The host prepares one immutable snapshot before constructing the picker and
runtime. It reads a validated last-good cache or the bundled seed, then may
refresh the exact public Models.dev URL in the background without provider
credentials. Picker interaction never waits for that work. Atomic refresh
publication affects only a later host rebuild, while a picker-driven
provider/model rebuild retains the snapshot that made the selected row
available.

The pre-host `smith --resume` picker uses the same rows before constructing a
host. Saved-session metadata includes full identity, recency, turn count,
provider/model, and a bounded preview. Older compatible snapshots remain
selectable with unknown fields labelled `?`; newer incompatible schemas remain
visible but disabled. Bare `--resume` is interactive-only, while an explicit
session ID works unchanged in terminal and headless modes.

### Change views and confirmation

`/diff` is read-only and appends directly to the transcript. Empty, non-Git,
binary, oversized, and conflicted states are named explicitly. `/review` names
its selected scope and provider spend in a confirmation modal before dispatch;
the reviewer receives read-only workspace authority and findings return to the
transcript.

`/undo` and `/revert` never have a default action. Their confirmation modals show
the exact reverse patch, origin (`Smith`, `user`, or `unknown`), and any paths
that cannot be proven safe. `y` confirms only after every post-image check
passes; `n` or `Esc` cancels. A stale path, ambiguous tool delta, or partial
validation failure leaves the entire workspace unchanged.

An unchanged untracked file selected for removal is first moved to bounded
session recovery storage. Smith does not use broad `git reset`, `git checkout`,
or a first-release `revert all` action.

**Scroll follow.** The transcript follows new output until the user scrolls up,
then stops and shows `▼ following paused` in the hint row. `Ctrl+L`, `End`, or
sending a message resumes it. Streaming never yanks the viewport away from
someone reading.

## 6. Streaming and motion

- Text deltas append to the last assistant block. The block is re-wrapped only
  from the last hard newline, so earlier lines never reflow.
- Render is coalesced at **30 fps max**, driven by a redraw flag rather than
  per-delta. A fast provider stream must not spend the frame budget on
  redundant frames.
- Runtime event sequence gaps render as an error block that names the missing
  range and points to the persisted journal as canonical. A lagging live
  subscriber must never make dropped output look like a complete transcript.
- The only animation is the single-cell spinner on the transcript's working
  row while a turn is active: `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` at 100 ms.
- **Reduced motion.** With `NO_MOTION=1`, `--no-motion`, or `TERM=dumb`, the
  spinner becomes a static `●` and the elapsed timer updates once per second.
  Nothing else in Smith animates, so this is the whole contract.

## 7. Status honesty

The footer and `/status` carry the provenance rules from `usage-accounting`
directly into glyphs. The footer's `ctx` value comes from the latest enforced
plan, while provider-reported cumulative input appears only in `/status`:

| Rendering | Meaning |
| --- | --- |
| `82% ctx` | Exact latest plan, 82% of its input budget remains |
| `~82% ctx` | Estimated latest plan, 82% remains |
| `? ctx` | Unknown — never rendered as `0` |
| `$0.031` | Exact, from a versioned price reference |
| `~$0.031` | Estimated |
| `cost ?` | No price reference for this endpoint |
| `⚡8.0k` | Cache read observed this turn |
| `⚡?` | Provider exposes no cache evidence |

Switching provider or model renders a one-line transcript notice —
`provider changed · openai → anthropic · prior cache not transferable` — and
the context segment falls back to `~` until the new provider reports usage.

### Tool argument visibility

Canonical runtime events do not expose tool argument values by default. The
interactive host resolves the matching canonical in-process call by ID and
passes only a typed, tool-specific projection into the TUI. Built-in rows show
an allowlisted local target plus bounded numeric or boolean qualifiers:
`Read(src/retry.rs) · ok` or `List(. · recursive) · running`. Commands, search
patterns, edit bodies, arbitrary values, and tool results stay hidden. Unknown
tools or unresolved calls show a bounded `tool(keys · values protected)`
fallback instead of guessing which value is safe.

The projection never changes the runtime event or journal and never enables
raw event arguments. Resumed transcripts derive the same projection from
canonical history, so live and replayed rows disclose the same reviewed
metadata.

The approval modal is the deliberate exception: it receives the exact bounded
invocation through the separate approval channel because the user cannot make
an informed safety decision from key names alone. Edit calls render the
existing bounded line diff; other calls render their bounded arguments.

## 8. Background work

Monitor notifications and child-agent progress reach the TUI immediately as
concise attributed transcript notices: `• source · summary`. They never splice
into a streaming assistant block and never steal composer focus. Terminal
events (a monitor stopped, a child finished) are never coalesced away.

The runtime's bounded safe-boundary inbox remains an internal delivery
mechanism for child results sent to the parent model. It is not a visible or
focusable TUI region. `/agent` provides child list, status, and result detail
on demand.

## 9. Accessibility

- **Screen readers.** Every modal announces its title as the first rendered
  text. Transcript blocks are separated by a blank line so a screen reader's
  paragraph navigation matches Smith's block structure.
- **Contrast.** Because Smith uses named ANSI colors, contrast is the terminal
  theme's responsibility. Smith's obligation is to never encode meaning in
  color alone (§4) and never rely on dim text for anything a user must act on.
- **No time-limited input.** No prompt, including approvals, expires on a timer
  in the interactive TUI. Headless `-p` fails closed instead of waiting, which
  is a different surface with a different contract.
- **Resize.** Every layout is recomputed from scratch on resize; nothing caches
  a wrapped line across a width change.

## 10. What this does not cover

Deferred to a later revision, and therefore not to be invented in code:

- Custom extension-drawn widgets beyond a declarative status item.
- Word-level or intra-line diff highlighting.
- Split panes, multiple simultaneous sessions on screen, and mouse selection.
- Broad reset/revert-all actions, staged commit/push controls, and automatic
  recovery for arbitrary shell side effects.
- Non-Git change recovery and queued commands during an active turn.
- Themes. There is one look; the terminal supplies the palette.
