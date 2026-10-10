use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use futures_util::stream;
use igloo_api::event::EventNotice;
use igloo_api::problem::Problem;
use igloo_core::build::Build;
use igloo_core::change::Change;
use igloo_core::repo::RepoId;
use igloo_core::workspace::Workspace;
use igloo_core::{Entity, Id, Prefixed, Resource as _, ValidationErrors, Validator};
use tokio::sync::watch;

use super::auth::Caller;
use super::{ApiError, ApiState};
use crate::agents::Task;
use crate::app::{AppError, InstallError, PlatformBuilder};
use crate::ci::Run;
use crate::ports::{EntityStore, EventEnvelope, EventLog, Sequence, StorageError};

/// Streams a notice per committed event as server-sent events: `id` is the event's sequence,
/// `event` its kind, `data` an `EventNotice`. `repo` keeps only events of that repository's
/// resources. `after`, or else `Last-Event-ID`, resumes after that sequence; without either the stream
/// starts at the log's current head and sends only events committed after it opened, and
/// `after=0` replays everything. Events are sent in sequence order, each once. Comments keep the connection alive every 15 seconds.
#[utoipa::path(
    get,
    path = "/v1/events",
    tag = "events",
    params(
        ("repo" = Option<String>, Query, description = "Only events of this repository's resources"),
        ("after" = Option<u64>, Query, description = "Resume after this sequence; 0 replays the whole log; the current head when absent"),
        ("Last-Event-ID" = Option<u64>, Header, description = "Resume after this sequence when `after` is absent"),
    ),
    responses(
        (status = 200, content_type = "text/event-stream", body = EventNotice),
        (status = 422, body = Problem),
    )
)]
pub(super) async fn stream(
    State(state): State<ApiState>,
    _: Caller,
    Query(params): Query<Vec<(String, String)>>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let filter = EventFilter::try_from((params, last_event_id)).map_err(AppError::from)?;
    let head = state.events.head();
    let after = filter.after.unwrap_or_else(|| *head.borrow());
    let feed = EventFeed {
        head,
        after,
        log: Arc::clone(&state.events),
        repos: state.event_repos.clone(),
        filter,
        pending: VecDeque::new(),
    };
    Ok(Sse::new(stream::unfold(feed, EventFeed::next)).keep_alive(KeepAlive::default()))
}

/// Which events a client wants, validated.
struct EventFilter {
    repo: Option<RepoId>,
    after: Option<Sequence>,
}

impl TryFrom<(Vec<(String, String)>, Option<String>)> for EventFilter {
    type Error = ValidationErrors;

    fn try_from(
        (params, last_event_id): (Vec<(String, String)>, Option<String>),
    ) -> Result<Self, Self::Error> {
        let mut repo = Ok(None);
        let mut after = None;
        for (key, value) in params {
            match key.as_str() {
                "repo" => {
                    repo = value
                        .parse::<RepoId>()
                        .map(Some)
                        .map_err(|error| error.to_string());
                }
                "after" => after = Some(value),
                _ => {}
            }
        }
        let after = after
            .map(|value| ("after", value))
            .or_else(|| last_event_id.map(|value| ("Last-Event-ID", value)))
            .map_or(Ok(None), |(name, value)| {
                value
                    .parse::<u64>()
                    .map(|value| Some(Sequence::from(value)))
                    .map_err(|_| format!("{name} must be an event sequence"))
            });
        let (repo, after) = Validator::new()
            .field("repo", repo)
            .field("after", after)
            .finish()?;
        Ok(Self { repo, after })
    }
}

/// The kinds of resource whose events clients see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResourceKind {
    Repo,
    Change,
    Task,
    Run,
    Build,
    Outcome,
    Job,
    Sandbox,
    Seal,
    Worker,
    Workspace,
}

impl ResourceKind {
    /// The kind of the entity `subject` (`<prefix>_<id>`) names.
    fn of_subject(subject: &str) -> Option<Self> {
        subject.split_once('_')?.0.parse().ok()
    }
}

impl FromStr for ResourceKind {
    type Err = ();

    fn from_str(prefix: &str) -> Result<Self, ()> {
        Ok(match prefix {
            "repo" => Self::Repo,
            "chg" => Self::Change,
            "task" => Self::Task,
            "run" => Self::Run,
            "bld" => Self::Build,
            "out" => Self::Outcome,
            "job" => Self::Job,
            "sbx" => Self::Sandbox,
            "seal" => Self::Seal,
            "wrk" => Self::Worker,
            "wsp" => Self::Workspace,
            _ => return Err(()),
        })
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Repo => "repo",
            Self::Change => "change",
            Self::Task => "task",
            Self::Run => "run",
            Self::Build => "build",
            Self::Outcome => "outcome",
            Self::Job => "job",
            Self::Sandbox => "sandbox",
            Self::Seal => "seal",
            Self::Worker => "worker",
            Self::Workspace => "workspace",
        })
    }
}

/// Finds the repository a resource belongs to. Repositories, changes, tasks, runs, builds and
/// workspaces have one; the rest do not. A repository found for a resource is remembered, as it never
/// changes and the memory holds at most `MAX_KNOWN` resources, so a miss costs one load.
#[derive(Clone)]
pub(crate) struct EventRepos {
    changes: Arc<dyn EntityStore<Change>>,
    tasks: Arc<dyn EntityStore<Task>>,
    runs: Arc<dyn EntityStore<Run>>,
    builds: Arc<dyn EntityStore<Build>>,
    workspaces: Arc<dyn EntityStore<Workspace>>,
    known: Arc<Mutex<HashMap<String, RepoId>>>,
}

impl EventRepos {
    /// How many resources are remembered before the memory is cleared.
    const MAX_KNOWN: usize = 10_000;

