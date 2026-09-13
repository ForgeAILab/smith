# Tasks: Fix installed coding-agent selection and honest candidate previews

## 1. Inventory enumerates installed agents (smith-config)

- [x] 1.1 Add `ModelLimitOrigin::BuiltIn` and fall back to the CLI bookkeeping
      constants in `inventory_limit` for `cli/...` ids with no explicit,
      trusted, or catalog limit.
- [x] 1.2 Test: a profile selecting `cli/<kind>/<model>` with no `[models]`
      declaration enumerates and is selectable.
- [x] 1.3 Test: explicit `[models."provider/cli/..."]` limits override the
      built-ins and keep configured provenance.

## 2. Honest candidate preview budgets (smith-config)

- [x] 2.1 Stop carrying profile-scoped request-output and reserve values from
      the active profile across candidate previews; derive per-candidate
      automatic budgets instead.
- [x] 2.2 Test: active profile request 32,768 no longer disables a candidate
      whose ceiling is 32,000; the preview shows the candidate's automatic
      budget.
- [x] 2.3 Test: a persistent (user-global/environment/flag/session) explicit
      request that exceeds a candidate ceiling still disables it with the
      bounded reason.
- [x] 2.4 Test: the active candidate's effective request and reserve are
      unchanged by previewing others.

## 3. Selection surfaces render installed agents once (smith-cli)

- [x] 3.1 Skip provider-qualified display rows whose model parses as a CLI
      id; keep the curated `CLI_AGENTS` rows with the PATH check.
- [x] 3.2 Test: model rendering contains each installed-agent id exactly
      once, and profile rows for installed agents are not unavailable.

## 4. Validation

- [x] 4.1 Full workspace `cargo test`, fmt, and clippy.
- [x] 4.2 Live verification: remove the hand-written
      `[models."google/cli/..."]` workaround tables from the user
      configuration, rebuild, and confirm `cc`/`cx` resolve and the
      resource-rendering path keeps them selectable.
