# Continuous integration

Smith's CI runs the complete workspace and Agent Runtime consumer-conformance
suite on:

- Linux with Rust 1.88;
- macOS with Rust 1.88; and
- Linux with current stable Rust plus dependency policy checks.

The workflow reads the exact Agent Runtime Git repository and revision from
`Cargo.lock`, checks out that immutable source, and rejects a missing,
ambiguous, different, or dirty runtime checkout before running Smith. If the
runtime repository becomes private, add `AGENT_RUNTIME_TOKEN` as a repository
secret with read access to it.

Committed manifests use the same immutable Git revision. Local co-development
may replace it through the git-ignored `.cargo/config.toml` patch table, so a
sibling checkout never leaks into a release.

Run the same gates locally from the Smith workspace:

```sh
RUNTIME_SHA="$(git -C ../agent-runtime rev-parse HEAD)"
SMITH_AGENT_RUNTIME_REVISION="$RUNTIME_SHA" bash scripts/ci.sh
cargo deny --locked check all
```

Set `SMITH_AGENT_RUNTIME_DIR` when the runtime is not at
`../agent-runtime`. Omitting `SMITH_AGENT_RUNTIME_REVISION` is convenient while
iterating, but release evidence must set it so the script verifies an exact
clean runtime commit.
