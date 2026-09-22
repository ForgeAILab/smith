#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::resolve::load::{
        apply_image_generation_defaults, built_in_defaults, env_name, join_key, position,
        setting_for_env, unknown_field,
    };
    use crate::resolve::provider::{nearest, unquote_segment};
    use crate::resolve::provenance::Contribution;

    include!("provenance.rs");
    include!("load.rs");
    include!("agent.rs");
    include!("provider.rs");
}
