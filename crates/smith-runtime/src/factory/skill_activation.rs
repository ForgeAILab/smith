use super::*;

use crate::capability_limits::{CapabilityStanding, catalog};
use crate::skills::RequestedSkill;
use agent_runtime::ability::activation::Activated;

impl SmithRuntime {
    /// Checks an explicit user selection against this session before a rebind.
    /// The shared runtime repeats admission when it installs the requested pin.
    pub fn prepare_skill_activation(
        &self,
        session: &agent_runtime::runtime::SessionHandle,
        name: &str,
    ) -> Result<RequestedSkill, RuntimeError> {
        self.prepare_skill_invocation(session, name)
            .map(|(selected, _)| selected)
    }

    /// Returns the admitted skill pin and verified instructions for a user turn.
    pub fn prepare_skill_invocation(
        &self,
        session: &agent_runtime::runtime::SessionHandle,
        name: &str,
    ) -> Result<(RequestedSkill, String), RuntimeError> {
        let refuse = |reason: String| RuntimeError::config(format!("skill `{name}`: {reason}"));
        if self.policy.harness.is_some() {
            return Err(refuse("direct activation is unavailable on installed CLI backends; use their skill MCP bridge".into()));
        }
        let selected = self.skill_catalog.selection(name)?;
        let rows = catalog(session, &[]);
        let id = format!("skill:{name}");
        if !rows.iter().any(|row| {
            row.id == id
                && matches!(
                    row.standing,
                    CapabilityStanding::Active | CapabilityStanding::Available
                )
        }) {
            return Err(refuse("denied by the active profile or session".into()));
        }
        let budget =
            ContextBudget::from_limits(&self.profile.profile.limits, &self.policy.context_policy);
        let instruction_limit = context_policy::skill_instruction_budget(budget.capability_budget);
        let mut tokens = 0u32;
        let mut instructions = None;
        for ability in self.skill_catalog.abilities() {
            let descriptor = ability.descriptor();
            if ability.name() == name
                || rows.iter().any(|row| {
                    row.id == descriptor.id().qualified()
                        && row.standing == CapabilityStanding::Active
                })
            {
                tokens = tokens.saturating_add(descriptor.context_cost().instruction_tokens);
            }
            if ability.name() == name {
                let body = match ability
                    .materialize()
                    .map_err(|error| refuse(error.to_string()))?
                {
                    Activated::SkillInstructions(body) => body,
                    _ => {
                        return Err(refuse(
                            "the declaration did not supply skill instructions".into(),
                        ));
                    }
                };
                if agent_runtime::ability::descriptor::ContextCost::estimate("", &body)
                    .instruction_tokens
                    > instruction_limit
                {
                    return Err(refuse(format!(
                        "instructions exceed the {instruction_limit}-token activation budget"
                    )));
                }
                instructions = Some(body);
            }
        }
        if tokens > instruction_limit {
            return Err(refuse(format!(
                "active skill instructions need {tokens} tokens; activation budget is {instruction_limit}"
            )));
        }
        let instructions = instructions
            .ok_or_else(|| refuse("the declaration did not supply skill instructions".into()))?;
        Ok((selected, instructions))
    }
}
