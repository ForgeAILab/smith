# Ownership after extraction

The context-window and image-generation implementations were in place before
these moves. The public factory types, `preflight`, and ordered `build` entry
point remain in `smith-runtime/src/factory.rs`.

| Module | Responsibility |
| --- | --- |
| `factory/provider.rs` and `provider/{resolution,credentials,adapter,children}.rs` | Resolve model/provider input, lease credentials, construct adapters, and prepare child routes. |
| `factory/context_policy.rs` | Derive context windows, output budgets, and compaction policy. |
| `factory/authority.rs` | Resolve workspace and approval authority. |
| `factory/capabilities.rs` | Register built-in and extension tool capabilities. |
| `factory/persistence.rs` | Resolve checkpoint durability. |
| `factory/{resolve,compose,delegation}.rs` | Accept the harness and assemble the final runtime and root delegation stage. |
| `factory/tests.rs` | The preserved factory unit tests. |

The `App` type, fields, and root-session state methods remain in
`smith-tui/src/app/state.rs`. Child conversation folding, delegated usage,
presentation, and inspection methods live in `app/state/children.rs`. The
methods keep their original public paths through `impl App`.

These are code-ownership changes. The public API, validation order, runtime
behavior, and user-visible output remain unchanged.
