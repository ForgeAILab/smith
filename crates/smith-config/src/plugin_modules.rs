//! User-owned capability narrowing. These settings never create a grant.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
/// Endpoint restriction: false withholds all endpoints, a list retains its intersection.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ModuleEndpointRestriction {
    /// False withholds; true merely retains confirmed authority.
    Switch(bool),
    /// Retain only listed, already granted destinations.
    Hosts(Vec<String>),
}
/// Optional reductions to one installed module's confirmed capability requests.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginModuleSection {
    /// Restrict network endpoints.
    pub network: Option<ModuleEndpointRestriction>,
    /// Restrict task data destinations.
    pub data_egress: Option<ModuleEndpointRestriction>,
    /// False withholds workspace reads.
    pub workspace_read: Option<bool>,
    /// False withholds workspace writes.
    pub workspace_write: Option<bool>,
    /// False withholds plugin data reads.
    pub plugin_data_read: Option<bool>,
    /// False withholds plugin data writes.
    pub plugin_data_write: Option<bool>,
    /// False withholds current-session history.
    pub session_history: Option<bool>,
    /// False withholds calls to the session model.
    pub model: Option<bool>,
    /// False withholds session notifications.
    pub session_notify: Option<bool>,
    /// Output ceiling per model call; may only lower the default of 4096.
    pub model_max_output_tokens: Option<u32>,
    /// Model calls per turn; may only lower the default of 4.
    pub model_max_calls_per_turn: Option<u32>,
}
/// Module id to restriction map in `[plugins.<name>.modules.<id>]`.
pub type PluginModuleRestrictions = BTreeMap<String, PluginModuleSection>;
