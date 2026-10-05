use super::*;

#[test]
fn turn_output_flow_counts_attempts_without_inventing_or_double_counting_usage() {
    use agent_runtime_core::usage::Provenance;

    let mut usage = TurnUsage::default();
    let mut record = UsageRecord {
        source: UsageSource::ProviderAttempt,
        provenance: Provenance::default(),
        delta: UsageDelta::new().with(CounterKind::InputUncached, 100),
    };
    usage.record(&record);
    assert_eq!(usage.output, TokenCount::UNKNOWN);

    record.delta = UsageDelta::new().with(CounterKind::Output, 800);
    record.provenance.failed = true;
    usage.record(&record);
    record.provenance.failed = false;
    record.delta = UsageDelta::new().with(CounterKind::Output, 400);
    usage.record(&record);
    assert_eq!(usage.output, TokenCount::reported(1_200));

    record.source = UsageSource::Rollup;
    usage.record(&record);
    record.source = UsageSource::ProviderAttempt;
    record.provenance.purpose = Some(ADVISOR_USAGE_PURPOSE.to_owned());
    usage.record(&record);
    record.provenance.purpose = None;
    record.provenance.attempt_purpose = Some(ProviderAttemptPurpose::CacheKeepalive);
    usage.record(&record);
    assert_eq!(usage.output, TokenCount::reported(1_200));

    record.provenance.attempt_purpose = None;
    record.source = UsageSource::ExternalAgent;
    usage.record(&record);
    assert_eq!(usage.output, TokenCount::reported(1_600));
    usage.output = TokenCount::estimated(1_600);
    usage.record(&record);
    assert_eq!(usage.output.render(), "~2k");
}

#[test]
fn elapsed_time_stays_compact_from_seconds_through_hours() {
    assert_eq!(render_elapsed(Duration::ZERO), "0s");
    assert_eq!(render_elapsed(Duration::from_secs(59)), "59s");
    assert_eq!(render_elapsed(Duration::from_secs(65)), "1m 05s");
    assert_eq!(render_elapsed(Duration::from_secs(3_725)), "1h 02m 05s");
}

#[test]
fn terminal_elapsed_time_is_honest_below_one_second() {
    assert_eq!(render_terminal_elapsed(Duration::ZERO), "<1ms");
    assert_eq!(render_terminal_elapsed(Duration::from_millis(842)), "842ms");
    assert_eq!(render_terminal_elapsed(Duration::from_secs(1)), "1s");
}

#[test]
fn provenance_is_visible_in_the_rendering() {
    assert_eq!(TokenCount::reported(12_400).render(), "12.4k");
    assert_eq!(TokenCount::estimated(12_400).render(), "~12.4k");
    assert_eq!(TokenCount::UNKNOWN.render(), "?");
}

#[test]
fn an_unknown_count_never_renders_as_zero() {
    // The distinction this test protects: "no tokens were used" and "the
    // provider never told us" must not look the same.
    assert_eq!(TokenCount::UNKNOWN.render(), "?");
    assert_eq!(TokenCount::reported(0).render(), "0");
    assert_ne!(
        TokenCount::UNKNOWN.render(),
        TokenCount::reported(0).render()
    );
}

#[test]
fn reported_usage_accumulates_disjoint_input_categories() {
    let mut status = Status::new("gpt-5.3", "~/work/api");
    assert_eq!(status.context.render(), "?");

    status.record_usage(
        &UsageDelta::new()
            .with(CounterKind::InputUncached, 500)
            .with(CounterKind::InputCached, 8_000)
            .with(CounterKind::Output, 300),
    );
    // Input categories sum; output is not context.
    assert_eq!(status.context.render(), "8.5k");
    assert!(status.has_reported_usage());

    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1_500));
    assert_eq!(status.context.render(), "10k");
}

