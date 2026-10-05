# Live pass on 0.3.6: screens outside the main view

2026-10-04. Installed `smith 0.3.6` in a private tmux server at 100x32,
covering the screens neither earlier pass reached: `smith --resume`, first-run
`smith` and `smith setup`, the ChatGPT login method picker, and `/connect`,
`/resume`, and `/model` inside a session. First-run setup used a scratch
`HOME`; everything else used the real `~/.smith`. Captures are in
[captures/](captures/); scratch paths are replaced by `<project>`, `<home>`,
and `<prompt>`.

Not exercised: xAI and Google connection, `/disconnect`, the OAuth progress
screens, and any setup or connection write.

Not confirmed on purpose: setup's review step. A scratch `HOME` does not
isolate the macOS Keychain, so confirming `Store API key securely` would have
replaced the real `smith/zai` entry. The next pass needs a separate Keychain
or the environment-variable method to go past review.

## Five screens, three renderings

The screens a user meets before and around a session look like three
different programs:

| Screen | Rendering | Footer |
| --- | --- | --- |
| `/connect`, `/resume`, `/model` in a session | inline list above the composer, no frame | `type to filter · ↑↓ choose · enter confirm · esc cancel` |
| `smith --resume`, ChatGPT login method, ChatGPT account choice | centered box, 96 wide, sized by entry count | `↑/↓ choose · Enter confirm · Esc cancel` |
| `smith setup`, first run, `/connect <provider>` | centered box, fixed 88x30 | `↑/↓ choose · Enter confirm · Shift+Tab Back · Esc cancel` |

Each standalone screen runs its own terminal loop (`choose_resume_session`,
`pick_one`, `choose_login_method`, `wait_for_login_surface`, setup), which is
why their keys and Esc behaviour differ (R2, S5, C1).

## Findings

| # | Finding | Evidence |
| --- | --- | --- |
| R1 | **`smith --resume` shows one of two sessions.** The box is sized from the entry count, but each entry takes two rows, so the second session is below the frame with no scroll cue. | `01-resume-picker` |
| R2 | **Esc in `smith --resume` exits Smith**, also when the list is empty and its text says `Nothing to resume for this project · Esc to start without resuming`. The caller returns exit code 0 on cancel (`main.rs`). | `02-resume-cancel`, `26-resume-empty`, `27-resume-empty-esc` |
| R3 | **Session rows lead with a truncated id** (`session-2042…`), repeat the full id in the detail, say `1 turns`, and show no time. The 0.3.6 `1 turn` fix covered `/agent` only. | `01-resume-picker`, `22-in-session-resume` |
| R4 | **Empty sessions are kept and offered.** Starting Smith and quitting without a prompt persists a session that `/resume` lists first (`No user preview · 0 turns · unknown provider/model`), `smith sessions list` lists as `?/?`, and the exit line offers to resume. | `22-in-session-resume`, `23-exit-empty`, `24-sessions-list` |
| S1 | **First-run setup opens with an internal sentence:** `no agent session or provider request exists yet`. | `03-first-run` |
| S2 | **Setup descriptions leak placeholders and other surfaces:** `keychain:smith/<provider>` is not filled in; the xAI entry says `Browser login with /connect xai`, a command that cannot be typed in setup; the ChatGPT entry says `unsupported public API boundary`. | `03-first-run`, `05-glm-credential-down` |
| S3 | **Setup layout is inconsistent between steps:** list steps are padded and field steps are not (`│API key`); `Esc cancel` on lists, `Esc Cancel` on fields; `Shift+Tab Back` is offered on the first step; a short list leaves twelve blank rows inside the fixed box. | `03-first-run`, `06-glm-key-field` |
| S4 | **The review step is a field dump:** `limits: context 1000000 · max input 1000000 · max output 131072 (trusted catalog v5)`, `request output: 32768 · output reserve: 32768`, `response: reasoning-only success becomes visible text; thinking stays enabled`, `pending action: write user config, then run local preflight`, and the destination as an absolute path wrapped mid-word. | `08-glm-after-key` |
| S5 | **Esc leaves setup without a word**, and Esc in the ChatGPT method picker reached from setup exits setup instead of going back a step. | `10-esc`, `13-login-esc` |
| S6 | **The ChatGPT steps change frame and vocabulary:** a different box and title, single-row entries, and `Smith PKCE callback · owner-only auth.json · direct Responses calls`. | `12-chatgpt-step` |
| C1 | **`/connect openrouter` in a session replaces the whole screen with a box titled `Smith setup`**, and `/connect` → ChatGPT replaces it with a standalone box; the transcript disappears until it returns. | `18-connect-chatgpt`, `20-connect-openrouter` |
| C2 | **`/connect` rows:** `OpenAI-compatible endpoint  OpenAI-compatible endpoint` repeats the label as its description, and scrolling to it widens the label column for every row. | `15-connect-picker`, `16-connect-bottom` |
| M1 | **`/model` opens at `1/466` on `gpt-5.6-luna`**, not on the current model, and shows five rows under fourteen blank ones. The detail reads `limits from smith-trusted-models r5 · input 255616 · output ceiling 128k [user…`. | `25-model-picker` |

