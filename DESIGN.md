# Smith TUI Design

The visual and interaction contract for the `smith` terminal client. Task 1.4
requires this document to be approved before TUI implementation continues; it
defines what the code in `crates/smith-tui` is allowed to assume.

Smith is an **operational coding surface**, not a dashboard. The transcript is
the product. Everything else — status, approvals, background work — earns its
space by being unavoidable, and gives the space back when it is not.

The text hierarchy follows the observable grammar of Claude Code: one marker
per speaker, one row per tool call, nested results, hanging indents, and detail
behind one expand key. Smith keeps its own decisions, listed with their reasons
in the "Kept" table below. This is an interaction grammar, not a pixel copy.

## Kept: Smith decisions that do not follow Claude Code

| Decision | Reason |
| --- | --- |
| Sixteen ANSI colours by name, no background fills | Smith cannot know the palette |
| No header; transcript owns the screen | `DESIGN.md` principle 1 |
| Informational commands print inline and need no dismissal | truth spec |
| Enter steers, Tab queues while busy | truth spec |
| `y` / `a` / `n` approvals with a quiet-window guard | owner decision 2026-10-02 |
| Command menu above the composer, five rows | truth spec |
| `●` rather than `⏺`; ASCII todo marks | emoji-capable code points change width between terminals |
| Pointer selection implemented by Smith | truth spec |
| Estimated numbers carry `~`; unknown is stated | `DESIGN.md` principle 3 |

## 1. Principles

1. **The transcript owns the screen.** There is no permanent top header. The
   composer and one compact status footer are the only persistent chrome.
2. **State is legible without color.** Color is a second channel, never the
   only one. Every state that matters is also carried by a glyph or a word,
   because terminals get themed, piped, and screenshotted in monochrome.
3. **Uncertainty is shown, not smoothed.** An estimated token count reads
   `~12.4k`, an unknown cost reads `cost unknown`. Smith never renders a guess
   with the same weight as a provider-reported fact.
4. **Nothing moves that the user did not cause.** Streaming text appends;
   layout does not reflow, jump, or animate underneath a reader.
5. **The keyboard is sufficient for everything.** The pointer scrolls and
   selects, but never uniquely: no action requires it.

## 2. Layout

```text
> explain the retry policy

● The retry policy classifies provider failures into three groups…

● Read(src/retry.rs)
  ⎿  Read 212 lines

● Bash(cargo test -p smith-tui retry) failed
  ⎿  running 3 tests
     test retry::backoff ... ok
     test retry::cancel ... FAILED
     … +14 lines (ctrl+o to expand)

  Todo
  [>] Fix the flaky test
  [ ] Run focused tests
  [x] Inspect retry policy (+2 done)

✻ Working… (12s · ↓ 1.2k tokens · esc to interrupt)
────────────────────────────────────────────────────────────────────────
> also cover the cancellation race▏
────────────────────────────────────────────────────────────────────────
  enter to steer · tab to queue                      gpt-5.3 · dev · ask
```

Regions, top to bottom:

| Region | Height | Rule |
| --- | --- | --- |
| Transcript | flex | Minimum 3 rows; below that Smith renders a size warning only. |
| Anchored pane | 0–9 | A compact picker while one is open; otherwise bounded process-local pending input followed by the latest public plan. Hidden when neither exists, and once a plan's every item is completed and its turn has stopped. |
| Working row | 0–1 | One live progress row directly above the composer while a turn runs; never part of the transcript. |
| Composer | 3–10 | 1–8 rows of input between two dim rules, with a `>` prompt. |
| Footer | 1–2 | One hint row below the composer: shortcuts or busy keys on the left, model, profile, and approval mode on the right. Resource pickers and prompts may add controls on the second row. Slash completion does not. |

Typing `/` opens command completion as a compact bottom-pane list directly
above the fixed composer. The command list shows at most five rows in two
aligned columns and scrolls its selected window; long descriptions yield
before the command name and selection. Only the selected row adds its argument
grammar on a detail line beneath it. Smith keeps this placement and five-row
window as its own decision.
Local resource choices opened by `/model`, `/provider`, `/profile`, `/resume`,
or `@` reuse that placement and show at most five matching rows; moving the
selection scrolls the bounded window instead of expanding or covering the
transcript. Cancelling a picker that a command opened leaves the composer
empty. Any compact picker temporarily replaces the todo pane, but only a
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

An empty, idle transcript shows a compact getting-started guide: type a task,
choose a model with `/model`, add a connection with `/connect`, and discover
commands with `/help`. It uses existing text and accent tokens, disappears
when transcript content or active work exists, and never enters session
history. The composer and identity footer retain their ordinary placement.

### Narrow and short terminals

- Below 60 columns the footer keeps model, profile, approval mode, and the
  active keys. Path and secondary state yield first; hints are dropped last.
- Consequential overlays below 60 columns use the full safe width, put the
  title and exact target first, word-wrap the action, place, deadline, and
  warning, and keep decision controls on their own lines. Permissions and raw
  arguments remain behind `Ctrl+O`; expanded detail is scrollable rather than
  clipped. The transcript stays scrollable while a prompt is open.
- Below 10 rows or 40 columns Smith renders only `terminal too small (need
  40×10)` — a half-rendered coding surface is worse than an honest refusal.
  An open prompt remains queued and unanswered while the terminal is too small.

### Setup before the coding surface

A genuinely empty interactive launch opens `Smith setup` before a runtime,
session, tool registry, approval channel, journal, or provider transport is
constructed. Partial or malformed configuration is an error, not an excuse to
replace user state. Non-interactive and machine-output launches never open
setup.

