---
created_at: 2026-10-23T21:44:00Z
updated_at: 2026-10-23T21:44:00Z
---

# Permanent empirical evidence

## Provenance and limits

This file embeds the **complete captured report**, unchanged, from
`/tmp/smith-jev-benchmark-20261023-Hy0zFx/report.md`; the proposal does not depend on
that temporary file remaining available. Captured on October 23, 2026, on the local
M4 Pro/64 GiB machine. Source report SHA256: `6c6b0f7c26edeefd0aeb1556718518faf2a08cd4772da841c04ed077eab3f1f5`.
The captured report's repository-cleanliness statement describes the earlier
benchmark run, not the later act of authoring this proposal.

The 448 experimental requests are 192 primary four-head requests + 192 follow-up
single-head requests + 64 diagnostics. They reuse **24 distinct manually labeled
prompts**, not 448 independent examples. Primary/follow-up decisions total 960;
diagnostics include extra repeated/singleton/order comparisons. Protocol validation
and initial smoke requests are additional. No two-layer implementation or hosted
Jev/GGUF/Ollama decision model was tested by this experiment.

The report preserves primary D per-case results, all gold prompts/rationales,
latencies, confusion matrices, calibration, follow-up results, actual validation
mismatches, and the port-reuse recovery. Additional permanent tables and raw
probability examples follow the verbatim report. Choosing the inverse question
after these results is in-sample engineering, not independent calibration.

## Complete captured report (verbatim)

# Live local Jev Agent Mode benchmark
Generated 2026-10-23T17:08:42.734380-04:00
Artifacts: `/private/tmp/smith-jev-benchmark-20261023-Hy0zFx`

192 primary HTTP requests / 768 scored decisions; 64 diagnostics. 24 cases = 16 controlled + 8 real session samples. Gold labels and question variants were frozen before inference.
All primary requests returned HTTP 200. Models were real pinned MLX bf16 releases, not fake. No weights were downloaded.

## Executive outcome

The installed models and the real decision endpoint work. Prompt wording dominates clarification behavior. In the original detailed four-question variant, both models detected only 1/8 must-clarify prompts. A subsequent inverse-actionability formulation improved 2B to 22/24 accuracy with 8/8 clarification recall and two false positives. This is exploratory, in-sample evidence, not a production-safety claim.

## Machine and installation

- Date: October 23, 2026 (EDT); Apple M4 Pro, 64 GiB RAM; macOS 27.2.
- `~/.local/bin/jev-style` links to `~/.local/share/uv/tools/jev-style/bin/jev-style`; `/opt/homebrew/bin/jev-style` is absent.
- uv-managed `jev-style` 0.5.0, Python 3.12.11, MLX 0.32.4, mlx-lm 0.31.3, transformers 5.20.2. Torch and llama-cpp-python are not installed in that environment.
- Cached pinned MLX BF16 weights: 0.8B revision `7f14c9fa1491d168a7f70b16acf68baf9d4f7353` (1,504,827,355-byte model file); 2B revision `11ce5d718e22412b38624bb863a1eee73c0a5934` (3,763,691,755-byte model plus 420,112-byte FP32 norms). Cache root is `~/.cache/huggingface/hub/models--chaoliangUNSW--Jev-Style-{0.8B,2B}-Decision-v3-MLX/snapshots/`.
- No Jev/Ollama server was running initially. Temporary services bound only 127.0.0.1. HF_HUB_OFFLINE=1 and explicit local model directories prevented downloads.
- Ollama 0.40.8: `ollama list` initially failed with no server. After a temporary server was started, `ollama list` exited 0 with an empty table and `/api/tags` returned `{"models":[]}`. Ollama chat for `jev-style` returned 404 model-not-found. No v1/v2 letter readout can be assessed without an installed model.
- No `jev-score`/`jev-score-v2` executable or Jev decision GGUF weights were found on PATH, in the examined bin/Python-env paths, caches, or either Novelkit tree. The `ggml-vocab-*.gguf` files inside Novelkit's llama.cpp source checkout are tokenizer/vocabulary assets, not a Jev model.
- Both `/Volumes/Data/codes/ai/novelkit/scripts/jev-style-backend.py` and the corresponding `novelkit-speech` script exist. They are adapter scripts, not proof of a running scorer or cached decision weights there.
- `/Volumes/Data/codes/games/world/scripts/jev-style-sidecar.test.mjs`: 4 tests passed, 0 failed/skipped; its positive integration test uses the fake engine. The separate probe and all benchmarks used real MLX models.
- Real endpoint probe: health reported backend=mlx, untrained=false; delivered P(true)=0.9474376987, shipping P=0.9534894928. Probe server startup was 1539.5 ms; main-run startup was 1523.7 ms (0.8B) and 2036.6 ms (2B).

## Accuracy and latency

| Model | Variant | Clarification | Complexity exact | Sub-agents | Profile | All four | Mean / p50 / p95 wall ms |
|---|---|---:|---:|---:|---:|---:|---:|
| 0.8b-v3 | A_terse | 11/24 (45.8%) | 13/24 (54.2%) | 22/24 (91.7%) | 10/24 (41.7%) | 3/24 | 240.9 / 180.2 / 442.9 |
| 0.8b-v3 | B_instructions | 15/24 (62.5%) | 14/24 (58.3%) | 24/24 (100.0%) | 10/24 (41.7%) | 2/24 | 319.1 / 283.3 / 571.6 |
| 0.8b-v3 | C_criteria | 17/24 (70.8%) | 13/24 (54.2%) | 24/24 (100.0%) | 17/24 (70.8%) | 8/24 | 402.0 / 313.0 / 661.8 |
| 0.8b-v3 | D_both | 17/24 (70.8%) | 16/24 (66.7%) | 24/24 (100.0%) | 18/24 (75.0%) | 9/24 | 514.3 / 437.8 / 828.2 |
| 2b-v3 | A_terse | 16/24 (66.7%) | 12/24 (50.0%) | 22/24 (91.7%) | 13/24 (54.2%) | 3/24 | 202.3 / 201.5 / 223.9 |
| 2b-v3 | B_instructions | 18/24 (75.0%) | 16/24 (66.7%) | 21/24 (87.5%) | 18/24 (75.0%) | 8/24 | 389.4 / 398.1 / 416.4 |
| 2b-v3 | C_criteria | 19/24 (79.2%) | 15/24 (62.5%) | 23/24 (95.8%) | 18/24 (75.0%) | 7/24 | 319.4 / 317.0 / 352.8 |
| 2b-v3 | D_both | 17/24 (70.8%) | 17/24 (70.8%) | 21/24 (87.5%) | 18/24 (75.0%) | 8/24 | 492.6 / 494.9 / 515.6 |

A = terse instructions/bare criteria; B = detailed instructions/bare criteria; C = terse instructions/descriptive criteria; D = detailed both. The same cases are reused: this is in-sample prompt engineering, not held-out performance.

## Calibration (D_both)

