# Verification: GPT-6.1 Sol release

## Source and model data

- Release code commit: `ed253a8c55c12eaf53276b7f472c8a38decd651d`.
- Direct ChatGPT and Codex model catalogs include `gpt-6.1-sol`.
- ChatGPT retains 272k default / 872k extended contexts and a 16,384-token
  output ceiling. The installed Codex catalog advertises those contexts.
- Models.dev seed regeneration matches the canonical downloaded source exactly.
- The seed supplies GPT-6.1 Sol limits, pricing, tool support, and the published
  OpenAI effort ladder without `none` or `minimal`.

## Automated checks

- Rust 1.88 formatting and Clippy with warnings denied passed.
- Workspace tests and the exact pinned runtime conformance suite passed:
  1,908 tests passed, zero failed; six explicit live/quota tests were ignored.
- `cargo deny --locked check all` passed.
- npm bootstrapper syntax, seven tests, package dry run, and portable Linux
  installer tests passed.
- Hosted Linux Rust 1.88, macOS Rust 1.88, stable Rust dependency policy, and
  npm checks passed:
  https://github.com/ForgeAILab/smith/actions/runs/36944242905
- The optimized local binary reports `smith 0.2.15`.

## Distribution checks

- Release workflow:
  https://github.com/ForgeAILab/smith/actions/runs/36944892414
- The downloaded Apple Silicon artifact reports `smith 0.2.15`, contains the
  new model ID, and accepts the GPT-6.1 Sol profile with both context choices.
- Apple Silicon archive SHA-256:
  `a1a2211741751a2966a7728f9445b38aa4d2b5e12623906bdc0cf4508ef0ccdc`.
- The published Apple Silicon archive matches the workflow artifact and the
  public `SHA256SUMS` manifest; the manifest names all six platform archives.
- The verified published binary is installed at `~/.local/bin/smith`, with the
  previous binary retained under `~/.local/share/smith/backups/`.
- The release workflow completed successfully, including both Linux musl
  smoke tests and npm trusted publication.
- Public release: https://github.com/ForgeAILab/smith/releases/tag/v0.2.15
- npm registry serves `@forgeailab/smith@0.2.15` after processing the accepted
  publication.
- The original main checkout was fast-forwarded with its pre-existing edits
  restored. All 20 original modified/untracked file contents were verified;
  only Smith's six release version fields changed in the existing lockfile.
- npm's `latest` dist-tag resolves to `0.2.15`.
- A clean npm cache launches bootstrapper `0.2.15`; its actual install command
  downloads the matching GitHub archive, verifies the checksum, and installs
  a binary reporting `smith 0.2.15` into a temporary test directory.