Setup is a keyboard-first sequence inside one frame, titled `Smith setup`
once: action, provider, authentication, model, automatic limit discovery,
response compatibility, default selection, and review. Each step presents one
choice or field. Each listed entry has a name line and a wrapped dim description
line; footer keys remain visible throughout. For a custom model Smith first
checks the endpoint's bounded model listing, then the trusted catalog. Only
when neither source knows the model window does setup ask for one numeric
value: the total context window. Smith derives the input ceiling from that
window and the output ceiling from its automatic request-budget rule; it does
not present separate input/output token fields. The review names every
non-secret value and its provenance, the exact user-config destination, and
the pending local preflight. API-key text is rendered only as masking glyphs.
Every listed entry starts its flow; an entry that cannot proceed says why.
Labels and review text are derived from the values that will be written.
`Shift+Tab` goes back with the previous non-secret provider name, endpoint,
and model available for editing. Returning to a field invalidates any pending
collision approval so changed values pass through review again. Secret input
is never restored. `Esc` cancels without writes; a denied credential
service returns to authentication with the environment-reference option still
available.

Publication is transactional. Smith enrolls the reviewed credential, writes a
same-directory atomic user-config edit, then exercises the shared runtime
factory's derivation-only preflight. Failure restores the exact prior config
bytes and prior credential. Preflight sends no provider request and constructs
no session state. Only a successful automatic first-run continues into the
ordinary coding surface; explicit `smith setup` commands exit after success.

## 3. Type and glyphs

Terminal typography is the user's, not ours. Smith commits only to a fixed
column grid, and assumes nothing about font family, ligatures, or size.

**Width safety.** The grammar's glyph set is `>`, `●`, `⎿`, `✻`, `❯`, `✓`,
and `…`. Each must report width 1 from `unicode-width` and must not be an
emoji-capable code point. `⏺`, `✳`, and `✔` are excluded: their width changes
between terminals. Other rendered glyphs are ASCII or verified single-width
code points. Right-aligned columns are computed from display width, never
`str::len`. Todo marks remain ASCII.

One marker per speaker, with separate glyphs for controls and progress:

| Marker | Meaning |
| --- | --- |
| `>` | User message |
| `●` | Smith or model prose, tool call, or informational notice |
| `⎿` | Nested detail or output belonging to the row above |
| `✻` | Working indicator or ephemeral successful-turn summary |
| `❯` | Selected menu or picker row |
| `✓` | Confirmed or current choice, accompanied by a state word |
| `…` | Folded output or a shortened value |

The first line of a block owns its marker. Continuation lines hang under the
text, never under the marker: two columns for user and Smith prose, and the
nested result's text column for output under `⎿`. User text remains the
terminal foreground rather than turning the whole prompt cyan. Errors and
denials remain Smith blocks, with words naming their state.

### Tool rows and nested results

A model-requested tool call is one row: `● Name(reviewed argument)`. The name
is the reviewed tool label (`Bash`, `Read`, `Update`, `Search`, `List`, or
`Agent`), not the registry id. The argument is the reviewed summary a person
would say. Protected arguments stay protected; do not list argument names or
print `details unavailable` in their place.

The bullet is dim while running, green on success, and red on failure or
denial. A non-success row ends with the word `failed` or `denied`; color is
never the only signal. The tool name is bold, while its reviewed argument is
dim. A call's completion updates its row rather than printing another
invocation. A user shell shortcut echoes once as `! command`, with its result
nested in the same way.

Results show at most four lines under `⎿`, then
`… +N lines (ctrl+o to expand)` with the remaining-line count. A tool with no
useful preview gets a one-line summary, such as `Read 212 lines` or
`Updated src/retry.rs with 4 additions and 1 removal`. Edit rows already name
their changes; a turn that changed no files gets no separate changes notice.
The reviewed successful-row suppression set in §7 remains suppressed; failed,
denied, and unreported calls remain visible.

`Ctrl+O` is the single expand key for folded tool output, approval detail, and
long diffs. It toggles all folded detail; pressing it again folds that detail.
It works while a prompt is open, and `/details` is the command form of the same
toggle. Expanded output remains bounded and redaction-safe; artifact bodies
still require authorized, paginated reads.

### Working row and turn end

Reasoning is progress, not a second assistant reply. From `turn_started` until
the turn ends, one dim working row sits directly above the composer:
`✻ Working… (12s · ↓ 1.2k tokens · esc to interrupt)`. It never enters the
transcript, and appending a block never moves or duplicates it. Elapsed time
comes from a local monotonic clock. Token flow is omitted until the provider
reports it and carries `~` when estimated. During retry or backoff, the retry
wording in §6 replaces `Working…`; elapsed time, available token flow, and the
interrupt key remain visible. Reduced motion uses a static `●`.

At a successful turn boundary the working row becomes one dim
`✻ Worked for 1m 12s` summary attached to that turn, beneath its last block
when one exists. It is not a transcript row, is not replayed as history, and
is hidden as soon as any later block is appended, including a local command
result. Its duration comes from the canonical millisecond interval between
the turn's start and completion envelopes: sub-second turns use milliseconds
(zero renders `<1ms`), longer turns use the compact second/minute/hour grammar,
and an absent or backward interval omits the duration instead of substituting
local reducer time. Earlier turns never show a summary on replay. Absence of
visible assistant text never adds a `reasoning only` diagnosis. Interrupted,
limited, needs-input, and failed turns retain their attributed notice with live
elapsed time when available. Canonical session history and the journal retain
the start/completion events, timestamps, and reasoning needed for model
continuity, replay, timelines, and diagnostics.

## 4. Color

