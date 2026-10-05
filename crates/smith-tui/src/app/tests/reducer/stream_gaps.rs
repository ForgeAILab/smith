use super::*;

#[test]
fn a_live_sequence_gap_parks_the_envelope_for_journal_replay() {
    let mut app = app();
    let mut first = event(RuntimeEvent::TurnStarted);
    first.seq = 4;
    app.apply(&first);
    let mut later = event(RuntimeEvent::TurnCompleted {
        finish: TurnFinish::Completed,
        visible_output: true,
    });
    later.seq = 7;
    app.apply(&later);

    // The out-of-order terminal must not fold ahead of the missing
    // events: the turn still reads as live until the host replays.
    assert_eq!(app.status.activity, Activity::Working);
    let gap = app.take_stream_gap().expect("a parked stream gap");
    assert_eq!(gap.first_missing, 5);
    assert_eq!(gap.last_missing, 6);
    assert_eq!(gap.deferred.seq, 7);
    assert!(app.take_stream_gap().is_none(), "taking the gap clears it");
}

#[test]
fn journal_replay_heals_a_gap_without_an_error_row() {
    let mut app = app();
    let mut first = event(RuntimeEvent::TurnStarted);
    first.seq = 4;
    app.apply(&first);
    let mut later = event(RuntimeEvent::TurnCompleted {
        finish: TurnFinish::Completed,
        visible_output: true,
    });
    later.seq = 6;
    app.apply(&later);
    let gap = app.take_stream_gap().expect("a parked stream gap");

    let mut missing = event(RuntimeEvent::CacheObservation {
        request: None,
        attempt: None,
        cache_plan: None,
        cache_identity: None,
        read_tokens: Some(128),
        write_tokens: Some(0),
    });
    missing.seq = 5;
    app.apply_recovered(&missing);
    app.apply_recovered(&gap.deferred);

    assert_eq!(app.status.activity, Activity::Idle);
    assert_eq!(app.status.cache_read, Some(128));
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|block| matches!(block, Block::Error { .. })),
        "a fully replayed gap must not report a skip: {:?}",
        app.transcript.blocks()
    );
}

#[test]
fn a_replayed_turn_terminal_still_releases_the_queued_turn() {
    // The wedge this protects against: the live stream lost
    // `TurnCompleted`, so without journal replay the queued next turn
    // would never dispatch and the UI would sit "working" forever.
    let mut app = app();
    let mut started = turn_event("turn-1", RuntimeEvent::TurnStarted);
    started.seq = 1;
    app.apply(&started);
    app.composer.replace("queued for later");
    assert_eq!(app.on_key(key(KeyCode::Tab)), None);
    assert_eq!(
        app.pending_input_previews()[0].entries,
        ["queued for later"]
    );

    let mut after_gap = event(RuntimeEvent::CacheObservation {
        request: None,
        attempt: None,
        cache_plan: None,
        cache_identity: None,
        read_tokens: Some(1),
        write_tokens: Some(0),
    });
    after_gap.seq = 4;
    app.apply(&after_gap);
    assert!(app.take_ready_submission().is_none(), "still parked");
    let gap = app.take_stream_gap().expect("a parked stream gap");
    assert_eq!((gap.first_missing, gap.last_missing), (2, 3));

    let mut usage = event(RuntimeEvent::CacheObservation {
        request: None,
        attempt: None,
        cache_plan: None,
        cache_identity: None,
        read_tokens: Some(2),
        write_tokens: Some(0),
    });
    usage.seq = 2;
    app.apply_recovered(&usage);
    let mut completed = turn_event(
        "turn-1",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    );
    completed.seq = 3;
    app.apply_recovered(&completed);
    app.apply_recovered(&gap.deferred);

    assert_eq!(app.status.activity, Activity::Idle);
    let submission = app
        .take_ready_submission()
        .expect("the replayed terminal releases the queued turn");
    assert_eq!(submission.display_text(), "queued for later");
}

