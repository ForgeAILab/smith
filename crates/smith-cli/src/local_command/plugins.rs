//! Plugin listing and reviewed user-layer enablement.

use smith_client::commands::{PluginSwitchRequest, PluginsAction};
use smith_client::local_result::LocalResult;
use smith_runtime::host::HostSession;

use super::CommandReport;
use super::file_commands::CommandContext;

pub(super) fn command(
    host: &HostSession,
    context: &CommandContext,
    action: PluginsAction,
) -> CommandReport {
    let result = (|| {
        let mut report = context.plugins_report().map_err(anyhow::Error::msg)?;
        smith_client::plugins_report::apply_module_states(
            &mut report,
            host.runtime().module_report(),
        );
        match action {
            PluginsAction::List => Ok(CommandReport::Show(LocalResult::Plugins(Box::new(report)))),
            PluginsAction::Switch { name, enabled } => {
                let prepared = crate::plugins::prepare_switch(
                    &host.runtime().policy().user_dir,
                    &report,
                    &name,
                    enabled,
                )?;
                Ok(CommandReport::PluginSwitch {
                    request: PluginSwitchRequest {
                        name,
                        enabled,
                        fingerprint: prepared.fingerprint,
                    },
                    preview: prepared.edit.preview(),
                })
            }
        }
    })();
    result.unwrap_or_else(|error: anyhow::Error| {
        CommandReport::Show(LocalResult::Message(Box::new(
            smith_client::message_report::MessageReport::Error {
                title: "plugins".into(),
                message: error.to_string(),
            },
        )))
    })
}