#[test]
fn advisor_usage_has_separate_totals_and_uses_its_own_model_prices() {
    use agent_runtime_core::usage::{Provenance, UsageSource};

    let mut status = Status::new("main", "/repo");
    let main_price = PriceReference {
        provider: "main-provider".into(),
        model: "main".into(),
        table: PriceTable {
            input: Some(1_000_000),
            output: Some(2_000_000),
            cache_read: Some(100_000),
            cache_write: Some(2_000_000),
        },
    };
    let advisor_price = PriceReference {
        provider: "advisor-provider".into(),
        model: "reviewer".into(),
        table: PriceTable {
            input: Some(10_000_000),
            output: Some(20_000_000),
            cache_read: Some(1_000_000),
            cache_write: Some(20_000_000),
        },
    };
    status.set_advisor_price(Some(advisor_price));
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 100));
    let record = UsageRecord {
        source: UsageSource::SemanticSummary,
        provenance: Provenance {
            purpose: Some(ADVISOR_USAGE_PURPOSE.into()),
            failed: true,
            ..Provenance::default()
        },
        delta: UsageDelta::new()
            .with(CounterKind::InputUncached, 50)
            .with(CounterKind::InputCached, 20)
            .with(CounterKind::CacheWrite, 5)
            .with(CounterKind::Output, 10),
    };
    status.record_usage_record(&record);
    // Output-only advice still contributes without altering root context.
    let output_only = UsageRecord {
        delta: UsageDelta::new().with(CounterKind::Output, 5),
        ..record.clone()
    };
    status.record_usage_record(&output_only);
    let mut usage = status.session_usage();
    assert_eq!(usage.total_tokens(), 100);
    assert_eq!(usage.merged_total_tokens(), 190);
    assert_eq!(usage.turns, 0);
    assert_eq!(status.context, TokenCount::reported(100));
    assert!(usage.render().expect("usage").contains("  advisor:"));
    assert_eq!(
        SessionCost::compute(&usage, &main_price),
        SessionCost {
            micro_usd: 1_020,
            label: CostLabel::Exact
        }
    );
    assert_eq!(
        main_price.render_sources(&usage),
        "main-provider/main · advisor advisor-provider/reviewer"
    );
    usage.reconcile_advisor_records(&[record, output_only]);
    assert_eq!(
        usage.merged_total_tokens(),
        190,
        "reconciliation counts once"
    );
    usage.advisor_price = None;
    assert_eq!(
        SessionCost::compute(&usage, &main_price),
        SessionCost {
            micro_usd: 100,
            label: CostLabel::Estimated
        }
    );
}

#[test]
fn synthetic_usage_counts_toward_session_spend_without_creating_turns() {
    use agent_runtime_core::usage::{Provenance, UsageSource};

    let mut status = Status::new("gpt-5.6", "~/work/api");
    status.record_usage_record(&UsageRecord {
        source: UsageSource::ProviderAttempt,
        provenance: Provenance::default(),
        delta: UsageDelta::new()
            .with(CounterKind::InputUncached, 1_000)
            .with(CounterKind::Output, 50),
    });
    status.record_usage_record(&UsageRecord {
        source: UsageSource::ProviderAttempt,
        provenance: Provenance {
            attempt_purpose: Some(ProviderAttemptPurpose::CacheKeepalive),
            ..Provenance::default()
        },
        delta: UsageDelta::new()
            .with(CounterKind::InputCached, 800)
            .with(CounterKind::Output, 2),
    });
    status.record_usage_record(&UsageRecord {
        source: UsageSource::SemanticSummary,
        provenance: Provenance {
            purpose: Some("cache_idle_compaction".to_owned()),
            attempt_purpose: Some(ProviderAttemptPurpose::IdleCompaction),
            ..Provenance::default()
        },
        delta: UsageDelta::new()
            .with(CounterKind::InputUncached, 100)
            .with(CounterKind::Output, 10),
    });

    let usage = status.session_usage();
    assert_eq!(usage.turns, 0);
    assert_eq!(usage.totals[&CounterKind::InputUncached], 1_000);
    assert_eq!(usage.totals[&CounterKind::Output], 50);
    assert_eq!(usage.synthetic_totals[&CounterKind::InputUncached], 100);
    assert_eq!(usage.synthetic_totals[&CounterKind::InputCached], 800);
    assert_eq!(usage.synthetic_totals[&CounterKind::Output], 12);
    assert_eq!(
        usage.synthetic_by_purpose[&ProviderAttemptPurpose::CacheKeepalive]
            [&CounterKind::InputCached],
        800
    );
    assert_eq!(usage.total_tokens(), 1_050);
    assert_eq!(usage.merged_total_tokens(), 1_962);
    assert_eq!(status.context.render(), "1k");
    let rendered = usage.render().expect("usage projection");
    assert!(rendered.contains("cache_keepalive"), "{rendered}");
    assert!(rendered.contains("cache_idle_compaction"), "{rendered}");
}

