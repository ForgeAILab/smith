//! Capability narrowing and inventory projection; executable composition lives in modules.
use smith_config::model::{ModuleEndpointRestriction, PluginModuleSection};
use smith_plugin::ModuleCapabilities;

pub(crate) fn narrow(
    mut grants: ModuleCapabilities,
    restriction: Option<&PluginModuleSection>,
) -> ModuleCapabilities {
    let Some(restriction) = restriction else {
        return grants;
    };
    for (values, rule) in [
        (&mut grants.network, &restriction.network),
        (&mut grants.data_egress, &restriction.data_egress),
    ] {
        match rule {
            Some(ModuleEndpointRestriction::Switch(false)) => values.clear(),
            Some(ModuleEndpointRestriction::Hosts(hosts)) => {
                values.retain(|host| hosts.contains(host))
            }
            _ => {}
        }
    }
    for (value, rule) in [
        (&mut grants.workspace_read, restriction.workspace_read),
        (&mut grants.workspace_write, restriction.workspace_write),
        (&mut grants.plugin_data_read, restriction.plugin_data_read),
        (&mut grants.plugin_data_write, restriction.plugin_data_write),
        (&mut grants.session_history, restriction.session_history),
        (&mut grants.model, restriction.model),
        (&mut grants.session_notify, restriction.session_notify),
    ] {
        if rule == Some(false) {
            *value = false;
        }
    }
    grants
}
