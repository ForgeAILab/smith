use std::collections::BTreeMap;

use agent_runtime_core::usage::CounterKind;
use smith_client::status::{PriceReference, PriceTable, SessionUsage};

use super::*;

#[test]
fn hosted_search_fallback_notice_names_the_session_backend() {
    assert_eq!(
        hosted_search_fallback_notice(WebSearchMode::Auto, WebSearchBackend::Brave),
        "Hosted web search is unavailable for this organization. Search switched to the brave backend for this session."
    );
    assert_eq!(
        hosted_search_fallback_notice(WebSearchMode::Hosted, WebSearchBackend::DuckDuckGo),
        "Hosted web search is unavailable for this organization. Hosted search is disabled for this session."
    );
}

fn usage(reported: bool, tokens: &[(CounterKind, u64)]) -> SessionUsage {
    let mut totals = BTreeMap::new();
    for (kind, value) in tokens {
        totals.insert(*kind, *value);
    }
    SessionUsage {
        turns: 1,
        reported,
        totals,
        ..SessionUsage::default()
    }
}

fn price() -> PriceReference {
    PriceReference {
        provider: "openai".to_owned(),
        model: "gpt-5.3".to_owned(),
        table: PriceTable {
            input: Some(2_000_000),
            output: Some(8_000_000),
            cache_read: None,
            cache_write: None,
        },
    }
}

#[test]
fn an_exact_session_prints_one_labelled_figure_naming_its_binding() {
    let usage = usage(true, &[(CounterKind::InputUncached, 1_000_000)]);
    let line = render_exit_cost_line(&usage, Some(&price())).expect("a priced line");
    assert_eq!(line, "$2.000 exact · openai/gpt-5.3");
}

#[test]
fn an_estimated_session_prints_the_estimated_glyph_and_word() {
    let usage = usage(false, &[(CounterKind::InputUncached, 1_000_000)]);
    let line = render_exit_cost_line(&usage, Some(&price())).expect("a priced line");
    assert_eq!(line, "~$2.000 estimated · openai/gpt-5.3");
}

#[test]
fn an_unpriced_model_prints_no_cost_line_at_all() {
    // usage-accounting: "A model the catalog does not price" — the exit
    // report prints the token lines and no cost line, never a price
    // substituted from another model, provider, or a hard-coded
    // default.
    let usage = usage(true, &[(CounterKind::InputUncached, 1_000_000)]);
    assert_eq!(render_exit_cost_line(&usage, None), None);
}

#[test]
fn binding_cost_lines_match_status_and_keep_single_binding_bytes() {
    let mut status = smith_client::status::Status::new("gpt-5.3", "project");
    status.switch_model(Some("openai".into()), "gpt-5.3");
    status.set_price(Some(price()));
    status.record_usage(
        &agent_runtime_core::usage::UsageDelta::new().with(CounterKind::InputUncached, 1_000_000),
    );
    let usage = status.session_usage();
    assert_eq!(
        render_exit_cost_line(&usage, status.price()).as_deref(),
        Some("$2.000 exact · openai/gpt-5.3")
    );
    assert_eq!(
        crate::local_command::render_status_cost(&usage, status.price(), ("openai", "gpt-5.3")),
        "$2.000 exact · openai/gpt-5.3"
    );
    let mut estimated = usage.clone();
    estimated.bindings[0].reported = false;
    assert_eq!(
        render_exit_cost_line(&estimated, status.price()).as_deref(),
        Some("~$2.000 estimated · openai/gpt-5.3")
    );
    assert_eq!(
        crate::local_command::render_status_cost(&estimated, status.price(), ("openai", "gpt-5.3")),
        "~$2.000 estimated · openai/gpt-5.3"
    );
    let mut advisor = usage;
    advisor
        .advisor_totals
        .insert(CounterKind::InputUncached, 1_000_000);
    advisor.advisor_price = Some(PriceReference {
        provider: "advisor".into(),
        model: "reviewer".into(),
        ..price()
    });
    assert_eq!(
        render_exit_cost_line(&advisor, status.price()).as_deref(),
        Some("$4.000 exact · openai/gpt-5.3 · advisor advisor/reviewer")
    );
    assert_eq!(
        crate::local_command::render_status_cost(&advisor, status.price(), ("openai", "gpt-5.3")),
        "$4.000 exact · openai/gpt-5.3 · advisor advisor/reviewer"
    );
    advisor.advisor_price = None;
    assert_eq!(
        render_exit_cost_line(&advisor, status.price()).as_deref(),
        Some("~$2.000 estimated · openai/gpt-5.3 · advisor price unknown")
    );
    assert_eq!(
        crate::local_command::render_status_cost(&advisor, status.price(), ("openai", "gpt-5.3")),
        "~$2.000 estimated · openai/gpt-5.3 · advisor price unknown"
    );
}

