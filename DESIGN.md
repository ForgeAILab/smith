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
5. **The keyboard is the only required input.** Smith uses the mouse only for
   optional transcript wheel scrolling; every other interaction remains
   available from the keyboard.

## 2. Layout

```text
┌────────────────────────────────────────────────────────────────────────┐
│                                                                        │
│ › explain the retry policy                                             │
│                                                                        │
│ • The retry policy classifies provider failures into three groups…     │  transcript
│                                                                        │  (flex, scrolls)
│ • Read(src/retry.rs · offset 1 · limit 200) · ok                       │
│ • Shell(cargo test -p smith-tui · cwd .) · ok                          │
│ • turn · completed in 12s                                              │
│                                                                        │
├────────────────────────────────────────────────────────────────────────┤
│   Todo                                                                 │
│   [x] Inspect retry policy                                             │
│   [>] Fix the flaky test                                               │
│   [ ] Run focused tests                                                │  anchored pane (0–9 rows)
│   Pending for this turn                                                │
│   › also cover the cancellation race                               │
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
| Anchored pane | 0–9 | A compact picker while one is open; otherwise bounded process-local pending input followed by the latest public plan. Hidden when neither exists. |
| Composer | 3–10 | One-row vertical inset around 1–8 rows of input. |
| Footer | 1–2 | Identity uses one row; resource pickers and prompts may add controls on the second. Slash completion does not. |

Typing `/` opens command completion as a compact bottom-pane list directly
above the fixed composer, following the same selected-row grammar as Codex.
Local resource choices opened by `/model`, `/provider`, `/profile`, `/resume`,
or `@` reuse that placement and show at most five matching rows; moving the
selection scrolls the bounded window instead of expanding or covering the
transcript. Any compact picker temporarily replaces the todo pane, but only a
resource picker adds a footer control row. Slash completion relies on the
established keyboard contract and keeps the one-row identity footer. Closing a
picker restores the unchanged todo projection. Modal overlays are reserved for
consequential interaction:
approval, provider-spend confirmation, agent-originated questionnaires,
undo/revert confirmation, and exit confirmation. They are centered, max 72
columns wide and 60% of height, and drawn over the transcript. Read-only
command information never opens a modal; it appends to the transcript. Only
one interactive surface is visible at a time. Runtime-originated approvals and
questionnaires wait in a stable FIFO prompt queue; a new prompt never
supersedes, implicitly denies, or drops an older one. The footer names the
visible prompt and remaining queue count.

### Narrow and short terminals

- Below 60 columns the footer keeps the provider/model and drops path, then
  secondary state.
- Consequential overlays below 60 columns use the full safe width, put the
  title and exact target first, wrap permissions and material arguments, and
  keep decision controls on their own final lines. Bounded detail remains
  scrollable rather than clipped.
- Below 10 rows or 40 columns Smith renders only `terminal too small (need
  40×10)` — a half-rendered coding surface is worse than an honest refusal.
  An open prompt remains queued and unanswered while the terminal is too small.

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
| `?` | Approval or questionnaire awaiting an explicit answer |

The first line of a block owns its marker. Continuation lines use a two-column
hanging indent, so wrapped content never looks like a new event. The `•` itself
is normally dim; a completed tool colors only that bullet green or red and
keeps the action verb bold. User text remains the terminal foreground rather
than turning the whole prompt cyan.

Reasoning is progress, not a second assistant reply. From `turn_started` until
the turn ends, the transcript appends one animated, dim
`Working… · 12s` row rather than rendering the provider's raw reasoning text.
Its duration comes from a local monotonic clock. At the turn boundary the
working row disappears and every successful terminal appends one quiet
`turn · completed in …` notice. Its duration comes from the canonical
millisecond interval between the turn's start and completion envelopes:
sub-second turns use milliseconds (zero renders `<1ms`), longer turns use the
compact second/minute/hour grammar, and an absent or backward interval omits
the duration instead of substituting local reducer time. Absence of visible
assistant text never adds a `reasoning only` diagnosis. Interrupted, limited,
needs-input, and failed turns retain their attributed notice with live elapsed
time when available. Canonical session history and the journal retain the
start/completion events, timestamps, and reasoning needed for model
continuity, replay, timelines, and diagnostics.

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
global and never requires a focus mode. There is no hidden modal state: a
modal or resource picker owns input and names its controls in the hint row.
Slash completion is the deliberate quiet exception: its selected-row grammar
and the keyboard contract below are sufficient, so it adds no control strip.

Smith enables terminal mouse reporting only to receive wheel events. The TUI
uses wheel-up and wheel-down for transcript scrolling and ignores button and
motion events; there is no click-to-position or application-owned clipboard
behavior. Because terminal mouse reporting is global, native drag selection
may require the terminal's selection modifier. Bracketed paste remains enabled
independently of pointer handling.

| Key | Action |
| --- | --- |
| `Enter` | Send while idle; steer an eligible serving provider turn while busy |
| `Shift+Enter` / `Alt+Enter` | Newline in the composer |
| `Esc` | Interrupt the running turn; with uncommitted steers, resubmit only eventual discards after cancellation; if idle, clear the composer |
| `Ctrl+C` | Add a non-blank composer draft to bounded local history and clear it; replace the identity footer with `press Ctrl+C again to exit` for the 1s double-press window; a second press exits from any state |
| `Ctrl+P` | Open command completion using the shared command registry |
| `Ctrl+R` | Open incremental reverse search over bounded process-local composer history |
| `Tab` | Queue a non-empty ordinary prompt while busy; cycle `profile_order` only when empty and idle; otherwise complete or move the active overlay selection |
| `Shift+Tab` | Move the active completion/questionnaire selection backward |
| `Alt+Up` | Restore the newest explicitly queued future turn for editing; never edit a runtime-accepted steer |
| `Left` / `Right` / `Backspace` / `Delete` | Edit ordinary text by Unicode character; cross or remove a registered paste/image placeholder as one unit |
| `PageUp` / `PageDown` / `Home` / `End` | Scroll transcript or jump to either edge |
| `Mouse wheel` | Scroll transcript without changing composer history |
| `Ctrl+L` | Jump to newest and re-enable follow |
| `?` (empty composer) | Show the same local guide as `/help`; never contact the provider |
| `Up` / `Down` (composer history) | Browse accepted or `Ctrl+C`-stashed input and return to the exact pre-navigation draft |
| `y` / `n` / `a` | Approval: allow once / deny / allow for session |
| `Up` / `Down` or `1`–`9` | Questionnaire: move to or stage a labelled choice |
| `Space` | Questionnaire: select the highlighted choice; never submit |
| `Tab` / `Shift+Tab` | Questionnaire: move between answer and explicit actions |

Large text pastes and real clipboard images are registered out of band and
appear in editable or pending input as accented `[Pasted text #N +L lines]`
and `[Image #N W×H]` labels. Each registered label has only two horizontal
cursor stops and one adjacent deletion removes it completely. Typed paths,
typed label lookalikes, and stale labels with no registered material remain
ordinary character-addressable text. When input commits, a pasted-text label
is replaced by its exact stored text in the user transcript; an image label
remains visible while its registered image is sent as a separate content part.
Composer history and uncommitted pending previews keep the compact labels.

Approval keys are deliberately *not* `Enter`-defaulted. An approval modal has
no default action, because a stray `Enter` from the composer must never grant a
shell command. A questionnaire also opens with no implicit answer or action.
`Enter` in its choice list only stages the highlighted choice; submission
requires moving to the explicit `Submit` action. `Decline` returns a typed
decline, while `Esc` cancels the interaction under the active turn's
cancellation policy. Free-form input reuses composer editing inside the
overlay and is not sent as a new user turn.

Typing `/` at the start of a composer draft opens a filtered completion menu.
Each result has a command name, one-line description, and argument hint.
`Tab` completes the selected command without executing it; `Shift+Tab` moves
selection backward; `Enter` executes; and `Esc` dismisses the menu while
preserving the draft. `Ctrl+P` opens the same registry and parser. `//` sends a
literal leading slash to the provider.

Outside an overlay, a non-empty ordinary prompt has two distinct busy-turn
intents. `Enter` prepares it once and asks Agent Runtime to steer the tracked
serving `TurnId`; it remains a labelled process-local preview until the runtime
emits a matching committed or discarded disposition. Only the committed event
appends the canonical user transcript row. `Tab` instead stores an editable,
bounded future whole turn entirely in Smith. Slash commands, `!` shell
shortcuts, child-agent forms, approvals, questionnaires, confirmations, and
reconfiguration never enter this queue implicitly.

Pending input is divided into accepted-but-uncommitted steers,
runtime-rejected steer follow-ups, and explicit future turns. Each entry keeps
its exact display text, already-expanded paste material, image parts, and
canonical workspace-relative file identities without reading files or
contacting a provider. A dispatched queued file observes dequeue-time workspace
content. The anchored pane labels each category in text, shows at most three
preview lines plus an overflow count, and shares its remaining bounded height
with the public todo projection. A compact picker or modal still owns the area
and its keys first. This state is process-local, counts as live work for exit
and reconfiguration, and is not presented as journal durability.

At a successful terminal boundary Smith submits at most one future turn:
runtime-rejected steers first (a rejected batch merged in FIFO order), then the
oldest explicit queue entry. That real-user admission occurs before automatic
goal continuation is re-enabled. Cancelled, failed, limited, or needs-input
boundaries restore uncommitted and queued material for review without provider
spend, except the explicit interrupt-for-steer path described above. A stale
expected turn may be retried once against the runtime-reported eligible turn;
no-active while idle becomes one ordinary send, and other failures restore the
same prepared material rather than fabricating a queued runtime turn.

The composer keeps at most 100 exact non-blank history entries from accepted
provider prompts, local commands/actions, confirmation flows, and drafts
cleared by the first `Ctrl+C`. Adjacent exact duplicates collapse into one
entry. Rejected input remains in place and does not enter history. This state
is process-local UI memory only: it is not canonical conversation history, is
not persisted in checkpoints, and is never sent to a provider unless the user
later submits recalled text.

With no overlay open, `Up` begins newest-first history navigation from an empty
or non-empty composer while preserving the current text as a scratch draft.
`Down` past the newest entry restores that draft exactly; editing recalled
text exits navigation without recording the edit. `Ctrl+R` opens a compact
anchored reverse-search surface over the same history. Typing performs a
case-insensitive substring search, repeated `Ctrl+R` cycles older matches with
bounded wrapping, `Enter` restores the selected text without submitting it,
and `Esc` restores the original draft. Existing pickers, approvals,
questionnaires, and confirmations retain keyboard ownership. A first
`Ctrl+C` during reverse search restores, stashes, and clears the original
draft before applying the existing double-press exit contract.

### Agent-first composer actions

The empty idle footer identifies the selected main agent profile beside provider/model,
project/branch, and honest context confidence. It does not reserve a persistent
shortcut strip: `?` from an empty composer and `/help` both render the bounded
local command/composer guide without provider spend or canonical history. This
identity disappears while work is active; it is not a permanent header. At 44
columns, low-priority path detail disappears before mode, activity, model,
approval, or context provenance. After a first `Ctrl+C`, the entire identity or
activity footer temporarily becomes the warning-toned text `press Ctrl+C again
to exit`. A second press within one second exits; expiry or any other key
restores the current status without leaving a transcript record.

Named `[profiles]` are the shared agent presets. Each profile selects a bounded
`build`, `plan`, or `review` posture and may be enabled for `main`, `child`, or
both placements. `plan` and `review` are read-only; profile instructions are
additive prompt guidance, never permission or approval. `Tab` cycles the
validated main-enabled `profile_order` only when the composer is empty, the
runtime is idle, and no overlay is open. `/profile` uses the same atomic
safe-boundary rebuild and clears narrower provider/model overrides.

Typing `@` at a token boundary opens one bounded picker with explicit `file`
and `agent` labels. Picker rows show the plain file or agent identity without a
leading `@`; choosing one inserts the visible `@identity` mention into the
composer. Files are canonical workspace-relative entries that honor ignore
policy. On submit, Smith performs an exact prepared `read` through the runtime
executor and contributes bounded content (or an artifact reference) with
`prepared_read` provenance. Unresolved, ambiguous, oversized, binary, or
outside-workspace references fail locally and preserve the draft before any
provider request. `@@` escapes one literal `@`; typed `@file:name` and
`@agent:name` disambiguate collisions.

Every child-enabled `@profile <task>` entry is an explicit depth-one,
read-only child. The profile may resolve another declared provider/model
through normal credential, catalog, context, and runtime preflight. Its
confirmation shows profile configuration, provider/model, limits, workspace
posture, expected result, and provider spend. Effective authority is the
intersection of parent authority, the host child ceiling, and profile posture;
children cannot delegate again or widen root policy.

Retained children appear as separate `@child-id` entries. Selecting one keeps
the stable child/session identity and confirms a new follow-up turn with its
cumulative limits and prior history. It is never interpreted as a preset or a
spawn. Interrupted children instead expose `/agent resume <child-id>`, whose
no-default confirmation names exact checkpoint continuation and does not
consume another task slot.

A first non-whitespace `!` performs one direct local shell action. It uses the
same schema preparation, broad shell authority, approval, scheduler, deadline,
cancellation, checkpoint, bounded output, and artifact-offload path as a
model-requested `shell` call, then renders the committed result locally. It
does not send a provider request. `!!` sends a normal prompt beginning with one
literal `!`.

During work, the latest public todo projection shares a bounded pane anchored
immediately above the composer with pending input. It remains there through
the turn terminal and clears when the next turn starts. Sensitive plans expose
no item text. A compact picker temporarily replaces this pane without mutating
either projection, which returns when the picker closes. The transcript's
progress row stays
quiet — `Working… · 12s` — while `/details` may explicitly add bounded
redaction-safe tool lifecycle lines. No aggregate `work` row is committed at
the terminal. Every terminal boundary still reconciles pending/in-progress
todo items to `cancelled (turn_ended_unfinished)` rather than inventing
completion.

The initial command set is deliberately bounded:

| Command | Result |
| --- | --- |
| `/help` | List every implemented command and composer shortcut locally, grouped by primary and advanced use. |
| `/status` | Show resolved runtime, context window, session, permission, Git, child, and attribution state locally. |
| `/goal [OBJECTIVE\|edit …\|budget N\|pause\|resume\|clear]` | Inspect or control one persistent multi-turn session goal without sending the command to the provider. |
| `/context` | Visualize the latest model-facing context plan, free input space, reserves, and compaction state locally. |
| `/think [on\|off\|default]` | Inspect or change thinking for the next complete turn when the provider/model exposes an exact toggle. |
| `/effort [LEVEL\|default]` | Inspect or change reasoning effort using only levels advertised for the active provider/model. |
| `/details` | Toggle bounded redaction-safe live tool detail beneath the working row. |
| `/timeline` | Show ordered root turn, child, terminal plan/gate, and recovery evidence locally. |
| `/new` | Save the current session and create a fresh identity. |
| `/resume [ID]` | With no ID, choose a project session locally; otherwise validate and resume `ID`. |
| `/profile [NAME]` | With no name, choose a configured profile; apply it while clearing narrower overrides. |
| `/provider [NAME]` | With no name, choose a configured provider; cascade to its model choices when needed. |
| `/model [PROVIDER/MODEL]` | With no pair, choose from provider-qualified models; apply provider and model atomically. |
| `/agent [ID\|previous\|next\|parent\|resume ID]` | List/inspect children or explicitly resume one safe interrupted checkpoint while the root composer retains focus. |
| `/diff [SCOPE]` | Inspect all, last-turn, staged, unstaged, untracked, file, or hunk changes. |
| `/review [SCOPE]` | Confirm and launch a provider-backed read-only review. |
| `/undo` | Preview the last fully attributable Smith turn and require explicit confirmation. |
| `/redo` | Preview and explicitly confirm the newest exact undo/selective-revert forward patch. |
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
the only channel. Its first two rows are always `system instructions` and
`tool schemas`, in that order. The system row aggregates canonical system,
developer, and ability-instruction totals for display only; telemetry keeps the
original kinds. After planning, both rows remain visible with an honest zero
when absent, although a zero category occupies no grid cell. Before the first
plan their rows read `? (not counted yet)` while the grid renders only known
capacity and reserve. Unknown never becomes zero, and Smith does not synthesize
a request merely to size it because tool activation depends on the submitted
turn. Below the map, Smith names exact versus estimated counting, segment
count, provider-reported cumulative session input, cache reads, and whether
compaction is waiting or has applied a summary. The map represents the latest
request Smith actually planned and does not retain or reveal raw context
content.

`/think` and `/effort` share the local command and resource-picker grammar.
They require an idle root session, spend no provider tokens themselves, and
affect the next whole turn rather than a running attempt or tool continuation.
The thinking picker offers only `on`, `off`, and provider default states that
the exact provider/model contract supports; mandatory thinking removes `off`.
The effort picker lists only advertised levels plus provider default. A model
that reasons but exposes no controls receives a written fixed/unavailable
result instead of a guessed selector. `/status` and `/context` name the
effective state, effort when applicable, and provider/profile/session
provenance without rendering raw reasoning.

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

Reasoning presence and reasoning control are different facts. On an unknown
endpoint a catalog boolean that says a model reasons means only fixed
reasoning. Adjustable controls require source-explainable switch behavior,
ordered effort choices, optional token-budget support, a provider wire
dialect, defaults, and provenance from an exact trusted provider/model
binding, explicit trusted configuration, or an exact endpoint that normalizes
controls itself — the OpenAI `reasoning_effort` ladder, the OpenRouter
unified reasoning API, and the Z.AI Coding Plan thinking switch, which apply
to every catalog-advertised reasoning model they serve. On those endpoints
the frozen Models.dev snapshot refines the default ladder with each model's
advertised control shape; a snapshot annotation is advertised metadata, not
an entitlement claim, and it never creates controls on an endpoint whose
wire dialect is unknown. Smith snapshots the effective selection at turn acceptance and
uses it unchanged for retries and tool continuations. Compatible session
overrides resume and flow into newly created children; a provider/model switch
that cannot represent the override clears it with a local notice rather than
mapping to a nearest value.

For configured OpenAI, OpenRouter, and Z.AI Coding Plan endpoints, model rows
may come from Smith's frozen Models.dev snapshot as well as explicit TOML. The exact
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

### Prepared approvals and questionnaires

An approval is a view of one immutable prepared action, not a reconstruction
from raw tool arguments. Its title is the first rendered text. The body shows,
in this order:

```text
tool + exact canonical target
bounded material arguments or reviewed patch
typed permissions
broad-authority warning, when applicable
preparation fingerprint
deadline
```

`y`, `a`, and `n` answer exactly that fingerprint. Edited arguments are never
approved in place; they must be prepared and authorized again as a new action.
Parallel actions retain a deterministic order and queue count, and each gets
one explicit decision or terminal cancellation. A restored action is labelled
`restored pending approval` and keeps its original request identity.

Questionnaires are a separate interaction type and never use approval
responders. The overlay is a short wizard of one to three labelled questions,
with one question visible at a time. Each step provides bounded choices,
optional free-form input when declared, progress such as `question 2 of 3`,
and explicit `Submit`, `Decline`, and `Cancel` actions. Answers are staged until
the final submit and grant no permission or remembered authority. Sensitive
free-form drafts render as masks; after submission their exact values remain
available to the live turn and protected checkpoint but are registered for
literal removal from default snapshots and journals. A restored questionnaire
is labelled `restored pending question`, retains its request identity, starts
with no fabricated UI answer, and may be answered exactly once.

Prompt deadlines are displayed as absolute local time plus bounded remaining
duration. Expiry closes the prompt with a visible `timed out` outcome; it never
selects a default, grants authority, or fabricates an answer. Shutdown and
turn cancellation drain the queue by resolving every responder exactly once.
Direct agent questionnaires are root-session-only by default. A child that
needs input reports an attributed `needs input` result through its parent
instead of opening a competing overlay.

**Scroll follow.** The transcript follows new output until the user scrolls up,
then stops and shows `▼ following paused` in the hint row. `Ctrl+L`, `End`, or
sending a message resumes it. Streaming never yanks the viewport away from
someone reading.

## 6. Streaming and motion

- Provider text and reasoning deltas enter speculative buffers keyed by
  `(request, attempt)`, never the committed transcript directly. The active
  attempt renders with a textual `draft` marker as well as dim styling.
- An explicit attempt-commit event appends that attempt to the assistant block.
  An explicit discard removes its raw text and may append one bounded
  `retrying after failed attempt` diagnostic. Usage from the discarded attempt
  remains available to status and diagnostics.
- Within a speculative or committed block, text is re-wrapped only from the
  last hard newline, so earlier lines never reflow.
- Render is coalesced at **30 fps max**, driven by a redraw flag rather than
  per-delta. A fast provider stream must not spend the frame budget on
  redundant frames.
- Runtime event sequence gaps render as an error block that names the missing
  range and points to the persisted journal as canonical. A lagging live
  subscriber must never make dropped output look like a complete transcript.
- Journal replay feeds the same pure reducer as the live path. It reconstructs
  the same committed transcript, tool state, capability/todo status, and
  visible-output result; speculative output with no commit never becomes
  canonical merely because a process stopped. Exact pending approval and
  questionnaire overlays are restored from the protected checkpoint, not
  fabricated from the deliberately redacted journal. Process-exit recovery
  markers use the same metadata-only notice projection in live and replayed
  state.
- The only animation is the single-cell spinner on the transcript's working
  row while a turn is active: `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` at 100 ms.
- **Reduced motion.** With `NO_MOTION=1`, `--no-motion`, or `TERM=dumb`, the
  spinner becomes a static `●` and the elapsed timer updates once per second.
  Nothing else in Smith animates, so this is the whole contract.

### Headless projection

The non-interactive surface shares the same committed event semantics. Text
mode writes only the final assistant answer to stdout and sends concise
lifecycle/authority evidence to stderr. Ordinary prompts remain one turn; an
explicitly active goal follows attributed internal continuations until it
stops. JSON emits one schema-v3 result;
stream JSON emits ordered versioned runtime events through shutdown and that
result last. Both machine modes project attempt commits/discards, the frozen
activation epoch, public-or-counts-only todo state, artifact references,
recovery metadata, optional goal state/continuation count, prepared approval
evidence, and interaction-required state.
They never expose raw approval arguments, sensitive todo items, questionnaire
content, or artifact bodies. No-TTY approval/question paths terminate with
stable non-success results rather than reading stdin or waiting indefinitely.

## 7. Status honesty

### Project instruction context

A standard interactive or headless host examines exactly
`<canonical-project-root>/AGENTS.md` before runtime construction. Absence is
ordinary; a present file must be a regular non-symlinked UTF-8 file no larger
than 32 KiB or startup fails before provider, session, or terminal work. Smith
captures one immutable snapshot for the constructed runtime and every direct
child. It does not search parent/nested directories, expand include syntax,
watch the file, or mutate an active context after an edit.

The snapshot is a required developer-instruction fragment separate from
Smith's independently revisioned product policy, optional retrieval context,
and canonical user history. Its source and content-derived revision enter the
composition/context manifest. A newly constructed runtime sees a changed file
and gets a new exact prompt/cache identity; an active runtime keeps its old
snapshot. An explicit complete host system-prompt override remains a complete
replacement and receives no implicit project fragment. Project text may guide
work but never grants tools, permissions, approval, executable trust, or a
wider workspace.

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
clones its arguments, applies the same credential-shaped-key and exact
registered-secret redaction used by persistence, and passes only that redacted
clone into a typed tool-specific TUI projector. Built-in rows show bounded
ordinary operation inputs and targets: `Read(src/retry.rs · offset 4 · limit
20) · ok`, `Search("TurnCompleted" · crates · extension rs · limit 20) · ok`,
or `Shell(cargo test -p smith-tui · cwd . · timeout 30000ms) · running`.
Credential, authorization, API-key, token, password, private-key, bearer, and
secret fields plus registered exact literals render as `[redacted]`; ordinary
paths, patterns, commands, flags, limits, and timeouts are not described as
protected.

The interactive host tries enrichment when a tool is requested and retries by
the same stable call ID when its completion arrives, so a transient
request-time history race cannot leave a completed known built-in on a generic
fallback. Edit old/new bodies and tool results stay outside the compact row
because they are bulk content available through diff, approval, artifact, and
explicit detail surfaces—not because all argument values are secret. Unknown,
malformed, or still-unresolved calls show bounded argument keys plus `unknown
schema` or `details unavailable` instead of guessing values.

The projection never changes the runtime event or journal and never enables
raw event arguments. Resumed transcripts derive the same projection from
canonical history, so live and replayed rows disclose the same reviewed
metadata.

The approval modal is the deliberate exception: it receives the runtime's
immutable prepared action through the separate approval channel because the
user cannot make an informed safety decision from key names alone. Edit calls
render the existing bounded line diff; other calls render their bounded
material arguments, exact resource, typed permission set, broad-authority
warning, deadline, and preparation fingerprint. Questionnaire content arrives
through an independent interaction channel and cannot approve a tool.

## 8. Background work

Monitor notifications and child-agent progress reach the TUI immediately as
concise attributed transcript notices: `• source · summary`. They never splice
into a streaming assistant block and never steal composer focus. Terminal
events (a monitor stopped, a child finished) are never coalesced away.

The runtime's bounded safe-boundary inbox remains an internal delivery
mechanism for child results sent to the parent model. It is not a visible or
focusable TUI region. `/agent` provides child list, status, and result detail
on demand.

A todo update replaces the bounded anchored pane above the composer. Public
items show status and text in authored order; sensitive plans show no pane,
even if an invalid replay payload attempts to attach text. The pane is not
focusable and never enters canonical model history. A compact picker replaces
the todo presentation while open and restores it unchanged on close.
Oversized tool output appears as a
bounded preview plus an opaque artifact reference. Artifact bodies remain in
user state and are fetched only through authorized, paginated reads.

On resume, Smith runs the coordinator's asynchronous recovery pass before it
accepts commands. The pass reconciles a lagging parent catalog against each
authoritative protected child checkpoint and reduces records into idle,
interrupted/resumable, blocked, expired, or terminal state without constructing
their provider.
Recovered idle children remain available for `@child-id` follow-up; an exact
interrupted checkpoint runs only after `/agent resume` confirmation. Historical
journal-only children and unresolved process-owned monitor identities appear
once as `legacy_ephemeral` / `process_exit` and are never fabricated into a
live session. Child artifacts remain child-owned until the safe-boundary
coordinator explicitly transfers a bounded copy and records lineage.

## 9. Accessibility

- **Screen readers.** Every modal announces its title as the first rendered
  text. Transcript blocks are separated by a blank line so a screen reader's
  paragraph navigation matches Smith's block structure.
- **Contrast.** Because Smith uses named ANSI colors, contrast is the terminal
  theme's responsibility. Smith's obligation is to never encode meaning in
  color alone (§4) and never rely on dim text for anything a user must act on.
- **No silent time-limited choice.** Runtime safety deadlines are named in the
  prompt and produce an explicit `timed out` result. Expiry never activates a
  default answer or approval. Headless `-p` fails closed with a versioned
  non-success result instead of waiting on stdin.
- **Prompt restoration.** Restored approvals and questionnaires announce
  `restored pending …` after the title, preserve the original request identity,
  and expose the same keyboard hints as a live prompt.
- **Resize.** Every layout is recomputed from scratch on resize; nothing caches
  a wrapped line across a width change.

## 10. What this does not cover

Deferred to a later revision, and therefore not to be invented in code:

- Custom extension-drawn widgets beyond a declarative status item.
- Word-level or intra-line diff highlighting.
- Split panes, multiple simultaneous sessions on screen, and
  application-owned mouse selection or clipboard writes.
- Broad reset/revert-all actions, staged commit/push controls, and automatic
  recovery for arbitrary shell side effects.
- Non-Git change recovery and queued commands during an active turn.
- Themes. There is one look; the terminal supplies the palette.