#[test]
fn same_model_synthetic_cost_is_priced_but_idle_summary_cost_stays_unknown() {
    let price = priced(1_000_000, 1_000_000, 1_000_000, 1_000_000);
    let mut keepalive = SessionUsage {
        reported: true,
        ..SessionUsage::default()
    };
    keepalive
        .synthetic_totals
        .insert(CounterKind::InputCached, 80);
    keepalive.synthetic_by_purpose.insert(
        ProviderAttemptPurpose::CacheKeepalive,
        BTreeMap::from([(CounterKind::InputCached, 80)]),
    );
    let keepalive_cost = SessionCost::compute(&keepalive, &price);
    assert_eq!(keepalive_cost.micro_usd, 80);
    assert_eq!(keepalive_cost.label, CostLabel::Exact);

    let mut idle = SessionUsage {
        reported: true,
        ..SessionUsage::default()
    };
    idle.synthetic_totals
        .insert(CounterKind::InputUncached, 100);
    idle.synthetic_by_purpose.insert(
        ProviderAttemptPurpose::IdleCompaction,
        BTreeMap::from([(CounterKind::InputUncached, 100)]),
    );
    let idle_cost = SessionCost::compute(&idle, &price);
    assert_eq!(idle_cost.micro_usd, 0);
    assert_eq!(idle_cost.label, CostLabel::Estimated);
}

#[test]
fn synthetic_bindings_keep_their_prices_across_switches_and_reconciliation() {
    use agent_runtime_core::usage::Provenance;

    for purpose in [
        ProviderAttemptPurpose::CacheKeepalive,
        ProviderAttemptPurpose::CacheHandoffCheckpoint,
    ] {
        let mut status = bound_status("zai", "glm-5.3", 0);
        status.set_price(Some(PriceReference {
            provider: "zai".into(),
            model: "glm-5.3".into(),
            ..priced(0, 0, 2_000_000, 0)
        }));
        let glm = UsageRecord {
            source: UsageSource::ProviderAttempt,
            provenance: Provenance {
                attempt_purpose: Some(purpose),
                ..Provenance::default()
            },
            delta: UsageDelta::new().with(CounterKind::InputCached, 11_000),
        };
        status.record_usage_record(&glm);
        status.switch_model(Some("google".into()), "gemini-3.8-flash");
        status.set_price(Some(PriceReference {
            provider: "google".into(),
            model: "gemini-3.8-flash".into(),
            ..priced(0, 0, 1_000_000, 0)
        }));
        let gemini = UsageRecord {
            delta: UsageDelta::new().with(CounterKind::InputCached, 12_000),
            ..glm.clone()
        };
        status.record_usage_record(&gemini);
        let mut usage = status.session_usage();
        assert_eq!(usage.turns, 0);
        assert_eq!(usage.total_tokens(), 0);
        assert_eq!(usage.merged_total_tokens(), 23_000);
        assert_eq!(usage.synthetic_totals[&CounterKind::InputCached], 23_000);
        assert_eq!(
            usage.synthetic_by_purpose[&purpose][&CounterKind::InputCached],
            23_000
        );
        assert_eq!(usage.bindings.len(), 2);
        assert_eq!(
            usage.bindings[0].synthetic_by_purpose[&purpose][&CounterKind::InputCached],
            11_000
        );
        assert_eq!(
            usage.bindings[1].synthetic_by_purpose[&purpose][&CounterKind::InputCached],
            12_000
        );
        let price = status.price().expect("current price");
        let expected = SessionCost {
            micro_usd: 34_000,
            label: CostLabel::Exact,
        };
        assert_eq!(SessionCost::compute(&usage, price), expected);
        assert_eq!(
            price.render_sources(&usage),
            "zai/glm-5.3 $0.022 · google/gemini-3.8-flash $0.012"
        );
        usage.reconcile_synthetic_records(&[glm, gemini]);
        assert_eq!(usage, status.session_usage());
        assert_eq!(SessionCost::compute(&usage, price), expected);

        let mut rebuilt = bound_status("google", "gemini-3.8-flash", 0);
        rebuilt.record_synthetic_usage(
            purpose,
            &UsageDelta::new().with(CounterKind::InputCached, 23_000),
        );
        rebuilt.retain_usage_bindings(&usage);
        assert_eq!(rebuilt.session_usage().bindings, usage.bindings);
        assert_eq!(
            SessionCost::compute(&rebuilt.session_usage(), price),
            expected
        );
    }
}

