# Three-stage image reconstruction benchmark

[Open the interactive comparison](comparison.html). Frozen model outputs, real Chrome screenshots, observations and raw event logs are included.

Each arm combines three horizontal screenshot parts into a dashboard, resumes to apply a revised screenshot, then resumes again to add a Bookings page while retaining the revised dashboard. Models are gpt-6.1-sol and gpt-6-astra at medium effort, through Smith and installed Codex CLI 0.163.0.

| Arm | Build SSIM | Revision SSIM | Bookings SSIM | Dashboard retained | Cached input |
|---|---:|---:|---:|---|---:|
| smith-sol | 0.826 | 0.836 | 0.836 | Yes | 13.18% |
| smith-astra | 0.823 | 0.845 | 0.850 | Yes | 20.70% |
| codex-sol | 0.833 | 0.844 | 0.850 | Yes | 91.30% |
| codex-astra | 0.833 | 0.836 | 0.851 | Yes | 92.64% |

## Evidence and limits

- [evaluation.json](evaluation.json) separates visible text, image identifiers, observations, revised values, superseded content, navigation, filters, booking forms, browser errors and mobile overflow.
- [image-history-audit.json](image-history-audit.json) hashes persisted inputs, checks source-byte equality and verifies all six supplied images remain in the correct order. It is not a capture of every outgoing native request.
- [input-parity.json](input-parity.json) checks initial prompt and image hashes across arms. Recovery prompts add no visual information or grading feedback.
- [runs.json](runs.json) includes provider-reported cached and uncached input. Timed attempts and recoveries are combined; any unreported usage from interrupted requests remains unknown.
- Smith uses the frozen binary with two native serialization fixes described in the [cache investigation](../cache-prefix-investigation-2026-10-23/REPORT.md). The remaining miss pattern also reproduces without images.
- Initial stage limits were 600 seconds. Smith/Sol stage 1 hit a provider stream timeout and was recovered in the same session. Smith/Astra stage 3 reached the wall-clock limit and was recovered from its existing images and observations. Those initial attempts are preserved in `*-initial-*` files. Smith/Sol stage 2 wrote the correct revision but reached its limit before a final answer; its original stage-3 prompt then continued the same session with storage copied to `/private/tmp` and a 900-second limit. Stage 3 also reached its limit after writing both pages. External browser checks passed on the produced pages. Thus 10/12 logical stages received a terminal model answer, while all 16 requested page snapshots were produced and rendered. See `storage-correction.json`.
- Elapsed times are confounded by concurrent execution and storage: Smith sessions were under `/Volumes/Data/tmp`, while Codex used its default session directory. A [passive sample](profiler-smith-sol-stage-2.txt) caught Smith waiting in `sync_all`. This is not a model-speed ranking. Future timing runs must place all state explicitly on the same local volume.
- This is one attempt per arm with documented interruption recoveries, not a statistical model ranking. Pixel SSIM is sensitive to typography and antialiasing and is separate from image comprehension.
- Six supplied image attachments across three stages do not reproduce the original 192-image session or its image-window eviction.

The model outputs are unmodified. Browser checks operate on frozen copies; no evaluator feedback was sent to the models. Small decorations are reconstructed in CSS/SVG, not taken from the reference images.
