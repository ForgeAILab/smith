#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::resolve::load::{
        built_in_defaults, env_name, join_key, position, setting_for_env, unknown_field,
    };
    use crate::resolve::provider::{nearest, unquote_segment};

    include!("provenance.rs");
    include!("load.rs");
    include!("agent.rs");
    include!("provider.rs");
}