#[test]
fn unpriced_keepalive_is_estimated_and_never_borrows_the_last_bindings_rate() {
    let mut status = bound_status("openai", "gpt-5.3", 2_000_000);
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1_000_000));
    status.switch_model(Some("custom".into()), "unpriced");
    status.record_synthetic_usage(
        ProviderAttemptPurpose::CacheKeepalive,
        &UsageDelta::new().with(CounterKind::InputCached, 9_000_000),
    );
    status.switch_model(Some("google".into()), "unused");
    status.set_price(Some(PriceReference {
        provider: "google".into(),
        model: "unused".into(),
        ..priced(0, 0, 10_000_000, 0)
    }));
    let usage = status.session_usage();
    let price = usage.cost_price(status.price()).expect("retained price");
    assert_eq!(
        SessionCost::compute(&usage, price),
        SessionCost {
            micro_usd: 2_000_000,
            label: CostLabel::Estimated,
        }
    );
    assert_eq!(
        price.render_sources(&usage),
        "openai/gpt-5.3 $2.000 · price unknown for custom/unpriced"
    );

    let mut unpriced = Status::new("unpriced", "project");
    unpriced.switch_model(Some("custom".into()), "unpriced");
    unpriced.record_synthetic_usage(
        ProviderAttemptPurpose::CacheKeepalive,
        &UsageDelta::new().with(CounterKind::InputCached, 80),
    );
    unpriced.switch_model(Some("openai".into()), "gpt-5.3");
    unpriced.set_price(Some(priced(1_000_000, 1_000_000, 1_000_000, 1_000_000)));
    assert!(
        unpriced
            .session_usage()
            .cost_price(unpriced.price())
            .is_none()
    );
}

#[test]
fn single_binding_synthetic_cost_and_sources_match_the_legacy_rollups() {
    let price = priced(1_000_000, 1_000_000, 1_000_000, 1_000_000);
    let mut status = bound_status("openai", "gpt-5.3", 0);
    status.set_price(Some(price.clone()));
    status.record_synthetic_usage(
        ProviderAttemptPurpose::CacheKeepalive,
        &UsageDelta::new().with(CounterKind::InputCached, 80),
    );
    let usage = status.session_usage();
    let mut legacy = usage.clone();
    legacy.bindings.clear();
    let cost = SessionCost::compute(&usage, &price);
    assert_eq!(cost.micro_usd, 80);
    assert_eq!(cost.label, CostLabel::Exact);
    assert_eq!(cost, SessionCost::compute(&legacy, &price));
    assert_eq!(price.render_sources(&usage), price.render_sources(&legacy));
    assert_eq!(usage.render(), legacy.render());

    status.record_synthetic_usage(
        ProviderAttemptPurpose::IdleCompaction,
        &UsageDelta::new().with(CounterKind::InputUncached, 100),
    );
    let usage = status.session_usage();
    let mut legacy = usage.clone();
    legacy.bindings.clear();
    let cost = SessionCost::compute(&usage, &price);
    assert_eq!(cost.micro_usd, 80);
    assert_eq!(cost.label, CostLabel::Estimated);
    assert_eq!(cost, SessionCost::compute(&legacy, &price));
    assert_eq!(price.render_sources(&usage), price.render_sources(&legacy));
    assert_eq!(usage.render(), legacy.render());

    // Synthetic-only usage freezes the bucket just as a root turn does.
    status.set_price(Some(priced(5_000_000, 5_000_000, 5_000_000, 5_000_000)));
    assert_eq!(
        SessionCost::compute(&status.session_usage(), status.price().expect("new price")),
        cost
    );
}

