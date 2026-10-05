use super::*;

#[test]
fn live_findings_capability_lifecycle_keeps_status_and_the_notice_for_detail() {
    let mut app = app();
    let snapshot = fingerprint("registry");
    let view = fingerprint("view");
    app.apply(&event(RuntimeEvent::RegistrySnapshotSealed {
        snapshot: snapshot.clone(),
        entries: 6,
    }));
    app.apply(&event(RuntimeEvent::ScopedViewDerived {
        snapshot,
        view: view.clone(),
        visible_entries: 4,
    }));
    app.apply(&event(RuntimeEvent::CapabilityRetrievalPerformed {
        resolver_revision: agent_runtime_registry::RegistryRevision::new("resolver-1"),
        index_revision: None,
        candidates: vec![
            agent_runtime_registry::RegistryId::tool("read"),
            agent_runtime_registry::RegistryId::tool("search"),
        ],
    }));
    app.apply(&event(RuntimeEvent::CapabilitiesActivated {
        epoch: 2,
        activation: vec![
            ActivatedCapability::new(
                agent_runtime_registry::RegistryId::tool("read"),
                agent_runtime_registry::RegistryRevision::new("read-1"),
            ),
            ActivatedCapability::new(
                agent_runtime_registry::RegistryId::tool("search"),
                agent_runtime_registry::RegistryRevision::new("search-1"),
            ),
        ],
    }));

    assert_eq!(
        app.status.capabilities.registry,
        Some((fingerprint("registry").as_str().to_owned(), 6))
    );
    assert_eq!(
        app.status.capabilities.view,
        Some((view.as_str().to_owned(), 4))
    );
    assert_eq!(
        app.status.capabilities.retrieval,
        Some((
            "resolver-1".to_owned(),
            vec!["tool:read".to_owned(), "tool:search".to_owned()]
        ))
    );
    assert_eq!(
        app.status.capabilities.activation,
        Some((2, vec!["tool:read".to_owned(), "tool:search".to_owned()]))
    );
    assert!(app.transcript.blocks().iter().any(|block| {
        matches!(
            block,
            Block::Notice { kind: source, text }
                if source.label() == "capabilities"
                    && text == "activation epoch 2: tool:read, tool:search"
        )
    }));
    assert!(
        !app.work_details,
        "activation must not expand transcript detail"
    );
    app.on_key(ctrl('o'));
    assert!(app.work_details);
    assert_eq!(
        app.status.capabilities.activation,
        Some((2, vec!["tool:read".to_owned(), "tool:search".to_owned()]))
    );
}

#[test]
fn public_todo_updates_remain_replaceable_and_replay_equivalent() {
    let update = event(RuntimeEvent::PlanUpdated {
        revision: 3,
        sensitivity: PlanSensitivity::Public,
        counts: BTreeMap::from([
            ("cancelled".to_owned(), 0),
            ("completed".to_owned(), 1),
            ("in_progress".to_owned(), 1),
            ("pending".to_owned(), 1),
        ]),
        items: Some(vec![
            PlanItemProjection {
                id: "inspect".to_owned(),
                text: "Inspect\nrelevant code".to_owned(),
                status: PlanItemStatus::Completed,
                reason: None,
            },
            PlanItemProjection {
                id: "change".to_owned(),
                text: "Implement the change".to_owned(),
                status: PlanItemStatus::InProgress,
                reason: None,
            },
            PlanItemProjection {
                id: "verify".to_owned(),
                text: "Run focused tests".to_owned(),
                status: PlanItemStatus::Pending,
                reason: None,
            },
        ]),
    });
    let mut live = app();
    live.apply(&event(RuntimeEvent::TurnStarted));
    live.apply(&update);
    let mut replayed = app();
    replayed.apply(&event(RuntimeEvent::TurnStarted));
    replayed.apply(&update);

    assert_eq!(live.plan, replayed.plan);
    assert_eq!(live.transcript.blocks(), replayed.transcript.blocks());
    assert!(live.work_detail_lines().is_empty());
    assert!(
        !live.transcript.blocks().iter().any(
            |block| matches!(block, Block::Notice { kind: source, .. } if source.label() == "plan")
        ),
        "plan updates must replace one work row instead of appending notices"
    );
}

#[test]
fn sensitive_todo_update_displays_counts_without_item_text() {
    const PROTECTED_ITEM: &str = "PROTECTED PLAN CONTENT";
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::PlanUpdated {
        revision: 1,
        sensitivity: PlanSensitivity::Sensitive,
        counts: BTreeMap::from([
            ("cancelled".to_owned(), 1),
            ("completed".to_owned(), 0),
            ("in_progress".to_owned(), 0),
            ("pending".to_owned(), 2),
        ]),
        items: Some(vec![PlanItemProjection {
            id: "protected".to_owned(),
            text: PROTECTED_ITEM.to_owned(),
            status: PlanItemStatus::Pending,
            reason: None,
        }]),
    }));

    let plan = app.plan.as_ref().expect("latest plan");
    assert_eq!(plan.sensitivity, PlanSensitivity::Sensitive);
    assert!(
        plan.items.is_none(),
        "sensitive item text survived the reducer seam"
    );
    assert!(app.work_detail_lines().is_empty());
    assert!(!format!("{:?}", app.transcript.blocks()).contains(PROTECTED_ITEM));
}
