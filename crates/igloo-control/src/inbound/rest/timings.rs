use std::collections::HashMap;
use std::sync::Arc;

use igloo_core::Timestamp;
use igloo_core::repo::RepoEvent;
use tokio::sync::Mutex;

use crate::ports::{EventEnvelope, EventLog, Sequence, StorageError};

/// When a resource started and ended, as its events say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Span {
    /// When it started, if it did.
    pub started: Option<Timestamp>,
    /// When it ended, if it did: a job finishing, failing or being cancelled, a run's last check
    /// ending or its erroring, a change merging or closing.
    pub ended: Option<Timestamp>,
    /// Whether `ended` can no longer move.
    sealed: bool,
}

/// What the event log says about when things happened: the start and end of jobs, runs and
/// changes, and when warm snapshots were recorded. It reads the log once, then only what was
/// appended since, so a lookup costs a read of the new events.
///
///
/// Cost: the first lookup after startup reads the whole event log, and the index then keeps one
/// entry per job, run and change and per recorded warm snapshot for the life of the process, so
/// memory grows with the number of those resources.
///
/// Invariant: `index` reflects every event up to `index.after`, in sequence order.
pub(crate) struct Timings {
    log: Arc<dyn EventLog>,
    index: Mutex<Index>,
}

#[derive(Default)]
struct Index {
    after: Sequence,
    spans: HashMap<String, Span>,
    built: HashMap<(String, String), Timestamp>,
}

impl Timings {
    const PAGE: usize = 1000;

    /// Timings over `log`.
    pub(crate) fn new(log: Arc<dyn EventLog>) -> Self {
        Self {
            log,
            index: Mutex::new(Index::default()),
        }
    }

    /// The span of the resource `subject` (`job_...`, `run_...`, `chg_...`).
    pub(crate) async fn span(&self, subject: &str) -> Result<Span, StorageError> {
        let index = self.current().await?;
        Ok(index.spans.get(subject).copied().unwrap_or_default())
    }

    /// When repository `repo` last recorded the warm snapshot of `key`.
    pub(crate) async fn built(
        &self,
        repo: &str,
        key: &str,
    ) -> Result<Option<Timestamp>, StorageError> {
        let index = self.current().await?;
        Ok(index.built.get(&(repo.to_owned(), key.to_owned())).copied())
    }

    /// The index, caught up with the log.
    async fn current(&self) -> Result<tokio::sync::MutexGuard<'_, Index>, StorageError> {
        let mut index = self.index.lock().await;
        loop {
            let events = self.log.read(index.after, Self::PAGE).await?;
            let Some(last) = events.last().map(|event| event.sequence) else {
                return Ok(index);
            };
            for event in &events {
                index.apply(event);
            }
            index.after = last;
        }
    }
}

impl Index {
    fn apply(&mut self, event: &EventEnvelope) {
        match event.kind.as_str() {
            "igloo.job.started" => {
                self.spans.entry(event.subject.clone()).or_default().started = Some(event.time);
            }
            "igloo.job.finished"
            | "igloo.job.failed"
            | "igloo.job.cancelled"
            | "igloo.change.merged"
            | "igloo.change.closed" => {
                let span = self.spans.entry(event.subject.clone()).or_default();
                span.ended.get_or_insert(event.time);
            }
            "igloo.run.check_ended" => {
                let span = self.spans.entry(event.subject.clone()).or_default();
                if !span.sealed {
                    span.ended = Some(event.time);
                }
            }
            "igloo.run.errored" => {
                let span = self.spans.entry(event.subject.clone()).or_default();
                span.ended = Some(event.time);
                span.sealed = true;
            }
            "igloo.repo.warm_snapshot_recorded" => {
                if let Ok(RepoEvent::WarmSnapshotRecorded { key, .. }) = event.decode() {
                    self.built
                        .insert((event.subject.clone(), key.to_string()), event.time);
                }
            }
            _ => {}
        }
    }
}
