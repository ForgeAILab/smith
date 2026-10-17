//! Rust guest SDK. Build guests for `wasm32-unknown-unknown`, then componentise.
//! No WASI imports are linked. Bindings use the host's WIT as their source.

/// Supported Smith interface package version.
pub const INTERFACE_VERSION: &str = "0.2.0";

#[cfg(target_arch = "wasm32")]
#[allow(missing_docs, unreachable_pub, clippy::too_many_arguments)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../smith-wasm-host/wit/v0.2",
        world: "module",
        pub_export_macro: true,
    });
}

/// Export an implementation of [`Module`] from a guest cdylib.
#[cfg(target_arch = "wasm32")]
#[macro_export]
macro_rules! export {
    ($guest:ident) => {
        // Canonical ABI glue flattens WIT records into up to 16 arguments.
        #[allow(clippy::too_many_arguments)]
        const _: () = {
            struct SmithSdkExport;
            impl $crate::bindings::Guest for SmithSdkExport {
                fn describe() -> $crate::bindings::smith::module::types::Descriptor {
                    // Keep a versioned Smith import even when the author's code
                    // uses no host services. Freestanding world exports do not
                    // otherwise retain the original package/version in a component.
                    let _ = $crate::host::log(
                        $crate::bindings::smith::module::types::LogLevel::Trace,
                        "SDK describe",
                    );
                    <$guest as $crate::Module>::describe()
                }
                fn mount(
                    ctx: $crate::bindings::smith::module::types::MountContext,
                ) -> Result<
                    $crate::bindings::smith::module::types::Mounted,
                    $crate::bindings::smith::module::types::ModuleError,
                > {
                    <$guest as $crate::Module>::mount(ctx)
                }
                fn call_tool(
                    id: String,
                    input: $crate::bindings::smith::module::types::ToolInput,
                ) -> Result<String, $crate::bindings::smith::module::types::ModuleError> {
                    <$guest as $crate::Module>::call_tool(id, input)
                }
                fn contribute_context(
                    id: String,
                    view: $crate::bindings::smith::module::types::ContextView,
                ) -> Result<
                    $crate::bindings::smith::module::types::ContextPatch,
                    $crate::bindings::smith::module::types::ModuleError,
                > {
                    <$guest as $crate::Module>::contribute_context(id, view)
                }
                fn process_tool_output(
                    id: String,
                    view: $crate::bindings::smith::module::types::ToolOutputView,
                ) -> Result<
                    $crate::bindings::smith::module::types::ToolOutputPatch,
                    $crate::bindings::smith::module::types::ModuleError,
                > {
                    <$guest as $crate::Module>::process_tool_output(id, view)
                }
                fn poll(
                    id: String,
                ) -> Result<
                    Vec<$crate::bindings::smith::module::types::Notice>,
                    $crate::bindings::smith::module::types::ModuleError,
                > {
                    <$guest as $crate::Module>::poll(id)
                }
                fn status(
                    id: String,
                ) -> Option<$crate::bindings::smith::module::types::StatusItem> {
                    <$guest as $crate::Module>::status(id)
                }
            }
    $crate::bindings::export!(SmithSdkExport with_types_in $crate::bindings);
        };
    };
}

#[cfg(target_arch = "wasm32")]
mod helpers;
/// Trait implemented by module authors.
#[cfg(target_arch = "wasm32")]
pub trait Module {
    /// Stable metadata, discovered before mounting.
    fn describe() -> bindings::smith::module::types::Descriptor;
    /// Declares this session's contributions.
    fn mount(
        ctx: bindings::smith::module::types::MountContext,
    ) -> Result<bindings::smith::module::types::Mounted, bindings::smith::module::types::ModuleError>;
    /// Handles a registered tool id.
    fn call_tool(
        id: String,
        input: bindings::smith::module::types::ToolInput,
    ) -> Result<String, bindings::smith::module::types::ModuleError>;
    /// Handles a registered context contributor id.
    fn contribute_context(
        id: String,
        view: bindings::smith::module::types::ContextView,
    ) -> Result<
        bindings::smith::module::types::ContextPatch,
        bindings::smith::module::types::ModuleError,
    >;
    /// Handles a registered output processor id.
    fn process_tool_output(
        id: String,
        view: bindings::smith::module::types::ToolOutputView,
    ) -> Result<
        bindings::smith::module::types::ToolOutputPatch,
        bindings::smith::module::types::ModuleError,
    >;
    /// Returns the current declaration for a registered status id.
    fn status(id: String) -> Option<bindings::smith::module::types::StatusItem>;
    /// Polls a watcher; modules without watchers need no implementation.
    fn poll(
        _id: String,
    ) -> Result<
        Vec<bindings::smith::module::types::Notice>,
        bindings::smith::module::types::ModuleError,
    > {
        Err(bindings::smith::module::types::ModuleError {
            kind: bindings::smith::module::types::ErrorKind::UnknownId,
            message: "unknown watcher".into(),
        })
    }
}
/// Typed, recoverable host function wrappers.
#[cfg(target_arch = "wasm32")]
pub use bindings::smith::module::host;
#[cfg(target_arch = "wasm32")]
pub use helpers::{
    CompletionRequestBuilder, ContextPatchBuilder, NoticeBuilder, StatusItemBuilder,
    ToolSpecBuilder, WatcherBuilder,
};