Sixteen-color ANSI only, by name — never by RGB or 256-color index. Smith
cannot know the user's palette, so it must ask for "red" and let the terminal
decide what red is. This is also what makes light and dark themes work without
Smith detecting either.

| Token | ANSI | Used for |
| --- | --- | --- |
| `default` | terminal default | Assistant text, transcript body |
| `dim` | dim modifier | Timestamps, hints, reviewed tool arguments, descriptions, code language labels |
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

- **Never color-only.** Successful tools have a written result or summary;
  failure and denial carry `failed` or `denied`. Confirmed choices carry a
  state word beside `✓`. Other errors and warnings name their condition.
- **No fixed background fills.** Composer, user text, and all other surfaces
  use the terminal default background; Smith cannot know the user's palette.
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

The composer sits between two dim rules, uses a `>` prompt, and shows
placeholder text when empty. A leading `!` switches the prompt to `!` and
the left hint to `bash mode`. `@` and `/` open their pickers. Up and Down move
between lines of a multi-line draft; they reach history only from the first or
last line. `Shift+Enter` and `\` then Enter insert a newline.

The hint row is below the lower rule. On the left it reads `? for shortcuts`
when idle and empty, and names the busy keys while a turn runs. On the right
it shows model, profile, and approval mode. Hints are the last thing dropped
when the terminal is narrow. `?` on an empty draft opens a shortcuts panel in
the anchored pane; any key closes it. The panel is not a transcript entry and
never contacts the provider.

Smith enables button and drag reporting (`1000`/`1002`, SGR-encoded) and owns
pointer selection itself. This is forced: mouse reporting is terminal-wide and
all-or-nothing on the button, so asking for wheel notches also takes away the
drag the terminal needs for its own selection, and no terminal protocol offers a
wheel-only mode. Rather than trade one away, Smith paints the selection and
writes it to the clipboard on release.

Selection is screen-space — a rectangle of *rendered cells*, read back out of
the frame buffer at copy time rather than mapped through word wrap and scroll
offset. So a drag copies exactly the glyphs under it, and crosses the
transcript, composer, and footer indifferently. The cost is that a highlight is
only valid against the frame it was drawn over: scrolling, new output, or a
resize clears it rather than marking whatever moved into those cells. A
successful copy is silent, the highlight being its own receipt; only a failed
clipboard write reports. All-motion reporting (`1003`) stays off — Smith has
nothing to do with a hovering pointer.

Bracketed paste remains enabled independently of pointer handling.

| Key | Action |
| --- | --- |
| `Enter` | Send while idle; steer an eligible serving provider turn while busy |
| `Shift+Enter` / `Alt+Enter` / `\` then `Enter` | Newline in the composer |
| `Esc` | Leave the child inspector for the root timeline; otherwise interrupt the running turn, with uncommitted steers resubmitting only eventual discards after cancellation; if idle, clear the composer |
| `Ctrl+C` | Add a non-blank composer draft to bounded local history and clear it; replace the identity footer with `press Ctrl+C again to exit` for the 1s double-press window; a second press exits from any state |
| `Ctrl+P` | Open command completion using the shared command registry |
| `Ctrl+R` | Open incremental reverse search over bounded process-local composer history |
| `Ctrl+O` | Expand or fold tool output, approval detail, and long diffs; the same toggle as `/details` |
| `Tab` | Queue a non-empty ordinary prompt while busy; cycle `profile_order` only when empty and idle; otherwise complete or move the active overlay selection |
| `Shift+Tab` | Move the active completion/questionnaire selection backward |
| `Alt+Up` | Restore the newest explicitly queued future turn for editing; never edit a runtime-accepted steer |
| `Left` / `Right` / `Backspace` / `Delete` | Edit ordinary text by Unicode character; cross or remove a registered paste/image placeholder as one unit |
| `Ctrl+A` / `Ctrl+E` | Move to the start or end of the current draft line |
| `Ctrl+W` | Delete the previous word |
| `Ctrl+U` / `Ctrl+K` | Delete to the start or end of the current draft line |
| `Alt+B` / `Alt+F` | Move one word left or right |
| `PageUp` / `PageDown` | Scroll transcript, including while an approval is open |
| `Home` / `End` | Move to the start or end of a non-empty draft; jump to either transcript edge when the draft is empty |
| `Mouse wheel` | Scroll transcript without changing composer history |
| `Left drag` | Select rendered cells; copies to the clipboard on release |
| `Left click` | Dismiss the current selection |
| `Ctrl+L` | Jump to newest and re-enable follow |
| `?` (empty composer) | Open the anchored shortcuts panel; any key closes it |
| `Up` / `Down` (composer) | Move between draft lines; from the first or last line browse accepted or `Ctrl+C`-stashed history and return to the exact pre-navigation draft |
| `Down` / `Up` (delegated agents) | Past the newest draft, walk the delegated-work panel: each child's read-only log replaces the transcript region, and `Up` from the first child returns to the root timeline |
| `y` / `a` / `n` | Approval: allow once / allow for session / deny, after the 500 ms quiet-window guard; `Esc` denies and `Enter` never answers |
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
Each result has two columns: command name, then one short dim description.
The name column is as wide as the longest visible name. Descriptions start
with a capital and end at a word boundary with `…` when they do not fit. Only
the selected row shows argument grammar on a detail line beneath it. The menu,
`Ctrl+P` palette, and `/help` use the same order and row format.

```text
  /help        List commands and keys
❯ /goal        Inspect or control a multi-turn goal
  /context     Show context usage or choose a window
  /status      Show session, usage, and workspace status
  /model       Switch model
               [OBJECTIVE | edit … | budget N | pause | resume | clear]