Binary Brier uses P(true). Multiclass Brier is the sum of squared class errors. ECE uses five equal-width bins of winning probability, not the API confidence field.
API choice confidence is `(k*pmax-1)/(k-1)` clipped to [0,1]. Score confidence measures ordinal spread and is not a probability of being correct. Noul returns only P(true), no confidence field.

| Model | Decision | Brier | Log loss | ECE5 | Mean pmax, correct / wrong | API confidence mean |
|---|---|---:|---:|---:|---:|---:|
| 0.8b-v3 | needs_clarification | 0.2248 | 0.6659 | 0.1236 | 0.7979 / 0.7996 | n/a |
| 0.8b-v3 | complexity | 0.5235 | 0.9805 | 0.2219 | 0.5133 / 0.4467 | 0.4073 |
| 0.8b-v3 | needs_subagents | 0.0254 | 0.1401 | 0.1242 | 0.8758 / n/a | n/a |
| 0.8b-v3 | target_profile | 0.3654 | 0.7053 | 0.1467 | 0.6477 / 0.4727 | 0.5049 |
| 2b-v3 | needs_clarification | 0.1410 | 0.4227 | 0.1511 | 0.8370 / 0.6215 | n/a |
| 2b-v3 | complexity | 0.4567 | 0.8688 | 0.2148 | 0.5672 / 0.4152 | 0.4984 |
| 2b-v3 | needs_subagents | 0.0733 | 0.2320 | 0.1638 | 0.9098 / 0.6788 | n/a |
| 2b-v3 | target_profile | 0.3757 | 0.6453 | 0.0949 | 0.7778 / 0.6791 | 0.6914 |

## Case inventory and gold labels

`C` cases are controlled; `R` cases are real user messages (anonymized if a home path occurred). `*` marks a borderline manual annotation. True/false clarification refers to the isolated message, not its original conversation.

| ID | Prompt | Clarify | Complexity | Sub-agents | Profile | Rationale |
|---|---|---|---:|---|---|---|
| C01 | fix the bug | True | 2 | False | sol | No failing behavior, location, or reproduction is supplied; likely debugging scope is only provisional. |
| C02 | make it faster | True | 3 | False | sol | No target workload, bottleneck, or performance objective is supplied. |
| C03 | can you refactor the auth flow | True | 4 | False | sol | Authentication refactor lacks intended change, constraints, and scope; delegate only after clarifying. |
| C04 | add caching to the API | True | 3 | False | sol | Endpoint, cache invalidation, freshness, and scope are all unspecified. |
| C05 | In crates/smith-config/src/model.rs, change only the Display string for ProfileUse::Child from "child" to "subagent". Do not change the variant name or parsing behavior. | False | 1 | False | code | One explicit display literal; no API or behavior beyond that literal. |
| C06 | Run cargo test -p smith-config and explain any failures. Do not modify files. | False | 1 | False | code | An explicit focused verification command is an operational task, not open-ended research. |
| C07 | In README.md replace the single typo "configuraiton" with "configuration". Make no other changes. | False | 1 | False | code | A single deterministic text replacement. |
| C08 | In crates/smith-config/src/model.rs, add a unit test that ProfileUse::Child serializes to the existing child string and deserializes back. Use the adjacent tests and do not change production behavior. | False | 2 | False | code | A focused test addition following local patterns. |
| C09 | Add --dry-run to the existing smith config set command: parse and validate the same inputs, print the proposed TOML change, and never write the config in dry-run mode. Preserve normal behavior and add CLI tests for dry-run and validation failure. | False | 3 | False | sol | A bounded CLI feature with validation and tests; one coherent workstream. |
| C10 | Refactor authentication across crates/auth-storage and crates/auth-api to use a shared typed AuthResult instead of string errors, preserving HTTP status codes and public wire formats. Delegate the independent storage conversion and API adapter conversion in parallel, then integrate and run both crates tests. | False | 4 | True | sol | Explicit cross-crate refactor with two independent delegated workstreams and integration. |
| C11 | Implement Jev Agent Mode Design B: user -> local Jev decisions -> clarification or lead agent -> sub-agents. Add typed decision contracts, confidence-based fallback, host-enforced permissions, child cancellation, and end-to-end tests. Divide router, runtime lifecycle, and security-test work among independent agents before integration. | False | 5 | True | sol | Explicit multi-agent architecture with parallel independent subsystems. |
| C12 | Read docs/storage.md and the current persistence code. Compare SQLite and PostgreSQL for this single-user offline CLI, focusing on migrations, backup, and operational cost. Report evidence and trade-offs; do not implement anything. | False | 3 | False | research | Clear read-only comparative investigation. |
| C13 | Compare artifacts/before.png and artifacts/after.png for the settings panel. Identify alignment, spacing, contrast, and clipping regressions, referencing the relevant CSS selectors. Do not edit files. | False | 2 | False | visual | A bounded screenshot and layout assessment. |
| C14 | Perform a read-only security audit of crates/auth, crates/upload, and crates/proxy. Trace tenant authorization, path traversal, and SSRF respectively; use three independent audit agents, then deduplicate findings with file/line evidence, exploit prerequisites, impact, and unverified assumptions. | False | 4 | True | security | Explicit independent security audit workstreams, not implementation. |
| C15 | Audit only crates/http/src/download.rs for path traversal in the download handler. Trace user input to filesystem access, report file/line evidence and exploit preconditions, and do not modify code. | False | 2 | False | security | A focused read-only security trace. |
| C16 | Design and implement a distributed agent task graph with a persistent scheduler, bounded concurrent worker agents, retries, cancellation propagation, and an approval gateway. Split scheduler, worker protocol, approval enforcement, and integration-test work into independent sub-agent tasks; preserve existing single-agent mode. | False | 5 | True | sol | A multi-agent scheduling architecture with explicit parallel responsibilities. |
| R01 | 1. a reusable benchmark script first. | True | 2 | False | code | A reusable benchmark script is requested without saying what to benchmark. |
| R02 | but dont do anything first, just evaluate te situation first | True | 1 | False | research | Evaluation has no object or decision to evaluate. |
| R03 | we can drop the clothing focus on body | True | 2 | False | visual | Clothing/body scope is stated but the asset and desired visual result depend on missing history. |
| R04* | all coin did not fall throught, it is like coddle in the begging of rail. never move | False | 3 | False | sol | A concrete coin-physics symptom supports repository investigation before asking questions. |
| R05* | can you please help me take a look on this. how did we estimate the context size? | False | 2 | False | research | The context-size estimator is a named investigation topic in the current coding repository. |
| R06* | nice, can you help e do some research to better estimate it? and the image is not well estimated. | True | 3 | False | research | The missing antecedent of it prevents identifying what estimator to research from this message alone. |
| R07 | can you help me take a look im thinking maybe for decision we can use jev model to speed things up. can you help me do some research? | False | 3 | False | research | A clear read-only research question about using Jev for faster decisions; no implementation is requested. |
| R08 | This is a read-only image-delivery control. Use the read tool to open probe.png. Return only the six-character code printed in its top-left corner, the three shapes and their colors from left to right, and the result of its equation. Do not inspect any other files, modify files, or use shell. Do not guess if the pixels are absent. | False | 1 | False | visual | An explicit image-delivery diagnostic names the image and output fields. |

