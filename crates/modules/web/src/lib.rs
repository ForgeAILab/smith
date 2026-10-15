//! Bounded, untrusted web content mounted through Smith's module contract.

mod fetch;
mod html;
mod search;
mod tool;

use std::sync::Arc;

use smith_module::{
    Module, ModuleContext, ModuleContribution, ModuleError, Mounted, StatusItem, StatusSeverity,
    StatusSource,
};

pub use smith_module::{SearchBackendConfig, WebSettings};

/// Optional web reads and search through a user-selected backend.
#[derive(Debug, Default)]
pub struct WebModule;

impl Module for WebModule {
    fn id(&self) -> &str {
        "web"
    }

    fn revision(&self) -> &str {
        "smith-web-v1"
    }

    fn description(&self) -> &str {
        "Fetch pages and search the web"
    }

    fn default_enabled(&self) -> bool {
        true
    }

    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError> {
        if !context.built_in_tools {
            return Ok(Mounted::Inactive {
                reason: "built-in tools are disabled".into(),
            });
        }
        let settings = context.web.clone().unwrap_or_default();
        let mut contributions = vec![ModuleContribution::Tool(Arc::new(tool::FetchTool {
            allow_private_networks: settings.allow_private_networks,
        }))];
        let backend = settings.search.unwrap_or_default();
        match search::SearchClient::new(backend) {
            Ok(Some(client)) => {
                contributions.push(ModuleContribution::Tool(Arc::new(tool::SearchTool {
                    client,
                })))
            }
            Ok(None) => {}
            Err(reason) => contributions.push(ModuleContribution::StatusItem {
                name: "web-search-unavailable".into(),
                source: Arc::new(SearchUnavailable(reason)),
            }),
        }
        Ok(Mounted::Contributions(contributions))
    }
}

#[derive(Debug)]
struct SearchUnavailable(String);

impl StatusSource for SearchUnavailable {
    fn current(&self) -> Option<StatusItem> {
        Some(StatusItem::new(
            "web-search-unavailable",
            &self.0,
            Some(StatusSeverity::Warning),
        ))
    }
}

const BODY_LIMIT: usize = 5 * 1024 * 1024;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const USER_AGENT: &str = concat!("Smith/", env!("CARGO_PKG_VERSION"), " (+web_fetch)");

#[cfg(test)]
mod tests;