```

`Tab` completes the selected command without executing it; `Shift+Tab` moves
selection backward; `Enter` executes; and `Esc` dismisses the menu while
preserving the draft. Name-prefix matches take priority; if none exist, the
menu searches registered descriptions. Enter executes the highlighted match
when the input names no exact command. An exact command retains its explicit
arguments and parser errors. All actions keep their existing idle and approval
checks. `Ctrl+P` opens the same registry and parser. `//` sends a
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

With no overlay open, `Up` from the first draft line begins newest-first
history navigation while preserving the current text as a scratch draft.
Within a multi-line draft, Up and Down move between lines before reaching
history at its first or last line. `Down` past the newest entry restores that
draft exactly; editing recalled text exits navigation without recording the
edit. `Ctrl+R` opens a compact
anchored reverse-search surface over the same history. Typing performs a
case-insensitive substring search, repeated `Ctrl+R` cycles older matches with
bounded wrapping, `Enter` restores the selected text without submitting it,
and `Esc` restores the original draft. Existing pickers, approvals,
questionnaires, and confirmations retain keyboard ownership. A first
`Ctrl+C` during reverse search restores, stashes, and clears the original
draft before applying the existing double-press exit contract.

### Agent-first composer actions

The hint row identifies model, selected main agent profile, and approval mode
on the right. Project/branch and honest context confidence fit only after the
active keys and that identity. The idle empty hint is `? for shortcuts`; during
work it becomes the busy keys without moving the composer. `/help` appends its
local command/composer guide inline, while `?` opens the anchored shortcuts
panel. Neither spends provider tokens or enters canonical history. At 44
columns, low-priority path detail disappears before mode, activity, model,
approval, or context provenance, and hints yield last. After a first `Ctrl+C`,
the entire identity or activity footer temporarily becomes the warning-toned
text `press Ctrl+C again to exit`. A second press within one second exits;
expiry or any other key restores the current status without leaving a
transcript record.

Named `[profiles]` are the shared agent presets. Each profile selects a bounded
`build`, `plan`, or `review` posture and may be enabled for `main`, `child`, or
both placements. `plan` and `review` are read-only; profile instructions are
additive prompt guidance, never permission or approval. `Tab` cycles the
validated main-enabled `profile_order` only when the composer is empty, the
runtime is idle, and no overlay is open. `/profile` uses the same atomic
safe-boundary rebuild and clears narrower provider/model overrides.

Typing `@` at a token boundary opens one bounded picker, with files first and
agents after them. Rows show the plain file or agent identity without a leading
`@`; agents carry a dim `agent` tag and no model metadata. Choosing one inserts
the visible `@identity` mention into the composer. Files are canonical
workspace-relative entries that honor ignore policy. On submit, Smith performs
an exact prepared `read` through the runtime
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
same schema preparation, broad shell authority, scheduler, deadline,
cancellation, checkpoint, bounded output, and artifact-offload path as a
model-requested `shell` call, then echoes `! command` once and nests the
committed result beneath it.
Submitting it is the authorization for exactly that prepared command, once:
Smith does not ask the user to approve a command they just typed, and the
submission grants nothing to any later call, including an identical one
requested by the model. It does not send a provider request. `!!` sends a
normal prompt beginning with one literal `!`.

During work, the latest public todo projection shares a bounded pane anchored
immediately above the composer with pending input. The pane hides the plan once
every item is completed and its turn has stopped, as in §2. A terminal turn
with work still outstanding keeps its reconciled todo. Sensitive plans expose
no item text. A compact picker temporarily replaces this pane without mutating
either projection, which returns when the picker closes. The working row stays
directly above the composer, outside the transcript, while `Ctrl+O` or
`/details` expands bounded redaction-safe detail in place. No aggregate `work`
row is committed at the terminal. Every terminal boundary still reconciles
pending/in-progress todo items to `cancelled (turn_ended_unfinished)` rather
than inventing completion.

The initial command set is deliberately bounded. This order is shared by the
menu, palette, and `/help`:

| Command | Result |
| --- | --- |
| `/help` | Show a short start-here guide, commands in menu order, and keys in a two-column table locally. |
| `/goal [OBJECTIVE\|edit …\|budget N\|pause\|resume\|clear]` | Inspect or control one persistent multi-turn session goal without sending the command to the provider. |
| `/context` | Visualize the latest model-facing context plan, free input space, reserves, and compaction state locally. |
| `/status` | Show resolved runtime, context window, session, permission, Git, child, and attribution state locally. |
| `/model [PROVIDER/MODEL]` | With no pair, choose from provider-qualified models; apply provider and model atomically. |
| `/diagnostics` | Show Context, Cache, Recovery, and Session facts locally, one fact per line. |
| `/think [on\|off\|default]` | Inspect or change thinking for the next complete turn when the provider/model exposes an exact toggle. |
| `/effort [LEVEL\|default]` | Inspect or change reasoning effort using only levels advertised for the active provider/model. |
| `/details` | Expand or fold bounded redaction-safe tool output, approval detail, and long diffs; the same toggle as `Ctrl+O`. |
| `/timeline` | Show ordered root turn, child, terminal plan/gate, and recovery evidence locally. |
| `/new` | Save the current session and create a fresh identity. |
| `/resume [ID]` | With no ID, choose a project session locally; otherwise validate and resume `ID`. |
| `/profile [NAME]` | With no name, choose a configured profile; apply it while clearing narrower overrides. |
| `/provider [NAME]` | With no name, choose a configured provider; cascade to its model choices when needed. |
| `/agent [ID\|previous\|next\|parent\|resume ID]` | List/inspect children or explicitly resume one safe interrupted checkpoint while the root composer retains focus. |
| `/diff [SCOPE]` | Inspect all, last-turn, staged, unstaged, untracked, file, or hunk changes. |
| `/review [SCOPE]` | Confirm and launch a provider-backed read-only review. |
| `/undo` | Preview the last fully attributable Smith turn and require explicit confirmation. |
| `/redo` | Preview and explicitly confirm the newest exact undo/selective-revert forward patch. |
| `/revert [FILE]` | Select one current file or hunk, preview it, and require explicit confirmation. |
| `/quit` | Exit under the active-work policy. |

