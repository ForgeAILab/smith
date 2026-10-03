//! Captures server snapshots and local MCP trust outcomes.

use smith_client::commands::McpAction;
use smith_client::local_result::LocalResult;
use smith_client::mcp_report::{McpReport, McpServer, McpServerState, McpValue, McpValueKind};
use smith_config::mcp;
use smith_config::trust::TrustStatus;
use smith_runtime::mcp::McpState;

use crate::mcp::McpContext;

use super::CommandReport;

pub(super) fn command(context: Option<&McpContext>, action: McpAction) -> CommandReport {
    match (context, action) {
        (None, _) => CommandReport::Show(LocalResult::Mcp(Box::new(report(None)))),
        (Some(context), McpAction::List) => {
            CommandReport::Show(LocalResult::Mcp(Box::new(report(Some(context)))))
        }
        // Showing the resolved invocation and its content identity is the
        // whole point of the confirmation: the decision is about exactly
        // this content, so exactly this content is what gets displayed.
        (Some(context), McpAction::Trust(server)) => match context.confirmation(&server) {
            Ok(content) => CommandReport::McpTrust { server, content },
            Err(error) => CommandReport::Show(LocalResult::Mcp(Box::new(McpReport::Error(error)))),
        },
    }
}

pub(crate) fn report(context: Option<&McpContext>) -> McpReport {
    let Some(context) = context else {
        return McpReport::Empty { guidance: true };
    };
    let servers = context
        .reports()
        .into_iter()
        .map(|report| {
            let values = context
                .server(&report.name)
                .map_or_else(Vec::new, |server| {
                    let confirmation = mcp::confirmation(server, TrustStatus::Untrusted);
                    confirmation
                        .environment
                        .into_iter()
                        .map(|value| (McpValueKind::Environment, value))
                        .chain(
                            confirmation
                                .headers
                                .into_iter()
                                .map(|value| (McpValueKind::Header, value)),
                        )
                        .map(|(kind, value)| McpValue {
                            kind,
                            name: value.name,
                            credential: value.credential,
                        })
                        .collect()
                });
            McpServer {
                name: report.name,
                transport: report.transport.to_owned(),
                source: report.source.to_string(),
                state: match report.state {
                    McpState::Connected { tools } => McpServerState::Connected { tools },
                    McpState::Connecting => McpServerState::Connecting,
                    McpState::Disabled => McpServerState::Disabled,
                    McpState::NeedsTrust(TrustStatus::Changed) => McpServerState::Changed,
                    McpState::NeedsTrust(TrustStatus::Denied) => McpServerState::Denied,
                    McpState::NeedsTrust(_) => McpServerState::Untrusted,
                    McpState::Failed { reason } => McpServerState::Failed { reason },
                },
                rejected: report.rejected,
                values,
            }
        })
        .collect::<Vec<_>>();
    if servers.is_empty() {
        McpReport::Empty { guidance: false }
    } else {
        McpReport::Servers(servers)
    }
}

pub(crate) fn trust(context: Option<&McpContext>, server: &str) -> McpReport {
    let Some(context) = context else {
        return McpReport::Unavailable;
    };
    match context.trust(server) {
        Ok(digest) => McpReport::Trusted {
            server: server.to_owned(),
            digest: digest.to_string(),
        },
        Err(error) => McpReport::Error(error),
    }
}
