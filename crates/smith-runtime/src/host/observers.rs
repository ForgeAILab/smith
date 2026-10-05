use super::*;

/// Connects the checkpoint wrapper built during factory preflight to the
/// journal opened immediately before session start.
#[derive(Debug, Default)]
pub(super) struct JournalCheckpointBarrier {
    journal: RwLock<Option<Arc<EventJournal>>>,
}

impl JournalCheckpointBarrier {
    pub(super) fn install(&self, journal: Arc<EventJournal>) -> Result<(), RuntimeError> {
        let mut target = self
            .journal
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if target.is_some() {
            return Err(RuntimeError::conflict(
                "checkpoint journal barrier was already installed",
            ));
        }
        *target = Some(journal);
        Ok(())
    }
}

#[async_trait]
impl CheckpointBarrier for JournalCheckpointBarrier {
    async fn before_checkpoint(&self, checkpoint: &TurnCheckpoint) -> Result<(), RuntimeError> {
        let journal = self
            .journal
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| {
                RuntimeError::internal(
                    "checkpoint journal barrier is unavailable before session start",
                )
            })?;
        journal
            .flush_before(checkpoint.watermark.event_sequence)
            .await
    }
}

#[derive(Debug)]
pub(super) struct ChangeTurnObserver(pub(super) Arc<smith_tools::ChangeRecorder>);

impl EventObserver for ChangeTurnObserver {
    fn observe(&self, event: &EventEnvelope) {
        match event.payload {
            RuntimeEvent::TurnStarted => self.0.start_turn(),
            RuntimeEvent::TurnCompleted { .. } => {
                let _ = self.0.finish_turn();
            }
            _ => {}
        }
    }
}

/// An observer installed before runtime construction and bound before start.
#[derive(Debug, Default)]
pub(super) struct DeferredObserver {
    target: RwLock<Option<Arc<dyn EventObserver>>>,
}

impl DeferredObserver {
    pub(super) fn install(&self, observer: Arc<dyn EventObserver>) -> Result<(), RuntimeError> {
        let mut target = self
            .target
            .write()
            .map_err(|_| RuntimeError::internal("journal observer state is poisoned"))?;
        if target.is_some() {
            return Err(RuntimeError::internal(
                "journal observer was installed more than once",
            ));
        }
        *target = Some(observer);
        Ok(())
    }
}

impl EventObserver for DeferredObserver {
    fn observe(&self, event: &EventEnvelope) {
        let target = self.target.read().ok().and_then(|target| target.clone());
        if let Some(target) = target {
            target.observe(event);
        }
    }
}

/// How many recent event envelopes [`EventRing`] retains.
///
/// Token deltas are coalesced before emission now, so a real burst is on the
/// order of tens of events per millisecond rather than the 1046-in-one-
/// millisecond burst that first overran the journal and broadcast channel
/// (see [`crate::journal::JournalConfig`]'s default). A gap is queried the
/// instant it is detected, not minutes later, so this bound only has to
/// outlast the time between a burst landing and the lagged subscriber asking
/// for it — a few thousand envelopes is generous headroom for that, without
/// keeping unbounded session history in memory. The case this bound does not
/// cover — a gap that predates process start on a resumed session, or a
/// session old enough to have scrolled the range out — is exactly what the
/// journal-read fallback in `Host::journal_events_between` exists for.
const EVENT_RING_CAPACITY: usize = 4096;

/// A bounded, in-memory record of the most recently emitted session events.
///
/// `Host::journal_events_between` used to answer every lagged-subscriber gap
/// by fsyncing and re-parsing the entire multi-megabyte journal file, on the
/// TUI's own event-loop task. That await starved the subscriber and the
/// journal writer alike, which produced the *next* gap — a self-reinforcing
/// cascade. This ring is populated by an observer installed the same way as
/// the journal's (see `start`, beside where the journal's `DeferredObserver`
/// slot is installed), so it sees the same synchronous, pre-broadcast event
/// stream the journal does, and answers the common case — a lag that just
/// happened — from memory instead of disk.
#[derive(Debug, Default)]
pub(super) struct EventRing {
    events: Mutex<VecDeque<EventEnvelope>>,
}

impl EventRing {
    /// Returns the requested inclusive range when every event in it is still
    /// held, `None` when any part of the range has been evicted or was never
    /// observed — the signal for the caller to fall back to the journal.
    ///
    /// Events come back raw, not redacted: redaction is a JSON round trip,
    /// deliberately kept off the synchronous `observe` hot path (see there),
    /// so it is the caller's job to redact before this reaches a display.
    pub(super) fn events_between(&self, first: u64, last: u64) -> Option<Vec<EventEnvelope>> {
        let events = self.events.lock().expect("event ring lock poisoned");
        let oldest = events.front()?.seq;
        let newest = events.back()?.seq;
        if first < oldest || last > newest {
            return None;
        }
        Some(
            events
                .iter()
                .filter(|event| event.seq >= first && event.seq <= last)
                .cloned()
                .collect(),
        )
    }
}

impl EventObserver for EventRing {
    fn observe(&self, event: &EventEnvelope) {
        // Non-blocking by construction, matching the journal observer this
        // is installed beside: a clone and a bounded push, nothing that can
        // stall the runtime's emission path. Redaction is deliberately not
        // done here — see `events_between` — so a burst of arrivals never
        // pays for a JSON round trip on this synchronous path.
        let mut events = self.events.lock().expect("event ring lock poisoned");
        events.push_back(event.clone());
        if events.len() > EVENT_RING_CAPACITY {
            events.pop_front();
        }
    }
}