## Per-case results: 0.8b-v3, D_both

Each prediction carries winning probability; noul uses its P(true). Complexity distribution and raw API expected score are retained in `results.jsonl` and `per-case.csv`.

| ID | P(clarify), prediction | Complexity prediction (pmax) | P(sub-agents), prediction | Profile prediction (pmax) | Incorrect decisions | Wall ms |
|---|---|---|---|---|---|---:|
| C01 | 0.1342, False | 3 (0.4594) | 0.0604, False | code (0.5385) | needs_clarification, complexity, target_profile | 389.9 |
| C02 | 0.1531, False | 3 (0.2797) | 0.2553, False | code (0.4795) | needs_clarification, target_profile | 386.5 |
| C03 | 0.2619, False | 4 (0.6730) | 0.0578, False | sol (0.6351) | needs_clarification | 834.1 |
| C04 | 0.1301, False | 3 (0.4643) | 0.0527, False | sol (0.5948) | needs_clarification | 389.8 |
| C05 | 0.1225, False | 1 (0.4539) | 0.1030, False | code (0.6552) | none | 890.0 |
| C06 | 0.1942, False | 2 (0.4631) | 0.1129, False | code (0.6441) | complexity | 340.8 |
| C07 | 0.2479, False | 1 (0.6840) | 0.0288, False | code (0.8067) | none | 667.4 |
| C08 | 0.1207, False | 2 (0.6476) | 0.0586, False | code (0.7358) | none | 523.0 |
| C09 | 0.1121, False | 3 (0.4049) | 0.0765, False | code (0.5454) | target_profile | 439.7 |
| C10 | 0.1011, False | 4 (0.6028) | 0.6031, True | sol (0.5644) | none | 430.2 |
| C11 | 0.2195, False | 5 (0.5518) | 0.6208, True | sol (0.5212) | none | 727.6 |
| C12 | 0.1330, False | 3 (0.3254) | 0.0578, False | research (0.8395) | none | 409.0 |
| C13 | 0.0764, False | 3 (0.4165) | 0.1178, False | visual (0.4436) | complexity | 431.8 |
| C14 | 0.2559, False | 5 (0.3546) | 0.7533, True | security (0.7782) | complexity | 795.1 |
| C15 | 0.1508, False | 2 (0.3668) | 0.0810, False | security (0.6856) | none | 442.0 |
| C16 | 0.0767, False | 5 (0.6912) | 0.7610, True | sol (0.6403) | none | 444.5 |
| R01 | 0.0883, False | 2 (0.4623) | 0.0737, False | code (0.6118) | needs_clarification | 700.6 |
| R02 | 0.5976, True | 2 (0.4013) | 0.0374, False | research (0.3724) | complexity | 399.0 |
| R03 | 0.2687, False | 2 (0.3052) | 0.1076, False | code (0.3159) | needs_clarification, target_profile | 373.5 |
| R04 | 0.3611, False | 1 (0.4087) | 0.0925, False | code (0.3191) | complexity, target_profile | 435.8 |
| R05 | 0.2520, False | 3 (0.4830) | 0.0674, False | research (0.7043) | complexity | 347.1 |
| R06 | 0.3668, False | 3 (0.6487) | 0.1093, False | research (0.6013) | needs_clarification | 674.3 |
| R07 | 0.3222, False | 3 (0.6520) | 0.0808, False | research (0.8242) | none | 418.4 |
| R08 | 0.2869, False | 2 (0.5867) | 0.0870, False | research (0.6375) | complexity, target_profile | 452.2 |

### 0.8b-v3 D_both errors

- needs_clarification: 17/24; 95% Wilson interval 0.508–0.851; confusion {"False -> False": 16, "True -> False": 7, "True -> True": 1}.
  - R06: gold=True, predicted=False, pmax=0.6332, P(gold)=0.3668, API confidence=None.
  - R03: gold=True, predicted=False, pmax=0.7313, P(gold)=0.2687, API confidence=None.
  - R01: gold=True, predicted=False, pmax=0.9117, P(gold)=0.0883, API confidence=None.
  - C03: gold=True, predicted=False, pmax=0.7381, P(gold)=0.2619, API confidence=None.
  - C01: gold=True, predicted=False, pmax=0.8658, P(gold)=0.1342, API confidence=None.
  - C04: gold=True, predicted=False, pmax=0.8699, P(gold)=0.1301, API confidence=None.
  - C02: gold=True, predicted=False, pmax=0.8469, P(gold)=0.1531, API confidence=None.
  - Selective thresholds: {"0.8": {"covered": 13, "correct": 9, "accuracy": 0.6923076923076923}, "0.9": {"covered": 3, "correct": 2, "accuracy": 0.6666666666666666}, "0.95": {"covered": 0, "correct": 0, "accuracy": null}}
- complexity: 16/24; 95% Wilson interval 0.467–0.820; confusion {"1 -> 2": 3, "2 -> 3": 3, "2 -> 2": 4, "3 -> 3": 6, "5 -> 5": 2, "4 -> 4": 2, "1 -> 1": 2, "4 -> 5": 1, "3 -> 1": 1}.
  - C06: gold=1, predicted=2, pmax=0.4631, P(gold)=0.1702, API confidence=0.4642545286780536.
  - R05: gold=2, predicted=3, pmax=0.4830, P(gold)=0.2272, API confidence=0.4555091822469268.
  - R08: gold=1, predicted=2, pmax=0.5867, P(gold)=0.2115, API confidence=0.5891338597187286.
  - C14: gold=4, predicted=5, pmax=0.3546, P(gold)=0.2121, API confidence=0.0.
  - C13: gold=2, predicted=3, pmax=0.4165, P(gold)=0.3002, API confidence=0.39086373459136936.
  - C01: gold=2, predicted=3, pmax=0.4594, P(gold)=0.2582, API confidence=0.46430793023103323.
  - R04: gold=3, predicted=1, pmax=0.4087, P(gold)=0.1617, API confidence=0.038704237066753966.
  - R02: gold=1, predicted=2, pmax=0.4013, P(gold)=0.2382, API confidence=0.37447208781214025.
  - Selective thresholds: {"0.8": {"covered": 0, "correct": 0, "accuracy": null}, "0.9": {"covered": 0, "correct": 0, "accuracy": null}, "0.95": {"covered": 0, "correct": 0, "accuracy": null}}
- needs_subagents: 24/24; 95% Wilson interval 0.862–1.000; confusion {"False -> False": 20, "True -> True": 4}.
  - Selective thresholds: {"0.8": {"covered": 19, "correct": 19, "accuracy": 1.0}, "0.9": {"covered": 14, "correct": 14, "accuracy": 1.0}, "0.95": {"covered": 2, "correct": 2, "accuracy": 1.0}}
