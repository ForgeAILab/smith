use super::*;

pub(super) fn fixture_result() -> ResultEnvelope {
    ResultEnvelope {
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status: ResultStatus::Ok,
        session_id: "session-fixture".into(),
        turn_id: "turn-fixture".into(),
        provider: "fixture-provider".into(),
        model: "fixture-model".into(),
        output: "answer".into(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session: UsageDelta::new(),
            synthetic_cache: SyntheticUsageOutput::default(),
            modules: Default::default(),
            current_turn_provenance: UsageProvenance::Unknown,
            session_provenance: UsageProvenance::Unknown,
        },
        lifecycle: LifecycleOutput::default(),
        goal: None,
        goal_continuation_turns: None,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: None,
        recovery: None,
        account: None,
        background_exit: None,
        reasoning: None,
        cache: None,
        resume_capsule: None,
        diagnostics: Vec::new(),
        error: None,
    }
}
