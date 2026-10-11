//! Actual runtime module outcomes and reviewed user-layer edits.

use smith_client::commands::ModulesAction;
use smith_client::local_result::LocalResult;
use smith_client::modules_report::module_report;
use smith_runtime::host::HostSession;

use super::CommandReport;

pub(super) fn command(host: &HostSession, action: ModulesAction) -> CommandReport {
    let runtime = host.runtime();
    match action {
        ModulesAction::List => CommandReport::Show(LocalResult::Modules(Box::new(module_report(
            runtime.module_report(),
            &runtime.policy().modules,
        )))),
        ModulesAction::Switch { id, enabled } => {
            match crate::modules::prepare_switch(
                &runtime.policy().user_dir,
                runtime.module_report(),
                &id,
                enabled,
            ) {
                Ok(prepared) => CommandReport::ModuleSwitch {
                    request: smith_client::commands::ModuleSwitchRequest {
                        id,
                        enabled,
                        fingerprint: prepared.fingerprint,
                    },
                    preview: prepared.edit.preview(),
                },
                Err(error) => CommandReport::Show(LocalResult::Message(Box::new(
                    smith_client::message_report::MessageReport::Error {
                        title: "modules".into(),
                        message: error.to_string(),
                    },
                ))),
            }
        }
    }
}