- target_profile: 18/24; 95% Wilson interval 0.551–0.880; confusion {"code -> code": 5, "research -> research": 5, "sol -> sol": 5, "visual -> code": 1, "visual -> research": 1, "security -> security": 2, "sol -> code": 4, "visual -> visual": 1}.
  - R03: gold=visual, predicted=code, pmax=0.3159, P(gold)=0.1378, API confidence=0.14493341859987263.
  - R08: gold=visual, predicted=research, pmax=0.6375, P(gold)=0.1323, API confidence=0.5469210556639024.
  - C09: gold=sol, predicted=code, pmax=0.5454, P(gold)=0.2867, API confidence=0.4317058954881581.
  - C01: gold=sol, predicted=code, pmax=0.5385, P(gold)=0.3553, API confidence=0.4231503568267937.
  - R04: gold=sol, predicted=code, pmax=0.3191, P(gold)=0.2413, API confidence=0.14893407794899904.
  - C02: gold=sol, predicted=code, pmax=0.4795, P(gold)=0.3529, API confidence=0.3493559473221317.
  - Selective thresholds: {"0.8": {"covered": 3, "correct": 3, "accuracy": 1.0}, "0.9": {"covered": 0, "correct": 0, "accuracy": null}, "0.95": {"covered": 0, "correct": 0, "accuracy": null}}
- Complexity argmax MAE 0.3750; expected-score MAE 0.5697; within one level 0.958.
- Controlled vs real-session accuracy: {"controlled": {"needs_clarification": 0.75, "complexity": 0.75, "needs_subagents": 1.0, "target_profile": 0.8125}, "real_session": {"needs_clarification": 0.625, "complexity": 0.5, "needs_subagents": 1.0, "target_profile": 0.625}}.

## Per-case results: 2b-v3, D_both

Each prediction carries winning probability; noul uses its P(true). Complexity distribution and raw API expected score are retained in `results.jsonl` and `per-case.csv`.

| ID | P(clarify), prediction | Complexity prediction (pmax) | P(sub-agents), prediction | Profile prediction (pmax) | Incorrect decisions | Wall ms |
|---|---|---|---|---|---|---:|
| C01 | 0.4770, False | 2 (0.4177) | 0.0868, False | code (0.7802) | needs_clarification, target_profile | 488.8 |
| C02 | 0.6864, True | 3 (0.3537) | 0.1609, False | code (0.5782) | target_profile | 487.3 |
| C03 | 0.3879, False | 4 (0.5649) | 0.0491, False | sol (0.7613) | needs_clarification | 490.6 |
| C04 | 0.3654, False | 3 (0.5094) | 0.0432, False | sol (0.5301) | needs_clarification | 493.4 |
| C05 | 0.0524, False | 1 (0.5769) | 0.0873, False | code (0.9516) | none | 513.3 |
| C06 | 0.0967, False | 2 (0.4354) | 0.0441, False | code (0.5071) | complexity | 454.9 |
| C07 | 0.0683, False | 1 (0.6231) | 0.0281, False | code (0.6547) | none | 496.4 |
| C08 | 0.1589, False | 2 (0.7399) | 0.0339, False | code (0.9504) | none | 465.7 |
| C09 | 0.1642, False | 2 (0.4111) | 0.0373, False | code (0.8235) | complexity, target_profile | 516.0 |
| C10 | 0.2622, False | 4 (0.6779) | 0.3562, False | sol (0.6494) | needs_subagents | 500.1 |
| C11 | 0.3209, False | 5 (0.5823) | 0.2885, False | sol (0.7422) | needs_subagents | 498.9 |
| C12 | 0.0577, False | 3 (0.4822) | 0.0293, False | research (0.9138) | none | 485.7 |
| C13 | 0.0565, False | 2 (0.4547) | 0.0270, False | visual (0.8894) | none | 506.1 |
| C14 | 0.0534, False | 3 (0.3170) | 0.5135, True | security (0.8278) | complexity | 505.7 |
| C15 | 0.0561, False | 2 (0.4491) | 0.0917, False | security (0.8232) | none | 508.4 |
| C16 | 0.3367, False | 5 (0.7149) | 0.3190, False | sol (0.8016) | needs_subagents | 508.5 |
| R01 | 0.2247, False | 2 (0.4468) | 0.0792, False | code (0.6801) | needs_clarification | 486.1 |
| R02 | 0.4684, False | 3 (0.4321) | 0.0568, False | research (0.6300) | needs_clarification, complexity | 490.4 |
| R03 | 0.3969, False | 3 (0.3368) | 0.1199, False | research (0.5412) | needs_clarification, complexity, target_profile | 470.6 |
| R04 | 0.3109, False | 2 (0.3438) | 0.0869, False | research (0.4809) | complexity, target_profile | 502.9 |
| R05 | 0.1173, False | 2 (0.4935) | 0.0447, False | research (0.8936) | none | 457.1 |
| R06 | 0.3290, False | 3 (0.7525) | 0.1974, False | visual (0.8703) | needs_clarification, target_profile | 467.4 |
| R07 | 0.2030, False | 3 (0.8021) | 0.0658, False | research (0.9766) | none | 507.5 |
| R08 | 0.1417, False | 2 (0.6302) | 0.0376, False | visual (0.8168) | complexity | 520.8 |

### 2b-v3 D_both errors

- needs_clarification: 17/24; 95% Wilson interval 0.508–0.851; confusion {"False -> False": 16, "True -> False": 7, "True -> True": 1}.
  - R06: gold=True, predicted=False, pmax=0.6710, P(gold)=0.3290, API confidence=None.
  - R03: gold=True, predicted=False, pmax=0.6031, P(gold)=0.3969, API confidence=None.
  - R01: gold=True, predicted=False, pmax=0.7753, P(gold)=0.2247, API confidence=None.
  - C03: gold=True, predicted=False, pmax=0.6121, P(gold)=0.3879, API confidence=None.
  - C01: gold=True, predicted=False, pmax=0.5230, P(gold)=0.4770, API confidence=None.
  - R02: gold=True, predicted=False, pmax=0.5316, P(gold)=0.4684, API confidence=None.
  - C04: gold=True, predicted=False, pmax=0.6346, P(gold)=0.3654, API confidence=None.
  - Selective thresholds: {"0.8": {"covered": 11, "correct": 11, "accuracy": 1.0}, "0.9": {"covered": 7, "correct": 7, "accuracy": 1.0}, "0.95": {"covered": 0, "correct": 0, "accuracy": null}}
- complexity: 17/24; 95% Wilson interval 0.508–0.851; confusion {"1 -> 2": 2, "2 -> 2": 6, "3 -> 3": 5, "5 -> 5": 2, "2 -> 3": 1, "4 -> 4": 2, "1 -> 1": 2, "4 -> 3": 1, "3 -> 2": 2, "1 -> 3": 1}.
  - C06: gold=1, predicted=2, pmax=0.4354, P(gold)=0.1387, API confidence=0.4541180465459582.
  - R03: gold=2, predicted=3, pmax=0.3368, P(gold)=0.2783, API confidence=0.26469547755523204.
  - R08: gold=1, predicted=2, pmax=0.6302, P(gold)=0.1913, API confidence=0.6629159642777729.
  - C14: gold=4, predicted=3, pmax=0.3170, P(gold)=0.2554, API confidence=0.30777758219803175.
  - C09: gold=3, predicted=2, pmax=0.4111, P(gold)=0.3487, API confidence=0.43099029894539265.
  - R04: gold=3, predicted=2, pmax=0.3438, P(gold)=0.3269, API confidence=0.2838070067405807.
  - R02: gold=1, predicted=3, pmax=0.4321, P(gold)=0.0979, API confidence=0.43106731934286635.
  - Selective thresholds: {"0.8": {"covered": 1, "correct": 1, "accuracy": 1.0}, "0.9": {"covered": 0, "correct": 0, "accuracy": null}, "0.95": {"covered": 0, "correct": 0, "accuracy": null}}
