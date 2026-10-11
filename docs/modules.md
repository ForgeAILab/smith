# Compiled modules

A module is native Rust compiled into Smith, selected by configuration, and
mounted when the runtime is composed. Smith currently ships `image-generation`
and `budget-notice`. Default builds include both; `cargo build -p smith-cli
--no-default-features` produces a working binary with the core tools and no
optional modules.

## User switches

```toml
[modules.image-generation]
enabled = false
```

`/modules` lists every known module, including first-party modules omitted from
the build. Rows show the description, mount state and reason, deciding key and
configuration layer, and first-party or named third-party origin. `smith config
modules` prints the same report without starting a session.

`/modules image-generation on` or `off` previews an edit to the user config,
asks for confirmation, and rebuilds at the next safe boundary. A running turn
finishes with its existing composition. A failed rebuild rolls back the edit.
Project, profile, environment, command-line, and session values can override
the user value; Smith names the winning layer rather than claiming the switch
took effect. Unknown ids and attempts to enable a module absent from the build
are refused before writing.

The legacy `tools.image_generation.enabled` is the same switch. If the user
file already contains it, `/modules` edits it in place. If both spellings
exist, the edit keeps them aligned. Different values in the same layer are an
error; across layers the ordinary precedence decides. Other image settings
keep their existing keys. `budget-notice` defaults on and is inactive when
semantic summaries are disabled or the input budget cannot fit its threshold.

## Contract

Module crates depend on `smith-module`, Agent Runtime contracts, and optionally
`smith-tools`. They do not import Smith's runtime factory, configuration, CLI,
or terminal renderer.

Implement `smith_module::Module`: `id`, `revision`, `description`,
`default_enabled`, optional `requirements`, and
`mount(&ModuleContext) -> Result<Mounted, ModuleError>`. The context supplies
resolved plain settings, user directory, posture, HTTP transport, optional
provider image binding and canonical session-history service, semantic-summary
availability, input ceiling, and built-in-tool availability.

Mount returns `Mounted::Contributions` or an explainable `Mounted::Inactive`.
Contributions are shared `Tool` values, any of the six Agent Runtime harness
pipeline components, observe-only `EventObserver` values, slash-command
declarations, or bounded live status sources. Pipeline ordering belongs to
Agent Runtime. Command dispatch belongs to the host's single command registry;
a declaration alone does not install a new executor. Status sources return
plain `StatusItem` data and receive no renderer handle.

Mounting returns values rather than registering side effects. Requirements
are ordered before mounting; missing, off, failed, or inactive requirements
block dependents. Cycles and mount failures are reported without mounting
partial contributions. Rebuilding from the selected set removes an unmounted
module's tools, pipeline, observers, commands, status, and composition evidence.

## Add a first-party module

1. Create `crates/modules/<id>` with workspace package fields and lints. Add
   its member and versioned path dependency to the root `Cargo.toml`.
2. Implement `Module` using the context and shared contracts. Keep its id and
   revision stable and its description to one line.
3. Add an optional `smith-cli` dependency and exactly one `module-<id>` feature
   enabling it. Add that feature to `default`.
4. Add a feature-gated `CompiledModule` entry in
   `crates/smith-cli/src/modules.rs`, the sole compiled-in list. Add the
   feature-absent descriptor alongside the existing first-party descriptors.
5. The catalog supplies the known id and default to configuration. If porting
   an existing switch, declare its legacy alias in `known_modules()` and
   preserve its default. Pass any own settings in
   `composition_from_resolved_config`; add named context fields only when
   a port needs host facts. Wire a legacy user-edit patch when adding an alias.
6. Cover mounting, failure, inactivity, off/on rebuilding, provenance, and the
   listing. Run architecture checks and the default and minimal-build gates.

## Compile in a third-party crate

Add its versioned dependency and an explicit entry to `compiled_modules()`:

```rust
CompiledModule {
    module: Arc::new(example_module::ExampleModule),
    origin: ModuleOrigin::ThirdParty { crate_name: "example-module".into() },
}
```

The entry's descriptor automatically supplies its id, default, and description
to configuration and listings. Evidence records
`ModuleProvenance::CompiledThirdParty("example-module")` and
`ModuleTrust::TrustedNative`. This code is unsandboxed native code in the
binary: compiling it is the embedder's trust decision, and its syscalls are
not mediated. Smith does not load native modules at runtime. The unpublished
`crates/test-support/module-fixture` crate exercises this path only in tests.