#[test]
fn an_unhealable_gap_is_still_visible_instead_of_silently_losing_output() {
    let mut app = app();
    let mut first = event(RuntimeEvent::TurnStarted);
    first.seq = 4;
    app.apply(&first);
    let mut later = event(RuntimeEvent::TurnCompleted {
        finish: TurnFinish::Completed,
        visible_output: true,
    });
    later.seq = 7;
    app.apply(&later);

    // The journal had nothing for 5..=6: the parked envelope goes through
    // the replay path alone. The loss is merged, not reported yet — it
    // only becomes a transcript line once the run of gaps ends.
    let gap = app.take_stream_gap().expect("a parked stream gap");
    app.apply_recovered(&gap.deferred);
    assert!(
        app.transcript.is_empty(),
        "a loss is not reported until its run of gaps ends: {:?}",
        app.transcript.blocks()
    );

    let mut caught_up = event(RuntimeEvent::CacheObservation {
        request: None,
        attempt: None,
        cache_plan: None,
        cache_identity: None,
        read_tokens: Some(1),
        write_tokens: Some(0),
    });
    caught_up.seq = 8;
    app.apply(&caught_up);

    assert_eq!(app.status.activity, Activity::Idle);
    assert!(
        app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Notice { kind: source, text }
                if source.label() == "stream" && text.contains("sequence 5 through 6")
        )),
        "{:?}",
        app.transcript.blocks()
    );
    // The old wording claimed the journal remained canonical for exactly
    // the range it had just failed to supply; the honest replacement
    // must never say that.
    let rendered = format!("{:?}", app.transcript.blocks());
    assert!(!rendered.contains("canonical"), "{rendered}");
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|block| matches!(block, Block::Error { .. })),
        "an unrecoverable gap is something the user cannot act on: {:?}",
        app.transcript.blocks()
    );
}

#[test]
fn a_run_of_consecutive_gaps_reports_exactly_one_notice() {
    // The scenario this guards against: a broadcast-channel overrun that
    // outruns the journal too produces several gaps in a row with real
    // content in between each one — exactly what fragmented a single
    // assistant reply into eight pieces before this collapsed.
    let mut app = app();
    let mut turn_started = event(RuntimeEvent::TurnStarted);
    turn_started.seq = 1;
    app.apply(&turn_started);

    for (deferred_seq, gap) in [(4u64, (2u64, 3u64)), (7, (5, 6)), (10, (8, 9))] {
        let mut envelope = event(RuntimeEvent::CacheObservation {
            request: None,
            attempt: None,
            cache_plan: None,
            cache_identity: None,
            read_tokens: Some(1),
            write_tokens: Some(0),
        });
        envelope.seq = deferred_seq;
        app.apply(&envelope);
        let parked = app.take_stream_gap().expect("a parked stream gap");
        assert_eq!((parked.first_missing, parked.last_missing), gap);
        app.apply_recovered(&parked.deferred);
    }
    assert!(
        app.transcript.is_empty(),
        "still mid-run: nothing should be reported yet: {:?}",
        app.transcript.blocks()
    );

    let mut turn_completed = event(RuntimeEvent::TurnCompleted {
        finish: TurnFinish::Completed,
        visible_output: true,
    });
    turn_completed.seq = 11;
    app.apply(&turn_completed);

    let stream_notices = app
            .transcript
            .blocks()
            .iter()
            .filter(|block| matches!(block, Block::Notice { kind: source, .. } if source.label() == "stream"))
            .count();
    assert_eq!(
        stream_notices,
        1,
        "three back-to-back gaps must collapse into one line: {:?}",
        app.transcript.blocks()
    );
    assert!(
        app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Notice { kind: source, text }
                if source.label() == "stream" && text.contains("sequence 2 through 9")
        )),
        "the merged notice must span the whole run: {:?}",
        app.transcript.blocks()
    );
}

#[test]
fn a_run_of_recovered_gaps_reports_one_recovery_notice() {
    let mut app = app();
    let mut turn_started = event(RuntimeEvent::TurnStarted);
    turn_started.seq = 1;
    app.apply(&turn_started);

    // The host calls this once per gap in a run, before folding the
    // recovered events back in through `apply_recovered`.
    app.note_recovered_events(2);
    app.note_recovered_events(3);

    let mut caught_up = event(RuntimeEvent::CacheObservation {
        request: None,
        attempt: None,
        cache_plan: None,
        cache_identity: None,
        read_tokens: Some(1),
        write_tokens: Some(0),
    });
    caught_up.seq = 2;
    app.apply_recovered(&caught_up);

    let stream_notices: Vec<_> = app
        .transcript
        .blocks()
        .iter()
        .filter_map(|block| match block {
            Block::Notice { kind: source, text } if source.label() == "stream" => {
                Some(text.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        stream_notices,
        vec!["live stream lagged; recovered 5 skipped events from the session journal"],
        "two accumulated recoveries must collapse into one line: {:?}",
        app.transcript.blocks()
    );
}