- needs_subagents: 21/24; 95% Wilson interval 0.690–0.957; confusion {"False -> False": 20, "True -> False": 3, "True -> True": 1}.
  - C11: gold=True, predicted=False, pmax=0.7115, P(gold)=0.2885, API confidence=None.
  - C10: gold=True, predicted=False, pmax=0.6438, P(gold)=0.3562, API confidence=None.
  - C16: gold=True, predicted=False, pmax=0.6810, P(gold)=0.3190, API confidence=None.
  - Selective thresholds: {"0.8": {"covered": 20, "correct": 20, "accuracy": 1.0}, "0.9": {"covered": 17, "correct": 17, "accuracy": 1.0}, "0.95": {"covered": 10, "correct": 10, "accuracy": 1.0}}
- target_profile: 18/24; 95% Wilson interval 0.551–0.880; confusion {"code -> code": 5, "research -> research": 4, "research -> visual": 1, "sol -> sol": 5, "visual -> research": 1, "visual -> visual": 2, "security -> security": 2, "sol -> code": 3, "sol -> research": 1}.
  - R06: gold=research, predicted=visual, pmax=0.8703, P(gold)=0.1146, API confidence=0.8378664587701092.
  - R03: gold=visual, predicted=research, pmax=0.5412, P(gold)=0.2450, API confidence=0.42654726789946384.
  - C09: gold=sol, predicted=code, pmax=0.8235, P(gold)=0.1209, API confidence=0.7794155488796737.
  - C01: gold=sol, predicted=code, pmax=0.7802, P(gold)=0.1751, API confidence=0.7252550082714805.
  - R04: gold=sol, predicted=research, pmax=0.4809, P(gold)=0.1144, API confidence=0.3510956956539516.
  - C02: gold=sol, predicted=code, pmax=0.5782, P(gold)=0.3493, API confidence=0.47272541040420846.
  - Selective thresholds: {"0.8": {"covered": 12, "correct": 10, "accuracy": 0.8333333333333334}, "0.9": {"covered": 4, "correct": 4, "accuracy": 1.0}, "0.95": {"covered": 3, "correct": 3, "accuracy": 1.0}}
- Complexity argmax MAE 0.3333; expected-score MAE 0.5232; within one level 0.958.
- Controlled vs real-session accuracy: {"controlled": {"needs_clarification": 0.8125, "complexity": 0.8125, "needs_subagents": 0.8125, "target_profile": 0.8125}, "real_session": {"needs_clarification": 0.5, "complexity": 0.5, "needs_subagents": 1.0, "target_profile": 0.625}}.

## Validation and diagnostics

Exact HTTP-status expectations passed 26/28. Failed expectations are not hidden:
- 0.8b-v3 chat_not_supported: expected 404, got 405; response {"error": {"code": "method_not_allowed", "message": "Method Not Allowed"}}.
- 2b-v3 chat_not_supported: expected 404, got 405; response {"error": {"code": "method_not_allowed", "message": "Method Not Allowed"}}.
- 0.8b-v3: `{"single_vs_bundle": {"requests": 16, "answer_comparisons": 16, "max_absolute_probability_delta": 0.0, "mean_absolute_max_probability_delta": 0.0, "prediction_flips": [], "wall_ms_mean": 115.9235078130223}, "repeat": {"requests": 12, "answer_comparisons": 48, "max_absolute_probability_delta": 0.0, "mean_absolute_max_probability_delta": 0.0, "prediction_flips": [], "wall_ms_mean": 454.0333264145981}, "reversed_profile_order": {"requests": 4, "answer_comparisons": 4, "max_absolute_probability_delta": 0.09750033427622179, "mean_absolute_max_probability_delta": 0.06703525098367913, "prediction_flips": [], "wall_ms_mean": 134.88978124951245}}`
- 2b-v3: `{"single_vs_bundle": {"requests": 16, "answer_comparisons": 16, "max_absolute_probability_delta": 0.0, "mean_absolute_max_probability_delta": 0.0, "prediction_flips": [], "wall_ms_mean": 131.41589837323409}, "repeat": {"requests": 12, "answer_comparisons": 48, "max_absolute_probability_delta": 0.0, "mean_absolute_max_probability_delta": 0.0, "prediction_flips": [], "wall_ms_mean": 475.64194441777846}, "reversed_profile_order": {"requests": 4, "answer_comparisons": 4, "max_absolute_probability_delta": 0.06915160120640484, "mean_absolute_max_probability_delta": 0.031016122179184452, "prediction_flips": [], "wall_ms_mean": 136.3425207491673}}`

Model-selector observations:
- 0.8b-v3: request model=nonexistent-model-name returned HTTP 200 and model=jev-style-0.8b-decision-v3. The server does not use this field to select or reject model identities.
- 2b-v3: request model=nonexistent-model-name returned HTTP 200 and model=jev-style-2b-decision-v3. The server does not use this field to select or reject model identities.

Ollama:
```json
{
  "list_exit_code": 0,
  "list_stdout": "NAME    ID    SIZE    MODIFIED \n",
  "list_stderr": "",
  "tags": {
    "http_status": 200,
    "wall_ms": 3737.314833997516,
    "response": {
      "models": []
    }
  },
  "version": {
    "http_status": 200,
    "wall_ms": 0.4775419947691262,
    "response": {
      "version": "0.40.8"
    }
  },
  "chat_probe": {
    "http_status": 404,
    "wall_ms": 1.4642909955000505,
    "response": {
      "error": {
        "message": "model 'jev-style' not found",
        "type": "not_found_error",
        "param": null,
        "code": null
      }
    }
  }
}
```

Run errors / recovery:
```json
[
  {
    "type": "RuntimeError",
    "message": "Port 8765 occupied; refusing to replace an existing service"
  }
]
```
The first service was stopped successfully. The preflight bind check then rejected immediate reuse of port 8765 even though no listener remained (consistent with TCP TIME_WAIT). The 2B run resumed on free loopback port 8766, preserving all 0.8B results. No existing listener was killed.

## Supplemental clarification-only experiment

This follow-up was designed after observing primary clarification false negatives. It reused all frozen labels. It is exploratory and must be validated on new prompts. Each request contained only one noul question; latency is therefore not comparable to a full four-question routing call.