#[test]
fn harness_cost_lines_omit_the_profile_provider() {
    use agent_runtime_core::usage::UsageDelta;
    use smith_client::status::{BindingUsage, Status};

    for (provider, model) in [
        ("zai", "cli/claude-code/haiku"),
        ("google", "cli/claude-code/sonnet"),
        ("zai", "cli/codex/gpt-6-astra"),
    ] {
        let mut status = Status::new(model, "project");
        status.switch_model(Some(provider.to_owned()), model);
        status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1_000));
        let mut usage = status.session_usage();
        assert_eq!(render_exit_cost_line(&usage, None), None);
        assert_eq!(
            crate::local_command::render_status_cost(&usage, None, (provider, model)),
            format!("unknown · no price reference for {model}")
        );

        let native = PriceReference {
            provider: "zai".into(),
            model: "glm-5.3".into(),
            ..price()
        };
        let mut child = BindingUsage::new(Some("zai".into()), "glm-5.3", Some(native.clone()));
        child.record(
            &UsageDelta::new().with(CounterKind::InputUncached, 11_000),
            true,
        );
        usage.bindings.push(child);
        usage
            .delegated_totals
            .insert(CounterKind::InputUncached, 11_000);
        usage.delegated_contributors = 1;
        usage
            .advisor_totals
            .insert(CounterKind::InputUncached, 12_000);
        usage.advisor_price = Some(native);
        let expected = format!(
            "~$0.046 estimated · price unknown for {model} · zai/glm-5.3 $0.022 · advisor zai/glm-5.3"
        );
        assert_eq!(
            render_exit_cost_line(&usage, None).as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            crate::local_command::render_status_cost(&usage, None, (provider, model)),
            expected
        );
    }
}

#[test]
fn cost_lines_show_each_binding_even_when_the_last_root_price_is_unknown() {
    use agent_runtime_core::usage::UsageDelta;
    use smith_client::status::{BindingUsage, Status};
    let mut status = Status::new("glm-5.3", "project");
    status.switch_model(Some("zai".into()), "glm-5.3");
    status.set_price(Some(PriceReference {
        provider: "zai".into(),
        model: "glm-5.3".into(),
        ..price()
    }));
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 11_000));
    status.switch_model(Some("google".into()), "gemini-3.8-flash");
    status.set_price(Some(PriceReference {
        provider: "google".into(),
        model: "gemini-3.8-flash".into(),
        table: PriceTable {
            input: Some(1_000_000),
            ..price().table
        },
    }));
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 12_000));
    let usage = status.session_usage();
    let expected = "$0.034 exact · zai/glm-5.3 $0.022 · google/gemini-3.8-flash $0.012";
    assert_eq!(
        render_exit_cost_line(&usage, status.price()).as_deref(),
        Some(expected)
    );
    assert_eq!(
        crate::local_command::render_status_cost(
            &usage,
            status.price(),
            ("google", "gemini-3.8-flash")
        ),
        expected
    );

    status.switch_model(Some("custom".into()), "model");
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 999_999));
    let usage = status.session_usage();
    let expected = "~$0.034 estimated · zai/glm-5.3 $0.022 · google/gemini-3.8-flash $0.012 · price unknown for custom/model";
    assert_eq!(
        render_exit_cost_line(&usage, None).as_deref(),
        Some(expected)
    );
    assert_eq!(
        crate::local_command::render_status_cost(&usage, None, ("custom", "model")),
        expected
    );

    let unknown = SessionUsage {
        bindings: vec![BindingUsage {
            totals: BTreeMap::from([(CounterKind::InputUncached, 10)]),
            ..BindingUsage::new(Some("custom".into()), "model", None)
        }],
        totals: BTreeMap::from([(CounterKind::InputUncached, 10)]),
        ..SessionUsage::default()
    };
    assert_eq!(render_exit_cost_line(&unknown, Some(&price())), None);
    assert_eq!(
        crate::local_command::render_status_cost(&unknown, Some(&price()), ("custom", "model")),
        "unknown · no price reference for custom/model"
    );
}

#[test]
fn advisor_palette_target_parses_installed_agents_and_preserves_the_session() {
    let mut selection = Selection::default();
    let mut resume = None;
    apply_palette_command(
        &mut selection,
        &mut resume,
        "current-session".to_owned(),
        SelectionCommand::Advisor(AdvisorChoice::Target("cli/claude-code/opus".to_owned())),
    );
    assert_eq!(
        selection.advisor,
        Some(AdvisorOverride::Target(AdvisorTarget::InstalledAgent {
            kind: "claude-code".to_owned(),
            model: "opus".to_owned(),
        }))
    );
    assert_eq!(resume.as_deref(), Some("current-session"));
}
