use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::watch;

use crate::ports::{
    CommitMeta, EventEnvelope, EventId, EventLog, NewEvent, Sequence, StorageError,
};

/// The global event log in a vector. Memory stores append to it on commit.
#[derive(Debug)]
pub struct MemoryEventLog {
    events: Mutex<Vec<EventEnvelope>>,
    head: watch::Sender<Sequence>,
}

impl MemoryEventLog {
    /// An empty log.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(Vec::new()),
            head: watch::Sender::new(Sequence::START),
        })
    }

    /// Appends one entity's new events with consecutive sequences, then wakes consumers.
    pub(crate) fn append(
        &self,
        subject: &str,
        first_stream_version: u64,
        events: Vec<(EventId, NewEvent)>,
        meta: &CommitMeta,
    ) {
        let mut log = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut sequence = log.last().map_or(Sequence::START, |last| last.sequence);
        for (offset, (id, event)) in (0..).zip(events) {
            sequence = sequence.next();
            log.push(EventEnvelope {
                id,
                kind: event.kind.to_owned(),
                schema_version: event.schema_version,
                subject: subject.to_owned(),
                stream_version: first_stream_version + offset,
                sequence,
                time: meta.time,
                actor: meta.actor,
                correlation_id: meta.correlation_id,
                causation_id: meta.causation_id,
                data: event.data,
            });
        }
        drop(log);
        self.head.send_replace(sequence);
    }
}

#[async_trait]
impl EventLog for MemoryEventLog {
    async fn read(
        &self,
        after: Sequence,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StorageError> {
        let log = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(log
            .iter()
            .filter(|event| event.sequence > after)
            .take(limit)
            .cloned()
            .collect())
    }

    fn head(&self) -> watch::Receiver<Sequence> {
        self.head.subscribe()
    }
}