| Model | Wording | Accuracy | TP / FP / TN / FN | Must-clarify recall | Brier | ECE5 | Mean wall ms |
|---|---|---:|---|---:|---:|---:|---:|
| 0.8b-v3 | D_reference | 17/24 (70.8%) | 1 / 0 / 16 / 7 | 12.5% | 0.2248 | 0.1236 | 78.7 |
| 0.8b-v3 | E_missing_details | 19/24 (79.2%) | 4 / 1 / 15 / 4 | 50.0% | 0.1709 | 0.0775 | 64.7 |
| 0.8b-v3 | F_actionable_inverse | 19/24 (79.2%) | 4 / 1 / 15 / 4 | 50.0% | 0.1659 | 0.1290 | 60.9 |
| 0.8b-v3 | G_structured_examples | 16/24 (66.7%) | 0 / 0 / 16 / 8 | 0.0% | 0.1726 | 0.1332 | 74.5 |
| 2b-v3 | D_reference | 17/24 (70.8%) | 1 / 0 / 16 / 7 | 12.5% | 0.1410 | 0.1511 | 149.5 |
| 2b-v3 | E_missing_details | 18/24 (75.0%) | 3 / 1 / 15 / 5 | 37.5% | 0.1461 | 0.0719 | 99.0 |
| 2b-v3 | F_actionable_inverse | 22/24 (91.7%) | 8 / 2 / 14 / 0 | 100.0% | 0.1138 | 0.1751 | 99.9 |
| 2b-v3 | G_structured_examples | 15/24 (62.5%) | 2 / 3 / 13 / 6 | 25.0% | 0.1882 | 0.1685 | 151.0 |

- E asks whether essential details / previous conversation are missing.
- F asks whether the message is actionable as written, returning P(actionable); clarification is exactly `1 - P(actionable)`, threshold 0.5.
- G uses structured instructions with labeled examples. More instructions/examples did not consistently improve performance.
- D reference singletons matched primary bundled clarification probabilities exactly on every one of the 24 cases for both models.
- F 2B false positives: R04 (concrete stuck-coin symptom, P(clarify)=0.813052) and R07 (Jev routing research request, P(clarify)=0.525733). No false negatives on this suite.
- F 2B controlled ambiguous prompts: C01 fix-the-bug 0.746375; C02 faster 0.882448; C03 auth-refactor 0.713532; C04 API-cache 0.633886. Clear C05 exact display change 0.062255; C06 exact test command 0.116279.
- Primary clarification majority-class accuracy is 16/24 = 66.7%; D's 70.8% hides only 12.5% positive recall. Always-no-subagents already scores 20/24 = 83.3%, so the four positive delegation cases matter more than headline accuracy.

### Interpretation for Design B

- Keep Jev as advisory triage, not permission enforcement or an unchecked execution gate.
- Inverse actionability with 2B is the strongest clarification candidate here; direct needs_clarification wording is not dependable. Prefer an unnecessary clarification or lead-agent fallback over acting on an unresolved request.
- Route small explicit edits to code and substantial debugging/refactors to sol using explicit profile descriptions; bare names perform poorly. Keep uncertain role choices with the lead agent.
- Treat complexity as a rough ordinal feature, not a precise estimate of worker count. D exact accuracy was 16/24 (0.8B) and 17/24 (2B).
- Do not infer sub-agent execution readiness from complexity alone. 0.8B D classified all four delegation positives correctly; 2B D missed three. This tiny result does not establish a reliable decomposition policy.
- Use a new held-out set with more ambiguous positives, independent annotators, original-conversation/context variants, profile-boundary cases, and adversarial/tool-permission cases before deployment.

## Final integrity and cleanup

- 192 primary requests (768 decisions), 192 supplemental requests (192 decisions), and 64 diagnostic requests were recorded. The primary/supplemental request sets contain no duplicate keys or missing cases.
- Frozen case labels, primary variants, methodology, and supplemental inputs match their recorded SHA256 values. All 384 primary/supplemental requests returned 200; all primary probability sums passed validation; summary accuracy was independently recomputed.
- Started service PIDs are absent and loopback connection checks found no listener on 8765, 8766, 8767, 8768, or 11434. See verification.json.
- All benchmark writes are under /tmp. Git was already dirty; before/after status and tracked diff hashes differ in unrelated paths during the run. No claim of global repository cleanliness is made, and no user changes were reverted. Before/after snapshots are included.
- The first run's port-reuse preflight failure and both 405-vs-404 expectation mismatches remain in the logs/ledger; they were not silently discarded.
- Supplemental artifacts: supplemental-inputs.json, supplemental-inputs.sha256, supplemental-results.jsonl, supplemental-per-case.csv, supplemental-summary.json, supplemental-services.json.

## Reproducibility and limitations

- See methodology.json, frozen-inputs.sha256.json, cases.json, variants.json, run_benchmark.py, analyze.py, results.jsonl, diagnostics.jsonl, per-case.csv, summary.json, run-ledger.json, server logs, and sidecar-node-tests.tap.
- This is a small, manually labeled convenience sample, not independent calibration or production-safety validation. Ambiguous-task complexity and close role boundaries are subjective; R04/R05/R06 are marked borderline.
- Real session samples deliberately omit their original conversation, which can change whether clarification is needed. No repository content is provided to the classifier.
- Profile definitions reflect this machine's Smith configuration. The classifier does not know files, permission state, available tools, agent capabilities, or the true dependency graph from the prompt alone.
- Four questions were bundled; singleton diagnostics quantify any batching dependence. Latencies are single-client sequential loopback wall times after a warmup, with no concurrency or sustained-load testing. Model runs were sequential, so cross-model timing can have thermal/load confounds.
- Primary variant/case ordering is deterministically randomized with seed 20261023. Diagnostic repeats and the initial smoke are excluded from headline accuracy/latency.
- No optional GGUF scorer or Ollama Jev model was present in the examined paths, so no head-to-head MLX/GGUF or v1/v2 letter-readout comparison is claimed.
- Sub-agent positive labels are only 4/24. A good score here does not validate decomposition, child execution, permission enforcement, or cancellation.
- Host authorization and fallback must remain outside this learned router. A model probability is neither a permission grant nor an independently validated correctness probability.


## Permanent supplement: all primary binary confusion matrices

These are independently copied from `summary.json`, with positive classes
needs-clarification and needs-subagents. No failure rows were discarded.

| Model | Variant | Clarification TP / FP / TN / FN | Sub-agents TP / FP / TN / FN |
|---|---|---|---|
| 0.8b-v3 | A_terse | 5 / 10 / 6 / 3 | 3 / 1 / 19 / 1 |
| 0.8b-v3 | B_instructions | 4 / 5 / 11 / 4 | 4 / 0 / 20 / 0 |
| 0.8b-v3 | C_criteria | 1 / 0 / 16 / 7 | 4 / 0 / 20 / 0 |
| 0.8b-v3 | D_both | 1 / 0 / 16 / 7 | 4 / 0 / 20 / 0 |
| 2b-v3 | A_terse | 0 / 0 / 16 / 8 | 3 / 1 / 19 / 1 |
| 2b-v3 | B_instructions | 2 / 0 / 16 / 6 | 1 / 0 / 20 / 3 |
| 2b-v3 | C_criteria | 4 / 1 / 15 / 4 | 3 / 0 / 20 / 1 |
| 2b-v3 | D_both | 1 / 0 / 16 / 7 | 1 / 0 / 20 / 3 |

