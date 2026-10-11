//! Independent crate exercising the contract through a test-only dependency.

use smith_module::{Module, ModuleContext, ModuleContribution, ModuleError, Mounted, SlashCommand};

/// A native crate explicitly inserted into a test's compiled module list.
#[derive(Debug)]
pub struct ThirdPartyModule;

impl Module for ThirdPartyModule {
    fn id(&self) -> &str {
        "third-party-fixture"
    }
    fn revision(&self) -> &str {
        "fixture-v1"
    }
    fn description(&self) -> &str {
        "Independent native module fixture"
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn mount(&self, _: &ModuleContext) -> Result<Mounted, ModuleError> {
        Ok(Mounted::Contributions(vec![ModuleContribution::Command(
            SlashCommand {
                name: "native-fixture".into(),
                description: "Test-only native command declaration".into(),
            },
        )]))
    }
}