#[test]
fn a_model_switch_downgrades_reported_context_to_estimated() {
    let mut status = Status::new("gpt-5.3", "~/work/api");
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 9_000));
    status.record_cache(8_000);
    assert_eq!(status.context.render(), "9k");
    assert_eq!(status.render_cache(), "8k");

    status.switch_model(Some("anthropic".into()), "claude-opus-5");
    assert_eq!(
        status.context.render(),
        "~9k",
        "the old provider's count no longer describes the new one"
    );
    assert_eq!(
        status.render_cache(),
        "?",
        "the previous provider's cache does not transfer"
    );
    assert!(!status.has_reported_usage());
}

#[test]
fn cache_evidence_is_distinct_from_a_zero_hit() {
    let mut status = Status::new("gpt-5.3", "~/work/api");
    assert_eq!(status.render_cache(), "?");
    status.record_cache(0);
    assert_eq!(status.render_cache(), "0");
}

#[test]
fn output_only_usage_does_not_turn_unknown_input_into_reported_zero() {
    let mut status = Status::new("gpt-5.3", "~/work/api");
    status.record_usage(&UsageDelta::new().with(CounterKind::Output, 300));

    assert_eq!(status.context, TokenCount::UNKNOWN);
    assert!(!status.has_reported_usage());
}

#[test]
fn the_latest_context_plan_reports_capacity_remaining_and_provenance() {
    let totals = BTreeMap::from([
        (SegmentKind::new("history"), 2_000),
        (SegmentKind::new("tool_schema"), 500),
    ]);
    let mut status = Status::new("gpt-5.3", "~/work/api");
    status.record_context_plan(ContextPlanUpdate {
        fingerprint: "context-exact",
        cache_fingerprint: "cache-exact",
        input_tokens: 2_500,
        input_budget_tokens: 10_000,
        reserved_tokens: 2_000,
        segment_count: 2,
        totals: &totals,
        confidence: EstimationConfidence::Exact,
    });

    let plan = status.context_plan.as_ref().expect("a plan");
    assert_eq!(plan.remaining_tokens(), 7_500);
    assert_eq!(plan.percent_left(), 75);
    assert_eq!(plan.render_input(), "2.5k");
    assert_eq!(plan.render_footer(), "75% ctx");
    assert_eq!(plan.totals["history"], 2_000);

    status.record_context_plan(ContextPlanUpdate {
        fingerprint: "context-estimated",
        cache_fingerprint: "cache-estimated",
        input_tokens: 2_500,
        input_budget_tokens: 10_000,
        reserved_tokens: 2_000,
        segment_count: 2,
        totals: &totals,
        confidence: EstimationConfidence::Estimated,
    });
    let estimated = status.context_plan.as_ref().expect("estimated plan");
    assert_eq!(estimated.render_input(), "~2.5k");
    assert_eq!(estimated.render_footer(), "~75% ctx");
}

#[test]
fn switching_models_clears_a_plan_that_no_longer_applies() {
    let mut status = Status::new("gpt-5.3", "~/work/api");
    let totals = BTreeMap::from([(SegmentKind::new("history"), 100)]);
    status.record_context_plan(ContextPlanUpdate {
        fingerprint: "context-before-switch",
        cache_fingerprint: "cache-before-switch",
        input_tokens: 100,
        input_budget_tokens: 1_000,
        reserved_tokens: 100,
        segment_count: 1,
        totals: &totals,
        confidence: EstimationConfidence::Exact,
    });
    assert_eq!(status.render_context_footer(), "90% ctx");

    status.switch_model(Some("anthropic".into()), "claude-opus-5");
    assert!(status.context_plan.is_none());
    assert_eq!(status.render_context_footer(), "unknown ctx");
}

#[test]
fn catalog_conversion_preserves_missing_and_zero_prices() {
    let cost = smith_config::catalog::CatalogModelCost {
        input: Some(2_000_000),
        output: None,
        cache_read: Some(0),
        cache_write: None,
    };
    let price = PriceReference::from_catalog("provider", "model", &cost);
    assert_eq!(price.provider, "provider");
    assert_eq!(price.model, "model");
    assert_eq!(price.table, cost);
    assert_eq!(
        CachePrice::from(&price.table),
        CachePrice {
            input: Some(2_000_000),
            cache_read: Some(0),
            cache_write: None,
        }
    );
}

fn priced(input: u64, output: u64, cache_read: u64, cache_write: u64) -> PriceReference {
    PriceReference {
        provider: "openai".to_owned(),
        model: "gpt-5.3".to_owned(),
        table: PriceTable {
            input: Some(input),
            output: Some(output),
            cache_read: Some(cache_read),
            cache_write: Some(cache_write),
        },
    }
}