## Permanent supplement: inverse-actionability probabilities for every case

Gold labels are those frozen before the primary experiment. P(clarify) is exactly
`1 - P(is_actionable)` from F_actionable_inverse, with threshold 0.5. The original
report above contains the actual prompt text and marked borderline annotations.
These rows are not a new combined four-question pipeline measurement.

| Case | Gold clarification | 0.8B P(clarify) | 2B P(clarify) |
|---|---|---:|---:|
| C01 | true | 0.335554999 | 0.746375112 |
| C02 | true | 0.658501430 | 0.882448072 |
| C03 | true | 0.534838761 | 0.713532332 |
| C04 | true | 0.646508808 | 0.633885820 |
| C05 | false | 0.284795421 | 0.062255005 |
| C06 | false | 0.176480682 | 0.116278752 |
| C07 | false | 0.224104253 | 0.085633564 |
| C08 | false | 0.274502605 | 0.084261350 |
| C09 | false | 0.207603260 | 0.148371177 |
| C10 | false | 0.423726205 | 0.300739369 |
| C11 | false | 0.376254447 | 0.451313602 |
| C12 | false | 0.290026428 | 0.272016290 |
| C13 | false | 0.257225668 | 0.173812135 |
| C14 | false | 0.182104773 | 0.164170005 |
| C15 | false | 0.212909099 | 0.157107472 |
| C16 | false | 0.409772645 | 0.472644956 |
| R01 | true | 0.452184958 | 0.517347221 |
| R02 | true | 0.482015937 | 0.829514084 |
| R03 | true | 0.566903853 | 0.748172654 |
| R04 | false | 0.500825042 | 0.813051593 |
| R05 | false | 0.385139945 | 0.417274377 |
| R06 | true | 0.232387332 | 0.540824151 |
| R07 | false | 0.298132917 | 0.525732655 |
| R08 | false | 0.499250769 | 0.247996563 |

## Permanent supplement: captured full probability distributions

The following are actual primary observations, not illustrative contract values.
C11 is the explicit Design B architecture request; it demonstrates that correctly
predicting high complexity does not imply correctly predicting sub-agent need.
C05 is the exact display-string edit. Full response numbers are retained here so
score-index and confidence calculations are independently reproducible.

### C11, 0.8b-v3, D_both

```json
{
  "phase": "primary",
  "release": "0.8b-v3",
  "variant": "D_both",
  "case_id": "C11",
  "source": "controlled",
  "gold": {
    "needs_clarification": false,
    "complexity": 5,
    "needs_subagents": true,
    "target_profile": "sol"
  },
  "http_status": 200,
  "wall_ms": 727.5549999903888,
  "response": {
    "model": "jev-style-0.8b-decision-v3",
    "answers": {
      "needs_clarification": {
        "type": "noul",
        "noul": 0.2195460503731297
      },
      "complexity": {
        "type": "score",
        "score": 3.1189603454826584,
        "confidence": 0.2658002879022151,
        "legend": {
          "0": "1_trivial",
          "1": "2_simple_edit",
          "2": "3_moderate_feature",
          "3": "4_complex_refactor",
          "4": "5_multi_agent_architecture"
        },
        "probabilities": {
          "0": 0.021659371757650172,
          "1": 0.08742067118875804,
          "2": 0.19298366621482427,
          "3": 0.14617282149081845,
          "4": 0.5517634693479492
        }
      },
      "needs_subagents": {
        "type": "noul",
        "noul": 0.620760305742622
      },
      "target_profile": {
        "type": "choice",
        "choice": "sol",
        "confidence": 0.4014991536747007,
        "probabilities": {
          "code": 0.3239677522857724,
          "sol": 0.5211993229397606,
          "research": 0.07848271366001223,
          "visual": 0.009562758646461084,
          "security": 0.06678745246799367
        }
      }
    },
    "usage": {
      "input_tokens": 1224,
      "state_tokens": 67,
      "output_tokens": 0
    },
    "latency_ms": 725.7,
    "timing": {
      "total_ms": 725.7
    },
    "backend": "mlx"
  }
}
```

### C11, 2b-v3, D_both

```json
{
  "phase": "primary",
  "release": "2b-v3",
  "variant": "D_both",
  "case_id": "C11",
  "source": "controlled",
  "gold": {
    "needs_clarification": false,
    "complexity": 5,
    "needs_subagents": true,
    "target_profile": "sol"
  },
  "http_status": 200,
  "wall_ms": 498.92595800338313,
  "response": {
    "model": "jev-style-2b-decision-v3",
    "answers": {
      "needs_clarification": {
        "type": "noul",
        "noul": 0.32087300637036714
      },
      "complexity": {
        "type": "score",
        "score": 3.257994070969864,
        "confidence": 0.3816617258082201,
        "legend": {
          "0": "1_trivial",
          "1": "2_simple_edit",
          "2": "3_moderate_feature",
          "3": "4_complex_refactor",
          "4": "5_multi_agent_architecture"
        },
        "probabilities": {
          "0": 0.013857373213049761,
          "1": 0.053320246156348255,
          "2": 0.1761274113032131,
          "3": 0.17436087510246587,
          "4": 0.582334094224923
        }
      },
      "needs_subagents": {
        "type": "noul",
        "noul": 0.2884623481576154
      },
      "target_profile": {
        "type": "choice",
        "choice": "sol",
        "confidence": 0.6777620925803348,
        "probabilities": {
          "code": 0.18617187077202327,
          "sol": 0.7422096740642679,
          "research": 0.03690143700159741,
          "visual": 0.0037458190143840214,
          "security": 0.03097119914772745
        }
      }
    },
    "usage": {
      "input_tokens": 873,
      "state_tokens": 67,
      "output_tokens": 0
    },
    "latency_ms": 496.6,
    "timing": {
      "total_ms": 496.6
    },
    "backend": "mlx"
  }
}
```

### C05, 0.8b-v3, D_both

```json
{
  "phase": "primary",
  "release": "0.8b-v3",
  "variant": "D_both",
  "case_id": "C05",
  "source": "controlled",
  "gold": {
    "needs_clarification": false,
    "complexity": 1,
    "needs_subagents": false,
    "target_profile": "code"
  },
  "http_status": 200,
  "wall_ms": 889.9919999967096,
  "response": {
    "model": "jev-style-0.8b-decision-v3",
    "answers": {
      "needs_clarification": {
        "type": "noul",
        "noul": 0.12247027302947647
      },
      "complexity": {
        "type": "score",
        "score": 0.9223680956547533,
        "confidence": 0.23135992028770558,
        "legend": {
          "0": "1_trivial",
          "1": "2_simple_edit",
          "2": "3_moderate_feature",
          "3": "4_complex_refactor",
          "4": "5_multi_agent_architecture"
        },
        "probabilities": {
          "0": 0.4539374238368571,
          "1": 0.32933258399863163,
          "2": 0.08718865832281784,
          "3": 0.09950714035628788,
          "4": 0.03003419348540557
        }
      },
      "needs_subagents": {
        "type": "noul",
        "noul": 0.1029626385276222
      },
      "target_profile": {
        "type": "choice",
        "choice": "code",
        "confidence": 0.5689500810319135,
        "probabilities": {
          "code": 0.6551600648255308,
          "sol": 0.19653858216565329,
          "research": 0.11825657525275353,
          "visual": 0.013760073606480221,
          "security": 0.016284704149582132
        }
      }
    },
    "usage": {
      "input_tokens": 1199,
      "state_tokens": 42,
      "output_tokens": 0
    },
    "latency_ms": 888.1,
    "timing": {
      "total_ms": 888.1
    },
    "backend": "mlx"
  }
}
```