### Inline local results

Read-only local commands append an attributed Smith block with a `●` marker
and a magenta command label. `/help`, `/status`, `/context`, and `/diagnostics`
stay inline and need no dismissal. Labels and values use aligned columns;
values word-wrap with a hanging indent and never break mid-word. Long paths
are shortened from the left with `…`. An unknown value reads `unknown`.
A result longer than the pane opens at its beginning; scrolling and `Ctrl+L`
retain their existing behavior.

`/help` leads with a short "start here", then commands in the menu's order and
format, then keys in a two-column table. Key descriptions use plain words,
such as `Enter while working: send now`. `/diagnostics` groups Context, Cache,
Recovery, and Session facts under headings, one fact per line; unreported
values read `unknown`, never `?/?/?`.

```text
● /status
  session         session-…
  provider        openai
  model           gpt-5.3
  context window  96% input left (2.7k used / 68.9k budget)
  model window    200k total · 131k reserved
```

`/status` uses dim labels and normal-foreground values. Its context section
names Smith's enforced **input budget** separately from the model's total
window and reserved output/reasoning space. It shows the latest request plan
and its exact/estimated provenance; cumulative provider input is separately
labelled as session usage because it is not the active context size. Before the
first plan it says `unknown (not planned yet)` instead of showing zero. Local
results stay unboxed: help section names are bold, command names and inline
code are cyan, diff additions are green, removals are red, and hunk headers
are cyan. Empty results use a dim `●`; errors use a red `●` with `failed`.
Unavailable and error results state their condition in words, not color alone.
Content word-wraps at terminal width and is bounded before it enters the
transcript, with an explicit truncation line rather than silent clipping.

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
plan their rows read `unknown (not counted yet)` while the grid renders only
known capacity and reserve. Unknown never becomes zero, and Smith does not
synthesize a request merely to size it because tool activation depends on the
submitted turn.
Below the map, Smith names exact versus estimated counting, segment
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

`/help`, `/status`, `/context`, `/diagnostics`, `/agent`, and every `/diff`
scope use this primitive. Consecutive results append in order and remain
visible while the composer stays active. They are TUI-local display records:
they are never represented as user
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
and `Esc` to restore the untouched composer draft. Rows use two aligned columns:
name, then one short dim description, with state at the right edge. Names and
state remain visible at 44 columns; descriptions yield at a word boundary with
`…`. Active choices read `✓ current`; incompatible or incomplete entries remain
visible with an `unavailable` reason but cannot be selected. Provenance and
full limits appear only on the selected row's detail line, never on every row.

```text
  Choose model · type to filter                                   1/10
❯ example-model       local · 128k context                  ✓ current
  sonnet              Claude Code CLI · 200k context
  opus                Claude Code CLI · 200k context
               limits from project config · input 124k · output 4k
```

Empty model and provider inventories point to `smith setup`; an empty session
inventory says there is nothing to resume. An unmatched filter instead says
there are no matches and offers `Ctrl+U` to clear the query without choosing a
resource.
At 44 columns, resource-picker hints preserve Enter/choose and Esc/cancel;
optional filtering guidance yields first.
Filtering and selection resolve no credential, read no model history, make no
network request, and spend no provider tokens.

Reasoning presence and reasoning control are different facts. On an unknown
endpoint a catalog boolean that says a model reasons means only fixed
reasoning. Adjustable controls require source-explainable switch behavior,
ordered effort choices, optional token-budget support, a provider wire
dialect, defaults, and provenance from an exact trusted provider/model
binding, explicit trusted configuration, or an exact endpoint that normalizes
controls itself — the OpenAI `reasoning_effort` ladder, the xAI Responses
OpenAI-effort ladder for catalog-backed Grok models, the OpenRouter unified
reasoning API, and the Z.AI Coding Plan thinking switch, which apply to every
catalog-advertised reasoning model they serve. A trusted exact model binding
may also declare the native Anthropic Messages `anthropic-effort` dialect;
mandatory adaptive thinking keeps `off` unavailable while the advertised
effort ladder remains selectable. On those endpoints the frozen
Models.dev snapshot refines the default ladder with each model's advertised
control shape; a snapshot annotation is advertised metadata, not an
entitlement claim, and it never creates controls on an endpoint whose wire
dialect is unknown. Smith snapshots the effective selection at turn acceptance and
uses it unchanged for retries and tool continuations. Compatible session
overrides resume and flow into newly created children; a provider/model switch
that cannot represent the override clears it with a local notice rather than
mapping to a nearest value.