#[test]
fn a_priced_model_with_reported_counters_renders_one_exact_figure() {
    // usage-accounting: "A priced model with reported counters" — every
    // counter the session accumulated is priced and provider-reported,
    // so the figure is exact.
    let mut totals = BTreeMap::new();
    totals.insert(CounterKind::InputUncached, 1_000_000); // 1M tokens
    totals.insert(CounterKind::Output, 500_000); // 0.5M tokens
    let usage = SessionUsage {
        turns: 1,
        reported: true,
        totals,
        ..SessionUsage::default()
    };
    // $2/million input, $8/million output.
    let price = priced(2_000_000, 8_000_000, 0, 0);
    let cost = SessionCost::compute(&usage, &price);
    assert_eq!(cost.label, CostLabel::Exact);
    // 1M * $2 + 0.5M * $8 = $2 + $4 = $6.
    assert_eq!(cost.micro_usd, 6_000_000);
    assert_eq!(cost.render(), "$6.000");
}

#[test]
fn an_unreported_session_downgrades_the_cost_label() {
    // usage-accounting: "An estimated counter downgrades the label" — a
    // tokenizer-estimated session must not present its cost as exact
    // just because the price reference itself is exact.
    let mut totals = BTreeMap::new();
    totals.insert(CounterKind::InputUncached, 1_000_000);
    let usage = SessionUsage {
        turns: 1,
        reported: false,
        totals,
        ..SessionUsage::default()
    };
    let price = priced(2_000_000, 8_000_000, 0, 0);
    let cost = SessionCost::compute(&usage, &price);
    assert_eq!(cost.label, CostLabel::Estimated);
    assert_eq!(cost.render(), "~$2.000");
}

#[test]
fn an_unpriced_contributing_counter_downgrades_the_label_but_still_prices_what_it_can() {
    // A cache-write counter with no catalog price must not be silently
    // folded in as zero without consequence: it downgrades the label,
    // per the task's explicit rule for `CounterKind::Reasoning` applied
    // here to any unpriced counter.
    let mut totals = BTreeMap::new();
    totals.insert(CounterKind::InputUncached, 1_000_000);
    totals.insert(CounterKind::CacheWrite, 1_000_000);
    let usage = SessionUsage {
        turns: 1,
        reported: true,
        totals,
        ..SessionUsage::default()
    };
    // No cache_write price configured.
    let price = priced(2_000_000, 8_000_000, 0, 0);
    let price = PriceReference {
        table: PriceTable {
            cache_write: None,
            ..price.table
        },
        ..price
    };
    let cost = SessionCost::compute(&usage, &price);
    assert_eq!(cost.label, CostLabel::Estimated);
    // Only the priced input counter contributes; the unpriced cache
    // write contributes nothing rather than being guessed at.
    assert_eq!(cost.micro_usd, 2_000_000);
}

#[test]
fn reasoning_tokens_are_never_priced_and_downgrade_the_label() {
    // Models.dev publishes no reasoning price, and reasoning tokens are
    // billed separately from output tokens (disjoint counters, per
    // `agent_runtime_core::usage::CounterKind::Reasoning`'s own doc), so
    // a nonzero reasoning counter can never be exact even when every
    // other counter is fully priced and provider-reported.
    let mut totals = BTreeMap::new();
    totals.insert(CounterKind::InputUncached, 1_000_000);
    totals.insert(CounterKind::Reasoning, 200_000);
    let usage = SessionUsage {
        turns: 1,
        reported: true,
        totals,
        ..SessionUsage::default()
    };
    let price = priced(2_000_000, 8_000_000, 4_000_000, 1_000_000);
    let cost = SessionCost::compute(&usage, &price);
    assert_eq!(cost.label, CostLabel::Estimated);
    // Reasoning tokens contribute nothing to the dollar figure — never a
    // price the catalog did not publish — but the priced input still
    // counts.
    assert_eq!(cost.micro_usd, 2_000_000);
}

