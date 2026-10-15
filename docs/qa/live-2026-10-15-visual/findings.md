# Live run: visual tool results with cmux-cua (2026-10-15)

Release build of `feat/visual-tool-results` driven in a private tmux server
against the owner's real `~/.smith` config. The scratch project
(`/Volumes/Data/smith-live-cua`) declared cmux-cua as `[mcp.servers.cua]`
(`cmux-cua mcp --socket …`), trusted with `/mcp trust cua`, and set
`vision = "luna"`. Approvals were `ask` and each call was approved by hand.
The task each time: read the operator-button colour and the display value of
an open Calculator window from the screenshot, not the accessibility tree.
Ground truth (an independent cmux-cua screenshot): Programmer mode, grey
operator buttons, display `30`.

| Run | Main model | Build | Result |
|---|---|---|---|
| 1 | `chatgpt/gpt-5.6-luna` | `1106510` + modality override | **Wrong:** "orange", "30". The image never reached the model. |
| 2 | `zai/glm-5.3` (text-only) + `vision = luna` | `1106510` | Placeholder received; found `view_image` only after steering; answered "grey", "30"; then the turn failed on a malformed `end_session` call. |
| 3 | `chatgpt/gpt-5.6-luna` | `f7a359e`, no override | **Correct:** "dark gray/charcoal", "30". |
| 4 | `zai/glm-5.3` + `vision = luna` | `f7a359e` | **Correct**, found `view_image` unaided in 1 min: "dark gray", "30". |

## Findings fixed in `f7a359e`

1. ChatGPT models resolved as text-only. The catalog has no `chatgpt`
   provider, so `vision = "luna"` was rejected and screenshots were
   projected away. Fixed with a modality-only mapping onto the OpenAI
   catalog entry; prices and limits are deliberately unchanged.
2. Smith's ChatGPT adapter wrote `function_call_output` as text only, so
   tool-result images were dropped silently and the model answered from
   priors (run 1's "orange"). It now sends `input_image` items. This
   predated the change and also affected `generate_image` results.
3. Text-only agents could not discover `view_image`. It is now pinned while
   the vision module is mounted, with a hint that `[image <id>: …]`
   placeholders can be inspected through it.

## Open, not part of this change

- A model tool call that violates an MCP tool's input schema (GLM omitted
  `session` on `end_session`) fails the whole turn instead of returning a
  tool error the model can correct.
- `generate_image` may hit the same authority mismatch for its
  `smith:image-files` write that `view_image` hit for reads (reported by
  Codex, unconfirmed).