For configured OpenAI, xAI, OpenRouter, and Z.AI Coding Plan endpoints, model rows
may come from Smith's frozen Models.dev snapshot as well as explicit TOML. The exact
endpoint binding is a trust boundary; a matching provider name alone is not.
Catalog rows retain the configured provider alias and lead with the catalog
display name plus a short provider/context description. The selected row's
detail line shows the provider-qualified ID, full limits, coding capabilities,
and source revision and age labelled `advertised`. Output metadata distinguishes
the model's published `output ceiling` from Smith's effective `request` budget,
including whether that request value is automatic or configured. The word
`advertised` is deliberate: the row does not claim that the current account,
plan, region, or credential is entitled to use the model.

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
selectable with unknown fields labelled `unknown`; newer incompatible schemas
remain visible but disabled. Bare `--resume` is interactive-only, while an
explicit session ID works unchanged in terminal and headless modes.

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
action: the command or reviewed patch, with exact target and material arguments
place and deadline
the one applicable warning
question
choices, each on its own line with its key
```

```text
╭ Bash command ────────────────────────────────────────────────────────╮
│                                                                      │
│   cargo publish --dry-run                                            │
│   in ~/work/api · deadline 14:30 (up to 10 min)                      │
│                                                                      │
│   Warning: Runs outside the sandbox with your files, environment,    │
│     and network.                                                     │
│                                                                      │
│   Do you want to proceed?                                            │
│     y  Yes                                                           │
│     a  Yes, and don't ask again for `cargo publish` this session     │
│     n  No                                                    (esc)   │
│                                                                      │
│   ctrl+o details · 1 more waiting                                    │
╰──────────────────────────────────────────────────────────────────────╯
```

The action leads; the identity hash, full typed permission list, and raw
arguments stay behind `Ctrl+O`. The exact target, material arguments,
broad-authority warning when applicable, and deadline remain visible before a
decision. For an edit the body is the reviewed diff, with bounded folded detail
and the same expand key instead of a fixed 18-row cut. The transcript remains
scrollable with its usual keys while the approval is open; scrolling never
answers or dismisses it.

`y`, `a`, and `n` answer exactly that fingerprint. A decision key counts only
after the prompt has been visible for 500 ms with no key arriving in that
window; each arriving key restarts it, so text the user was already typing can
never answer a prompt, and the draft is kept. `Esc` denies, `Enter` never
answers, and no choice is defaulted. The same guard covers rotation,
trust, and recovery confirmations. Multi-line action text keeps its line
breaks. Every confirmation names its own action in its body and its controls.
Edited arguments are never
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
then stops and shows `▼ following paused` in the hint row. `Ctrl+L`, `End` with
an empty draft, or sending a message resumes it. Streaming never yanks the
viewport away from someone reading.

## 6. Streaming and motion

- Provider text and reasoning deltas enter speculative buffers keyed by
  `(request, attempt)`, never the committed transcript directly. Visible model
  text uses the same `●` block grammar while streaming and after commit;
  provider reasoning stays out of the visible transcript.
- An explicit attempt-commit event appends that attempt to the assistant block.
  An explicit discard removes its raw text. Usage from the discarded attempt
  remains available to status and diagnostics.
- Streamed and committed text go through one Markdown renderer. Committing a
  block never changes the layout of text already on screen. An unclosed
  construct renders as far as it is known: an open code fence runs to the end
  of the buffer, while an unclosed `**` stays literal.
- Supported constructs are headings, bold, italic, inline code, fenced code
  with its language as a dim label, ordered and unordered lists with nesting,
  block quotes, tables, horizontal rules, and links. Tables wider than the
  pane fall back to stacked `key: value` rows. Links show an underlined label
  and, when the label is not the URL, the visible URL in dim parentheses. OSC 8
  hyperlinks are emitted only when the terminal is known to support them.
  Emphasis requires a non-space inside each marker; `2 * 3 * 4` stays literal.
- `ProviderAttemptFinished` owns the retry decision shown by the root
  conversation. When its optional `index`, `max_attempts`, and
  `retry_delay_ms` metadata says a next attempt has been admitted, the
  transcript appends one informational notice led by the action:
  `provider · retrying 2/3 in 200ms: <bounded provider cause>`. The notice
  never uses the terminal error marker. The initial attempt remains
  `Working…`; while the admitted delay is pending, its live row reads
  `✻ Retrying 2/3… (<turn elapsed> · backoff <positive rounded-up wait> · esc to interrupt)`.
  When the matching `ProviderAttemptStarted` arrives, the retry identity is
  retained and the backoff segment is replaced by the ordinary provider-phase
  elapsed segment. A `retry_delay_ms` of `0` is an admitted immediate retry;
  absent metadata never invents an attempt total, delay, or retry admission.
- If the finish metadata identifies the final configured attempt and carries no
  retry delay, the root renders one attributed terminal provider cause as
  `provider · failed after 3/3 attempts: <bounded provider cause>`. A
  non-retryable failure keeps the existing runtime error path and is not
  duplicated. Successful completion, cancellation, a new turn, shutdown, or
  any terminal turn event clears the bounded retry presentation state.
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
- The only animation is the single-cell `✻` working indicator directly above
  the composer while a turn is active. Nothing in the transcript animates.
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
ordinary; a present file must resolve inside the project root to a regular UTF-8
file no larger than 32 KiB — an in-project symlink is followed, one leaving the
project is refused — or startup fails before provider, session, or terminal
work. Smith
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
| `unknown ctx` | Unknown — never rendered as `0` |
| `$0.031` | Exact, from a versioned price reference |
| `~$0.031` | Estimated |
| `cost unknown` | No price reference for this endpoint |
| `⚡8.0k` | Cache read observed this turn |
| `⚡unknown` | Provider exposes no cache evidence |

Switching provider or model renders a one-line transcript notice —
`● provider changed · openai → anthropic · prior cache not transferable` — and
the context segment falls back to `~` until the new provider reports usage. It
also clears the resolved price, because a price describes one model and
carrying it across a switch would bill the new one at the old one's rate.

Delegated tokens are counted, kept separate from the root's, and reported at
exit and in `/status`. A session that delegated nothing renders exactly the
line it always did. One that delegated breaks the total down:

```text
total · input 143k · cached 90k · output 2.7k
  root: 12 turn(s) · input 121k · cached 90k · output 2.4k
  agents: 4 agent(s) · input 22k · output 302