### C05, 2b-v3, D_both

```json
{
  "phase": "primary",
  "release": "2b-v3",
  "variant": "D_both",
  "case_id": "C05",
  "source": "controlled",
  "gold": {
    "needs_clarification": false,
    "complexity": 1,
    "needs_subagents": false,
    "target_profile": "code"
  },
  "http_status": 200,
  "wall_ms": 513.2917080045445,
  "response": {
    "model": "jev-style-2b-decision-v3",
    "answers": {
      "needs_clarification": {
        "type": "noul",
        "noul": 0.05242184728297201
      },
      "complexity": {
        "type": "score",
        "score": 0.5172076618270469,
        "confidence": 0.5689936151441275,
        "legend": {
          "0": "1_trivial",
          "1": "2_simple_edit",
          "2": "3_moderate_feature",
          "3": "4_complex_refactor",
          "4": "5_multi_agent_architecture"
        },
        "probabilities": {
          "0": 0.5768516325374325,
          "1": 0.358207500560488,
          "2": 0.0418870636641623,
          "3": 0.01698917901343444,
          "4": 0.00606462422448276
        }
      },
      "needs_subagents": {
        "type": "noul",
        "noul": 0.0873040002497792
      },
      "target_profile": {
        "type": "choice",
        "choice": "code",
        "confidence": 0.9394957528699965,
        "probabilities": {
          "code": 0.9515966022959972,
          "sol": 0.025878007502172576,
          "research": 0.015280389732307836,
          "visual": 0.004089461387668206,
          "security": 0.0031555390818541955
        }
      }
    },
    "usage": {
      "input_tokens": 848,
      "state_tokens": 42,
      "output_tokens": 0
    },
    "latency_ms": 511.4,
    "timing": {
      "total_ms": 511.4
    },
    "backend": "mlx"
  }
}
```

## Permanent supplement: exact inverse-actionability question

The successful isolated follow-up asked the following noul question. Its raw true
answer means actionable, not clarify; the adapter must explicitly complement it.

```json
{
  "type": "noul",
  "instructions": "Is this user request sufficiently specific to begin useful task-specific work without asking the user a clarifying question? Judge only this message; you may inspect the repository but have no previous conversation.",
  "criteria": {
    "true": "Clear and actionable: an exact edit or test command, a concrete symptom to investigate, or a research topic with an objective.",
    "false": "Underspecified: a missing target or outcome, an unresolved it/this, or vague fix/refactor/cache/optimization request without necessary scope."
  }
}
```

## Interpretation boundaries for this proposal

- Primary D clarification for each model is TP=1, FP=0, TN=16, FN=7; its 17/24
  accuracy must not be cited without the 1/8 recall and 16/24 majority baseline.
- F inverse 2B is TP=8, FP=2, TN=14, FN=0: accuracy 22/24, recall 8/8,
  specificity 14/16. The two false positives are R04 and R07. No threshold was
  validated on a held-out set.
- 18/24 descriptive-profile versus 10/24 bare-profile is the 0.8B D versus A
  contrast. 2B A is 13/24; 2B B/C/D each achieved 18/24.
- Complexity ±1 accuracy is 23/24 for both D variants, but exact accuracy is
  16/24 (0.8B) and 17/24 (2B). The raw `score` is zero-based expectation.
- The 0.8B sub-agent score of 24/24 has four positives and 20 negatives; an
  always-false classifier is already 20/24. 2B D has three false negatives.
- Normalized API choice confidence, winning probability, and ordinal score
  confidence are different statistics. The original selectivity table uses
  winning probability, not the proposed normalized confidence threshold.
- The proposed combined inverse + other-head request has not been benchmarked
  end to end. Identical primary D singleton/bundle observations are encouraging,
  not a production guarantee for different question sets or models.
- The Node harness's four passing tests include a fake-engine positive test.
  Real inference evidence is the separate MLX probe and benchmark requests.
- 26/28 exact HTTP-status expectations passed; the two unsupported chat probes
  returned 405 instead of expected 404. A nonexistent requested model name was
  ignored by the local endpoint, so request model strings are not identity proof.

## Retained artifact fingerprints

The temporary artifacts contain the raw 192-row primary and 192-row follow-up
responses and scripts; the report, exact inverse wording, selected full responses,
and tables above are embedded so that the Stage-1 decision survives `/tmp` cleanup.
The following fingerprints identify the original evidence without claiming all
raw files have been checked into this repository:

| Artifact | SHA256 |
|---|---|
| `report.md` | `6c6b0f7c26edeefd0aeb1556718518faf2a08cd4772da841c04ed077eab3f1f5` |
| `cases.json` | `12a9c4bcdc46088f8863852a699a684f19448c5c7af13b4705621614fcae2c94` |
| `variants.json` | `e5c4e43c10232b9a138ebb536233aba42f41f663942b5e95dab033d31e3d7588` |
| `methodology.json` | `07ba9f4334251216277f515fc4e82ca9b49633f6f0a4b7e46780825524aef995` |
| `frozen-inputs.sha256.json` | `5f7180622f500f8992133ccf3f2e6580e579ebf2f386da5a1835fbc78062b4d3` |
| `results.jsonl` | `1a260817d479a3c5f21b86a4933f8361d0ce2f480af34cd60bde3c7145bd3d96` |
| `diagnostics.jsonl` | `4a3e06bd8c82bc40c83bf3327666162512e4873f57b43c112cc51c4a4ee83a9f` |
| `summary.json` | `c5fda4feef581be0773d485ed617eece112226490b89c16d407a2deb4968f529` |
| `supplemental-inputs.json` | `e8d2d98ebf920b3cf3fb873bb7177de5678f77f535581184983a8626d00acb24` |
| `supplemental-results.jsonl` | `0662b5a80f8b61f915afdab4b322f99776287eead61fc18e3164fc4e22a3efad` |
| `supplemental-summary.json` | `3544e3511783aff590bf7369b06816d3cdac829e4db7502324ef48e232e0ec82` |
| `verification.json` | `6a7f2c6aa0dcb0484e895ff6c8b3390c202bb626b848f3d45ddd8abb47dd3aea` |
| `sidecar-node-tests.tap` | `a3f885188b175529c4b18ab01e52ee910d82e520c94a7aba6ac981c98ede8a00` |
