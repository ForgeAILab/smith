# Continuous integration

Smith's CI runs the complete workspace and Agent Runtime consumer-conformance
suite on:

- Linux with Rust 1.88;
- macOS with Rust 1.88; and
- Linux with current stable Rust plus dependency policy checks.

The workflow deliberately has no guessed Agent Runtime URL. Configure these
GitHub repository variables before enabling required checks:

- `AGENT_RUNTIME_REPOSITORY`: `owner/repository`;
- `AGENT_RUNTIME_REVISION`: an exact lowercase 40-character commit SHA.

If the runtime repository is private, add `AGENT_RUNTIME_TOKEN` as a repository
secret with read access to that repository. The workflow rejects a missing,
abbreviated, different, or dirty runtime revision before running Smith.

This exact CI checkout is a co-development compatibility gate. It does not make
the current sibling `path` dependency releasable. Publication remains blocked
until Smith's manifests use an immutable released semantic version or Git
revision and the sibling checkout moves to an uncommitted local Cargo patch.

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