    /// Lookups over the stores `platform` was given.
    pub(crate) fn new(platform: &PlatformBuilder) -> Result<Self, InstallError> {
        Ok(Self {
            changes: platform.store::<Change>()?,
            tasks: platform.store::<Task>()?,
            runs: platform.store::<Run>()?,
            builds: platform.store::<Build>()?,
            workspaces: platform.store::<Workspace>()?,
            known: Arc::default(),
        })
    }

    /// The repository of the resource `subject` names, if it has one.
    async fn of(&self, kind: ResourceKind, subject: &str) -> Result<Option<RepoId>, StorageError> {
        if let Some(repo) = self.remembered(subject) {
            return Ok(Some(repo));
        }
        let repo = match kind {
            ResourceKind::Repo => subject.parse().ok(),
            ResourceKind::Change => Self::load(&*self.changes, subject, Change::repo).await?,
            ResourceKind::Task => Self::load(&*self.tasks, subject, Task::repo).await?,
            ResourceKind::Run => Self::load(&*self.runs, subject, Run::repo).await?,
            ResourceKind::Build => {
                Self::load(&*self.builds, subject, |build| build.spec().repo).await?
            }
            ResourceKind::Workspace => {
                Self::load(&*self.workspaces, subject, |workspace| {
                    workspace.spec().repo
                })
                .await?
            }
            ResourceKind::Outcome
            | ResourceKind::Job
            | ResourceKind::Sandbox
            | ResourceKind::Seal
            | ResourceKind::Worker => None,
        };
        if let Some(repo) = repo {
            self.remember(subject, repo);
        }
        Ok(repo)
    }

    fn remember(&self, subject: &str, repo: RepoId) {
        let mut known = self
            .known
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if known.len() >= Self::MAX_KNOWN {
            known.clear();
        }
        known.insert(subject.to_owned(), repo);
    }

    fn remembered(&self, subject: &str) -> Option<RepoId> {
        self.known
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(subject)
            .copied()
    }

    async fn load<E>(
        store: &dyn EntityStore<E>,
        subject: &str,
        repo: impl Fn(&E) -> RepoId,
    ) -> Result<Option<RepoId>, StorageError>
    where
        E: Entity + Prefixed + Send + Sync + 'static,
    {
        let Ok(id) = subject.parse::<Id<E>>() else {
            return Ok(None);
        };
        Ok(store.load(id).await?.map(|loaded| repo(loaded.entity())))
    }
}

/// An event with the resource it belongs to, ready to send.
struct ResolvedEvent<'a> {
    envelope: &'a EventEnvelope,
    kind: ResourceKind,
    repo: Option<RepoId>,
}

impl From<ResolvedEvent<'_>> for EventNotice {
    fn from(event: ResolvedEvent<'_>) -> Self {
        Self::new(
            event.envelope.sequence.get(),
            event.envelope.kind.clone(),
            event.kind.to_string(),
            event.envelope.subject.clone(),
            event.repo.map(|repo| repo.to_string()),
            event.envelope.time,
        )
    }
}

/// The state of one event stream: reads the log after `after` and follows its head.
struct EventFeed {
    log: Arc<dyn EventLog>,
    head: watch::Receiver<Sequence>,
    repos: EventRepos,
    filter: EventFilter,
    after: Sequence,
    pending: VecDeque<EventNotice>,
}

impl EventFeed {
    const PAGE: usize = 256;

    async fn next(mut self) -> Option<(Result<Event, Infallible>, Self)> {
        loop {
            if let Some(notice) = self.pending.pop_front() {
                let data = serde_json::to_string(&notice).unwrap_or_default();
                let event = Event::default()
                    .id(notice.sequence.to_string())
                    .event(notice.kind)
                    .data(data);
                return Some((Ok(event), self));
            }
            let events = match self.log.read(self.after, Self::PAGE).await {
                Ok(events) => events,
                Err(error) => {
                    tracing::warn!(%error, "reading the event log failed");
                    return None;
                }
            };
            if events.is_empty() {
                // The receiver notes the head it last saw, so an append since the read
                // above wakes this at once.
                self.head.changed().await.ok()?;
                continue;
            }
            for envelope in &events {
                self.after = envelope.sequence;
                if let Err(error) = self.enqueue(envelope).await {
                    tracing::warn!(%error, "resolving an event's repository failed");
                    return None;
                }
            }
        }
    }

    async fn enqueue(&mut self, envelope: &EventEnvelope) -> Result<(), StorageError> {
        let Some(kind) = ResourceKind::of_subject(&envelope.subject) else {
            return Ok(());
        };
        let repo = self.repos.of(kind, &envelope.subject).await?;
        if self.filter.repo.is_some() && repo != self.filter.repo {
            return Ok(());
        }
        self.pending.push_back(
            ResolvedEvent {
                envelope,
                kind,
                repo,
            }
            .into(),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use igloo_core::Id;
    use uuid::Uuid;

    use super::EventRepos;
    use crate::testing::MemoryPlatform;

    #[test]
    fn the_remembered_repositories_are_bounded() {
        let MemoryPlatform { builder, .. } = MemoryPlatform::new();
        let repos = EventRepos::new(&builder).expect("repos");
        let repo = Id::from_uuid(Uuid::from_u128(1));
        for n in 0..=EventRepos::MAX_KNOWN * 2 {
            repos.remember(&format!("task_{n}"), repo);
        }
        let known = repos.known.lock().expect("lock").len();
        assert!(known <= EventRepos::MAX_KNOWN, "{known}");
        assert!(
            repos
                .remembered(&format!("task_{}", EventRepos::MAX_KNOWN * 2))
                .is_some()
        );
    }
}