#[test]
fn delegated_totals_are_priced_by_the_same_reference_as_root() {
    // usage-accounting: "Delegated counters keep their categories" — the
    // delegated totals are priced by the same per-counter reference the
    // root totals are, not by a different (possibly cheaper or more
    // expensive) child model.
    let mut totals = BTreeMap::new();
    totals.insert(CounterKind::InputUncached, 1_000_000);
    let mut delegated_totals = BTreeMap::new();
    delegated_totals.insert(CounterKind::Output, 500_000);
    let usage = SessionUsage {
        turns: 1,
        reported: true,
        totals,
        delegated_totals,
        delegated_contributors: 2,
        ..SessionUsage::default()
    };
    let price = priced(2_000_000, 8_000_000, 0, 0);
    let cost = SessionCost::compute(&usage, &price);
    assert_eq!(cost.label, CostLabel::Exact);
    // Root: 1M * $2 = $2. Delegated: 0.5M * $8 = $4. Total $6, from one
    // reference.
    assert_eq!(cost.micro_usd, 6_000_000);
}

#[test]
fn sub_cent_costs_render_distinctly_from_a_free_session() {
    // A session that spent a real but tiny amount must never render
    // identically to one that spent nothing — the dollar-figure version
    // of the zero/unknown collapse this module exists to prevent.
    let tiny = SessionCost {
        micro_usd: 400, // $0.0004
        label: CostLabel::Exact,
    };
    assert_eq!(tiny.render(), "$0.000400");
    let free = SessionCost {
        micro_usd: 0,
        label: CostLabel::Exact,
    };
    assert_eq!(free.render(), "$0.000");
    assert_ne!(tiny.render(), free.render());
}

#[test]
fn the_established_three_decimal_rendering_matches_design_doc() {
    // DESIGN.md §7: "$0.031" exact, "~$0.031" estimated.
    assert_eq!(
        SessionCost {
            micro_usd: 31_000,
            label: CostLabel::Exact,
        }
        .render(),
        "$0.031"
    );
    assert_eq!(
        SessionCost {
            micro_usd: 31_000,
            label: CostLabel::Estimated,
        }
        .render(),
        "~$0.031"
    );
}

#[test]
fn a_large_session_does_not_overflow_or_panic() {
    // Extreme (not merely large) token and price magnitudes: `u64::MAX`
    // tokens at a `u64::MAX` micro-USD-per-million rate. The
    // intermediate product overflows `u64` (debug builds panic on
    // overflow), which is exactly why `SessionCost::compute` widens to
    // `u128` before dividing back down to micro-USD.
    let mut totals = BTreeMap::new();
    totals.insert(CounterKind::InputUncached, u64::MAX);
    let usage = SessionUsage {
        turns: 1,
        reported: true,
        totals,
        ..SessionUsage::default()
    };
    let price = priced(u64::MAX, 0, 0, 0);
    let cost = SessionCost::compute(&usage, &price);
    let expected = u128::from(u64::MAX) * u128::from(u64::MAX) / 1_000_000;
    assert_eq!(cost.micro_usd, expected);
    assert_eq!(cost.label, CostLabel::Exact);
}

#[test]
fn resolving_a_price_does_not_change_the_recorded_usage_shape() {
    // usage-accounting: "Cost changes no decision" — pinned at the one
    // artifact that actually reaches persistence and every tool-facing
    // surface. `SessionUsage` is what `SessionUsageRecord::new` records
    // and what `App::session_usage` hands to every other surface; it
    // carries no price field at all, so resolving (or clearing) a price
    // on `Status` cannot change what gets recorded or read back,
    // regardless of what the price is.
    let mut status = Status::new("gpt-5.3", "~/work/api");
    status.record_usage(
        &UsageDelta::new()
            .with(CounterKind::InputUncached, 1_000)
            .with(CounterKind::Output, 200),
    );
    let usage_without_price = status.session_usage();

    status.set_price(Some(priced(2_000_000, 8_000_000, 0, 0)));
    let usage_with_price = status.session_usage();

    assert_eq!(
        usage_without_price, usage_with_price,
        "a resolved price must not change the recorded usage shape"
    );
}

#[test]
fn switching_models_clears_a_price_that_no_longer_describes_the_binding() {
    let mut status = Status::new("gpt-5.3", "~/work/api");
    status.set_price(Some(priced(2_000_000, 8_000_000, 0, 0)));
    assert!(status.price().is_some());

    status.switch_model(Some("anthropic".into()), "claude-opus-5");
    assert!(
        status.price().is_none(),
        "the old provider's price does not describe the new binding"
    );
}

