use super::*;

#[tokio::test]
async fn journal_events_between_returns_exactly_the_missed_range() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Terminal))
        .await
        .expect("a new hosted session");
    host.session()
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");

    let all = host.timeline_events().await.expect("timeline events");
    assert!(all.len() >= 3, "a completed turn appends lifecycle events");

    // A lagged live subscriber asks for an interior range: the healing read
    // must return exactly those events, in order, and nothing else.
    let first = all[1].seq;
    let last = all[all.len() - 2].seq;
    let range = host
        .journal_events_between(first, last)
        .await
        .expect("a ranged journal read");
    assert_eq!(
        range.iter().map(|event| event.seq).collect::<Vec<_>>(),
        all.iter()
            .map(|event| event.seq)
            .filter(|seq| (first..=last).contains(seq))
            .collect::<Vec<_>>(),
    );

    let beyond = host
        .journal_events_between(u64::MAX - 1, u64::MAX)
        .await
        .expect("an empty ranged journal read");
    assert!(
        beyond.is_empty(),
        "a range the journal never saw must come back empty, not invented"
    );
    host.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn journal_events_between_serves_a_recent_range_from_the_ring_without_reading_the_journal() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Terminal))
        .await
        .expect("a new hosted session");
    host.session()
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");

    let all = host.timeline_events().await.expect("timeline events");
    assert!(all.len() >= 3, "a completed turn appends lifecycle events");
    let first = all[1].seq;
    let last = all[all.len() - 2].seq;

    // The ring observed every one of these events synchronously as they were
    // emitted, well within its bound. Delete the durable journal now: if the
    // read below fell back to disk instead of the ring, it would fail
    // outright rather than quietly reading the wrong thing.
    let journal_path = host
        .paths()
        .expect("persistent paths")
        .journal(host.session().id())
        .expect("journal path");
    std::fs::remove_file(&journal_path).expect("the journal file is removable");

    let range = host
        .journal_events_between(first, last)
        .await
        .expect("a ring-served read that never touches the now-missing journal file");
    assert_eq!(
        range.iter().map(|event| event.seq).collect::<Vec<_>>(),
        all.iter()
            .map(|event| event.seq)
            .filter(|seq| (first..=last).contains(seq))
            .collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn journal_events_between_falls_back_to_the_journal_when_the_ring_cannot_cover_the_range() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Terminal))
        .await
        .expect("a new hosted session");
    let session_id = host.session().id().clone();
    host.session()
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");
    let all = host.timeline_events().await.expect("timeline events");
    assert!(all.len() >= 3, "a completed turn appends lifecycle events");
    let first = all[1].seq;
    let last = all[all.len() - 2].seq;
    host.shutdown().await.expect("a clean shutdown");

    // A resumed process starts with an empty in-memory ring: this range
    // predates it entirely, so only the durable journal has it.
    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session_id.clone()),
    )
    .await
    .expect("a resumed hosted session");
    let range = resumed
        .journal_events_between(first, last)
        .await
        .expect("a journal-fallback read after resume");
    assert_eq!(
        range.iter().map(|event| event.seq).collect::<Vec<_>>(),
        all.iter()
            .map(|event| event.seq)
            .filter(|seq| (first..=last).contains(seq))
            .collect::<Vec<_>>(),
        "the fallback must still return exactly the requested range"
    );
    resumed.shutdown().await.expect("a clean resumed shutdown");
}