$0.031 exact · openai/gpt-5.3
```

The merged line carries counters only and never a turn count. A child's turns
belong to the delegation coordinator and never enter this projection, so the
only turn figure available is the root's — printing merged tokens beside it
would claim those turns spent those tokens. Compactions stay on the root line
for the same reason. Only what this process observed is counted: a child
recovered from an earlier process contributes nothing and is not a
contributor, because inventing counters for it would be worse than admitting
they are unknown.

Cost is computed from the catalog's per-counter price for the exact
provider/model binding the session resolved, and root and delegated counters
are priced by that same reference. It is labelled `exact` only when every
contributing counter was both provider-reported and priced; anything
tokenizer-estimated, provider-derived, or unpriced downgrades it to
`estimated`. Reasoning tokens are billed separately from output and Models.dev
publishes no reasoning price, so they are never priced at the output rate —
a nonzero reasoning counter downgrades the label rather than being charged a
rate the source never published. When the catalog carries no price for the
model, `/status` reports the cost as unknown and names the binding it has no
price for, while the exit report prints its token lines and no cost line at
all: never a price substituted from another model, provider, or a hard-coded
default. Cost is presentation only. It never enters routing, approval,
context, or budget decisions, and never reaches the model.

### Tool argument visibility

Canonical runtime events do not expose tool argument values by default. The
interactive host resolves the matching canonical in-process call by ID and
clones its arguments, applies the same credential-shaped-key and exact
registered-secret redaction used by persistence, and passes only that redacted
clone into a typed tool-specific TUI projector. Each call owns one
`● Name(reviewed argument)` row, showing only its reviewed label and bounded
argument summary, with its result nested beneath it:

```text
● Read(src/retry.rs · lines 4–23)
  ⎿  Read 20 lines

● Search("TurnCompleted" in crates · rs files · up to 20 matches)
  ⎿  Found 3 matches

● Bash(cargo test -p smith-tui · in . · up to 30s) failed
  ⎿  running 3 tests
     test retry::backoff ... ok
     test retry::cancel ... FAILED
     … +14 lines (ctrl+o to expand)
```

Completion updates the same row; non-success ends with `failed` or `denied`.
Result bounds and the `Ctrl+O` / `/details` toggle follow §3.
Credential, authorization, API-key, token, password, private-key, bearer, and
secret fields plus registered exact literals render as `[redacted]`; ordinary
paths, patterns, commands, flags, limits, and timeouts are not described as
protected, but enter the row only through its reviewed summary.

The interactive host tries enrichment when a tool is requested and retries by
the same stable call ID when its completion arrives, so a transient
request-time history race cannot leave a completed known built-in on a generic
fallback. Edit old/new bodies and tool results stay outside the compact row
because they are bulk content available through diff, approval, artifact, and
explicit detail surfaces—not because all argument values are secret. Unknown,
malformed, or still-unresolved calls show only the reviewed label and any
reviewed argument summary. Without a reviewed summary, the row shows the label
alone. It never lists argument names, prints `details unavailable`, or guesses
values. Results remain nested and redaction-safe.

The projection never changes the runtime event or journal and never enables
raw event arguments. Live and resumed transcripts must derive the same reviewed
projection from canonical history. Process-only enrichment is not replayed and
does not supply durable transcript metadata.

A small reviewed set of rows is suppressed entirely, and only when the call
succeeded: `write_todos`, whose effect the anchored pane already renders;
`registry.search`, a capability bootstrap the user did not ask for; and the
`agent` actions delegation's own lifecycle lines already report — `wait`,
`result`, `resume`, and `stop`. `agent spawn` always renders, because it is
the one row that announces a spawn, and `agent follow_up` and `agent list`
render because nothing else reports them. The set is enumerated in code, never
inferred from a call's name, arguments, or result size, and the delegation
action comes from the reviewed projection's own vocabulary rather than a scan
of free text. A failed, denied, or unreported call always renders its row: a
hidden row is a claim that something else already said this, and a failure is
redundant with nothing. Suppression is presentation only — the call, its
arguments, and its result stay in canonical history, the journal, and machine
output — and it applies identically live and after resume.

The approval modal is the deliberate exception: it receives the runtime's
immutable prepared action through the separate approval channel because the
user cannot make an informed safety decision from a compact summary alone.
The action leads: edit calls render the bounded line diff; other calls render
their bounded material arguments and exact resource. Place and deadline, the
applicable broad-authority warning, the question, and the choices follow. The
full typed permission set, preparation fingerprint, and raw arguments remain
behind `Ctrl+O`, as in §5. Questionnaire content arrives through an independent
interaction channel and cannot approve a tool.

## 8. Background work

Monitor notifications reach the TUI immediately as concise attributed
transcript notices: `● source · summary`. They never splice into a streaming
assistant block and never steal composer focus. Terminal events (a monitor
stopped, a child finished) are never coalesced away.

Child-agent progress reaches the TUI on the same deadline but not the same
surface. Only delegation's boundaries are the root conversation's business.
A spawn announces itself exactly once, on its own tool row rather than a
separate notice repeating it:

```text
● Agent(spawn · "explore the autoloads and data layer" · read only
  · shared · profile inherited)
  ⎿  Spawned agent