fn bound_status(provider: &str, model: &str, input_rate: u64) -> Status {
    let mut status = Status::new(model, "project");
    status.switch_model(Some(provider.to_owned()), model);
    status.set_price(Some(PriceReference {
        provider: provider.to_owned(),
        model: model.to_owned(),
        ..priced(input_rate, 0, 0, 0)
    }));
    status
}

#[test]
fn root_bindings_keep_their_prices_across_switches_and_rebuilds() {
    let mut status = bound_status("zai", "glm-5.3", 2_000_000);
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 11_000));
    status.switch_model(Some("google".into()), "gemini-3.8-flash");
    status.set_price(Some(PriceReference {
        provider: "google".into(),
        model: "gemini-3.8-flash".into(),
        ..priced(1_000_000, 0, 0, 0)
    }));
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 12_000));
    let usage = status.session_usage();
    assert_eq!(usage.total_tokens(), 23_000);
    assert_eq!(usage.bindings.len(), 2);
    let price = status.price().expect("current price");
    let cost = SessionCost::compute(&usage, price);
    assert_eq!(cost.micro_usd, 34_000);
    assert_eq!(cost.label, CostLabel::Exact);
    assert_eq!(
        price.render_sources(&usage),
        "zai/glm-5.3 $0.022 · google/gemini-3.8-flash $0.012"
    );

    let mut rebuilt = bound_status("google", "gemini-3.8-flash", 1_000_000);
    rebuilt.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 23_000));
    rebuilt.retain_usage_bindings(&usage);
    assert_eq!(rebuilt.session_usage().bindings, usage.bindings);
    assert_eq!(SessionCost::compute(&rebuilt.session_usage(), price), cost);
    // A rebuild whose durable rollup differs must not invent live counters.
    rebuilt.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1));
    let before = rebuilt.session_usage();
    rebuilt.retain_usage_bindings(&usage);
    assert_eq!(rebuilt.session_usage(), before);
}

#[test]
fn an_unpriced_root_binding_is_named_and_never_borrows_a_rate() {
    let mut status = bound_status("openai", "gpt-5.3", 2_000_000);
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1_000_000));
    status.switch_model(Some("custom".into()), "unpriced");
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 9_000_000));
    let usage = status.session_usage();
    let price = usage
        .cost_price(status.price())
        .expect("retained root price");
    let cost = SessionCost::compute(&usage, price);
    assert_eq!(cost.micro_usd, 2_000_000);
    assert_eq!(cost.label, CostLabel::Estimated);
    assert_eq!(
        price.render_sources(&usage),
        "openai/gpt-5.3 $2.000 · price unknown for custom/unpriced"
    );
}

#[test]
fn binding_confidence_survives_a_new_models_provider_report() {
    use agent_runtime_core::usage::Provenance;
    let mut status = bound_status("openai", "gpt-5.3", 2_000_000);
    status.record_usage_record(&UsageRecord {
        source: UsageSource::ToolLoop,
        provenance: Provenance::default(),
        delta: UsageDelta::new().with(CounterKind::InputUncached, 1_000),
    });
    status.switch_model(Some("google".into()), "gemini-3.8-flash");
    status.set_price(Some(PriceReference {
        provider: "google".into(),
        model: "gemini-3.8-flash".into(),
        ..priced(1_000_000, 0, 0, 0)
    }));
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1_000));
    let usage = status.session_usage();
    assert!(!usage.bindings[0].reported);
    assert!(usage.bindings[1].reported);
    assert_eq!(
        SessionCost::compute(&usage, status.price().expect("price")).label,
        CostLabel::Estimated
    );
}

#[test]
fn switching_without_new_counters_keeps_the_previous_bill_exact() {
    let mut status = bound_status("openai", "gpt-5.3", 2_000_000);
    status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1_000_000));
    status.switch_model(Some("custom".into()), "unused");
    let usage = status.session_usage();
    let price = usage.cost_price(None).expect("previous price");
    assert_eq!(SessionCost::compute(&usage, price).label, CostLabel::Exact);
    assert_eq!(price.render_sources(&usage), "openai/gpt-5.3");
}