Also seen: quick start proposes `glm-5.2` (`smith-config` `setup.rs`) while
this configuration runs `glm-5.3` as a user-declared model; whether the
trusted catalog should move is a separate question.

## Exit summary with two models

The 0.3.5 pass left this open: a GLM-then-Gemini session printed
`$0.012 exact · google/gemini-3.8-flash`. It is a pricing bug, not only a
naming one. `SessionCost::compute` (`smith-client` `status.rs`) multiplies the
session's cumulative totals by one `PriceReference`; `Status::switch_model`
clears the price but keeps the totals, and `tui_driver.rs` installs the new
model's price. Every GLM token is therefore billed at Gemini's rates, and
because `usage_reported` turns true again on Gemini's first report the label
stays `exact`. `/status` shares the computation. The usage log records no
cost, but files the whole session's totals under the last model's name.

Delegated children are priced at the root's rate by an explicit
`usage-accounting` rule ("the delegated totals are priced by the same
per-counter reference the root totals are"), so a child on another model has
the same flaw; changing that is a spec change.

## Re-check on the 0.3.7 release build

2026-10-04, release build of `feat/inline-pickers-and-cost` at `6d79a98`, same
tmux setup. Captures are in [captures-0.3.7/](captures-0.3.7/). Setup was
completed against a scratch `HOME` with the environment-variable credential
method, so the Keychain was not touched.

Fixed and seen working: R1–R4 (`01-resume`, `24-in-session-resume`,
`27-exit-empty`, `28-exit-with-turn`), S1–S6 (`04-first-run` to `16-setup-complete`;
Esc on the ChatGPT method list returns to setup, `06-chatgpt-esc-back`; Ctrl+C
prints `Setup cancelled · nothing was written`, `15-ctrl-c-cancel`), C1 and C2
(`/connect openrouter` and `/connect chatgpt` draw above the composer with the
transcript in place, `17`–`20`, `25`–`26`), M1 (`/model` opens on the current
model, `21-model-picker`), and the per-model exit line.

Resumed pricing (`29-resume-cost-*`): resuming the single-model session and
quitting prints `$0.000 exact · zai/glm-5.3`; resuming the GLM-then-Gemini
session prints the token line marked `estimated` and no cost line, because
its restored usage cannot be attributed to one model. 0.3.6 printed a
confident figure at the last model's rates there.

Not exercised: completing a connection inside a session (the embedded review,
its effects, and the notices after the rebuild are unit-tested only — every
live `/connect` was left before the step that writes), `/disconnect`, the
ChatGPT and xAI progress screens live, and child-agent pricing.

New:

| # | Finding | Evidence |
| --- | --- | --- |
| N1 | **The environment-variable name is not kept** after going back past its field and forward again; the field is empty. Provider names, endpoints and model IDs are kept. | `10-existing-entry`, `11-env-field-again` |
| N2 | **Ctrl+C after a failed connection check prints the stale error** (`env:ZAI_API_KEY resolves to nothing…`) above `Setup cancelled · nothing was written`. | `15-ctrl-c-cancel` |
| N3 | **Keys typed while the host rebuilds after `/model` are dropped**: text typed right after choosing a model never reaches the composer. 0.3.6 does the same (`23-0.3.6-type-after-switch`), so it predates this change. It is also the cause of the one flaky PTY run (`/quit` sent right after a switch, under full-workspace load). | `22-type-after-switch`, `23-0.3.6-type-after-switch` |
| N4 | **A `/connect` step leaves `? for shortcuts` in the hint row** with the step's keys on a second row; `?` does nothing there. In-session pickers drop it while open. | `17-connect-openrouter`, `25-connect-chatgpt-inline` |
| N5 | **The review's last row says `then checks the connection`**, but the check is local (the credential reference must resolve; no request is sent). `checks the configuration` would be accurate. | `09-review` |