```

The reviewed projection supplies the action, the bounded task excerpt, the
tool scope, and the workspace the call declared. It names an explicitly chosen
profile; otherwise it says `profile inherited` without inventing a resolved
name. The result nests under `⎿` with the same bounds and expand key as any
other tool result. A replayed spawn row must use the same reviewed summary and
result as the live row.

`ChildSpawned` carries no originating call id. Today's host-side spawn
enrichment is process-local and is not replayed. To keep live and replayed
transcript rows the same, the child's id, resolved workspace posture, profile,
and turn ceiling belong in the coordinator-backed panel and inspector unless
canonical history retains the same reviewed metadata and its association with
the call. An unbounded child omits the turn ceiling rather than printing the
sentinel's absurd number.

Every terminal outcome — resuming, blocking on input, completed, stopped,
interrupted, failed — still enters the transcript as `● sub-agent · summary`,
where it occurred in time. Those are new information, not a repetition of the
spawn, so they are never folded back into its row. What the child does in
between is panel state, because the root transcript is a record of the work
the user asked for, and a child narrating each of its tool calls into it
buries that record without telling the user anything the panel row does not
already show.

The panel row says what a child is actually doing, using the same reviewed
tool projection the transcript uses rather than a bare tool name, beside the
profile it runs and the turn and token counts the delegation coordinator owns:

```text
  ● child-1  review · Read(src/retry.rs) · 2/5 turns · 12.4k tokens    1m04s
```

Those counts come from the coordinator on the host's redraw poll, for every
visible child rather than only the inspected one; Smith derives none of them
from the event stream. Everything past the child's id lives in the activity
text, which clips first, so no projection or count can push the docked clock
off screen. A tool with no reviewed display schema falls back to its reviewed
label alone, never raw argument values or a list of argument names. A child
recovered from a durable record has no profile to name — the coordinator's
status carries none — and shows `profile unknown` rather than guessing.

Nothing is discarded. Every lifecycle event, printed or not, appends to that
child's bounded log, which is the inspector's content. `Down` past the newest
composer draft walks the delegated-work panel; the selected child's log
replaces the transcript region until `Esc`, with the panel marking which row
the region belongs to. The view is read-only and never takes focus from the
root composer: while it is open, an ordinary submission is a follow-up to that
child, a `/command` still addresses the root, and the identity footer is
replaced by the keys that say so. Turn, token, session, and workspace figures
in the view's header come from the coordinator on the host's redraw poll, never
from a client-side guess.

A background shell task is the quiet sibling of a monitor: it runs to
completion silently, spools its output to disk, and reports exactly once — a
terminal notice with a bounded tail through the same safe-boundary inbox.
While one runs it stays visible rather than ambient: the identity footer and
delegated-work panel list it by task ID, and the exit confirmation names it as
active work. Ctrl+B moves a running foreground shell command to the background
without killing it; Esc keeps its interrupt-and-kill meaning unchanged.

The root model may name one registered child-enabled agent profile per spawn,
and it resolves through the same preflighted route a user-invoked `/agent
<preset>` resolves — the routes already exist per child-enabled profile, so
this is the model reaching what the user could already reach, not a new
authority. The tool enumerates the available names in its own schema so the
model chooses rather than guesses, and an unregistered, non-child-enabled, or
unrouted name fails the call with an error naming the available ones, creating
no child and no lifecycle event. A spawn that names no profile inherits the
parent's, exactly as it did before selection existed.

A child's write access follows from its resolved profile's posture rather than
a constant, and needs three things together: a posture that is not read-only,
a spawn that asked for the full tool scope, and a workspace policy that is not
the read-only view. Any one of them failing leaves the child read-only. The
workspace key is load bearing rather than belt-and-braces — a read-only view
resolves to the same workspace handle a shared project does, so the tool set
is the only thing withholding a write from it, and the read-only view is what
a spawn that names no workspace gets. A writing child is built from the
parent's own approval policy and workspace, holds no permission the root does
not hold, and cannot spawn a child of its own.

The runtime's bounded safe-boundary inbox remains an internal delivery
mechanism for child results sent to the parent model. It is not a visible or
focusable TUI region. `/agent` lists children and opens the same read-only
inspector the arrows reach, by name rather than by position.

A todo update replaces the bounded anchored pane above the composer. Open
items — pending, in progress, and cancelled — show status and text in authored
order, and every completed item collapses into a single struck row beneath
them naming the most recently completed one, with `(+N done)` when more than
one is finished. A cancelled item is not done and keeps its own row. The
collapsed row is charged against the same visible-item budget an uncollapsed
item would have used, so the pane never grows: the plan's tail shrinks as the
work lands instead of spending a row per finished step.

The anchored pane hides a plan once every item is completed and its turn has
stopped, as in §2, rather than pinning a finished list until the next turn — a
completed plan is otherwise the most persistent thing on screen and the least
useful. It stays while the turn still works, so the user sees the plan land. A
terminal turn with work still outstanding keeps its reconciled todo, because
that is unfinished business rather than a result.

Sensitive plans show no pane, even if an invalid replay payload attempts to
attach text. The pane is not focusable and never enters canonical model
history. A compact picker replaces the todo presentation while open and
restores it unchanged on close.
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
- Split panes and multiple simultaneous sessions on screen.
- Word- or line-granular selection gestures (double- and triple-click), and
  selection that survives the content moving underneath it. Both need a
  text-space selection model; §7 uses a screen-space one.
- Broad reset/revert-all actions, staged commit/push controls, and automatic
  recovery for arbitrary shell side effects.
- Non-Git change recovery and queued commands during an active turn.
- Themes. There is one look; the terminal supplies the palette.
