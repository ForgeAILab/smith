//! The explicit build catalog and temporary default-based module selection.

use smith_module::{CompiledModule, ModuleComposition};

/// The single compiled-in list. First-party entries gain feature gates as ported.
fn compiled_modules() -> Vec<CompiledModule> {
    Vec::new()
}

pub(super) fn composition() -> ModuleComposition {
    // Ports add descriptors for first-party modules omitted by cargo features
    // here, beside the sole implementation list, before layered selection lands.
    ModuleComposition::with_defaults(compiled_modules(), Vec::new())
}
