//! Versioned WASM component engine, independent of configuration and clients.

#[allow(missing_docs, unreachable_pub)]
pub(crate) mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "module",
        additional_derives: [serde::Serialize, serde::Deserialize],
    });
}

mod adapter;
mod bounds;
mod broker;
mod engine;
mod limits;
mod projection;
pub use adapter::{COMPONENT_CACHE_DIRECTORY, CapabilityGrant, WasmModule, WasmModuleConfig};
pub use engine::WasmEngine;
pub use limits::{WasmError, WasmLimits};

// Generated with `wasm-tools parse tests/fixtures/diagnostic.wat`; the engine
// consumes binary bytes, so no production WAT parser is needed.
const DIAGNOSTIC_COMPONENT: &[u8] = include_bytes!("../tests/fixtures/diagnostic.wasm");

fn diagnostic_engine() -> wasmtime::Result<wasmtime::Engine> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true).epoch_interruption(true);
    wasmtime::Engine::new(&config)
}

/// Exercises component compilation and instantiation for build diagnostics.
/// Keeping this reachable from the CLI includes the engine in size measurements.
pub fn diagnostic() -> wasmtime::Result<&'static str> {
    let engine = diagnostic_engine()?;
    let component = wasmtime::component::Component::new(&engine, DIAGNOSTIC_COMPONENT)?;
    let linker = wasmtime::component::Linker::<()>::new(&engine);
    let mut store = wasmtime::Store::new(&engine, ());
    store.set_epoch_deadline(1);
    let instance = linker.instantiate(&mut store, &component)?;
    let answer = instance.get_typed_func::<(), (u32,)>(&mut store, "answer")?;
    let (value,) = answer.call(&mut store, ())?;
    answer.post_return(&mut store)?;
    if value != 42 {
        return Err(wasmtime::Error::msg(
            "unexpected component diagnostic result",
        ));
    }
    Ok("wasmtime 36.0.17; smith:module@0.1.0; component-model; epoch interruption")
}

#[cfg(test)]
mod tests {
    #[test]
    fn compiles_and_instantiates_embedded_component() {
        assert!(super::diagnostic().unwrap().contains("smith:module@0.1.0"));
    }
}
