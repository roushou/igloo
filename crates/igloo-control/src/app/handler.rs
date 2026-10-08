use std::convert::Infallible;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use igloo_core::{Entity, ErrorCode, Id, Timestamp};

use super::{AppError, Command, CommandHandler, RequestContext};
use crate::ports::{Clock, EntityStore, IdGenerator, IdGeneratorExt, Versioned};

/// Handles a command aimed at one existing entity: load it, apply one change, commit.
///
/// A commit that loses a race is retried from a fresh load, so `change` may run more than once;
/// entity methods are pure, which makes that safe.
pub struct EntityHandler<E: Entity, C, T, F, Err> {
    store: Arc<dyn EntityStore<E>>,
    clock: Arc<dyn Clock>,
    target: T,
    change: F,
    _marker: PhantomData<fn(C) -> Err>,
}

/// Handles a command that creates an entity: generate its id, construct it, commit.
pub struct CreateHandler<E: Entity, C, F, Err> {
    store: Arc<dyn EntityStore<E>>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
    construct: F,
    _marker: PhantomData<fn(C) -> Err>,
}

impl<E, C, T, F, Err> EntityHandler<E, C, T, F, Err>
where
    E: Entity + Send + Sync + 'static,
    C: Command,
{
    /// How many times a change is attempted when commits lose races.
    pub const ATTEMPTS: usize = 3;

    /// A handler finding its entity with `target` and changing it with `change`.
    pub fn new(
        store: Arc<dyn EntityStore<E>>,
        clock: Arc<dyn Clock>,
        target: T,
        change: F,
    ) -> Self {
        Self {
            store,
            clock,
            target,
            change,
            _marker: PhantomData,
        }
    }
}

impl<E, C, T> EntityHandler<E, C, T, (), Infallible>
where
    E: Entity + Send + Sync + 'static,
    C: Command + Sync,
    T: Fn(&C) -> Id<E> + Send + Sync + 'static,
{
    /// A handler whose change cannot be rejected, such as `Sandbox::stop`.
    pub fn infallible<G>(
        store: Arc<dyn EntityStore<E>>,
        clock: Arc<dyn Clock>,
        target: T,
        change: G,
    ) -> impl CommandHandler<C>
    where
        G: Fn(&mut E, &C, Timestamp) -> C::Output + Send + Sync + 'static,
    {
        EntityHandler::new(
            store,
            clock,
            target,
            move |entity: &mut E, command: &C, now| {
                Ok::<_, Infallible>(change(entity, command, now))
            },
        )
    }
}

#[async_trait]
impl<E, C, T, F, Err> CommandHandler<C> for EntityHandler<E, C, T, F, Err>
where
    E: Entity + Send + Sync + 'static,
    C: Command + Sync,
    T: Fn(&C) -> Id<E> + Send + Sync + 'static,
    F: Fn(&mut E, &C, Timestamp) -> Result<C::Output, Err> + Send + Sync + 'static,
    Err: std::error::Error + ErrorCode + Send + Sync + 'static,
{
    async fn handle(&self, command: C, context: &RequestContext) -> Result<C::Output, AppError> {
        let id = (self.target)(&command);
        let mut attempt = 1;
        loop {
            let mut entity = self
                .store
                .load(id)
                .await?
                .ok_or_else(|| AppError::not_found(E::NAME, &id))?;
            let now = self.clock.now();
            let output = (self.change)(entity.entity_mut(), &command, now)
                .map_err(|error| AppError::domain(&error))?;
            match self
                .store
                .commit(&mut entity, &context.commit_meta(now))
                .await
            {
                Ok(()) => return Ok(output),
                Err(error) => match AppError::from(error) {
                    AppError::Conflict if attempt < Self::ATTEMPTS => attempt += 1,
                    other => return Err(other),
                },
            }
        }
    }
}

impl<E, C, F, Err> CreateHandler<E, C, F, Err>
where
    E: Entity + Send + Sync + 'static,
    C: Command<Output = Id<E>>,
{
    /// A handler building the entity with `construct`.
    pub fn new(
        store: Arc<dyn EntityStore<E>>,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGenerator>,
        construct: F,
    ) -> Self {
        Self {
            store,
            clock,
            ids,
            construct,
            _marker: PhantomData,
        }
    }
}

#[async_trait]
impl<E, C, F, Err> CommandHandler<C> for CreateHandler<E, C, F, Err>
where
    E: Entity + Send + Sync + 'static,
    C: Command<Output = Id<E>> + Sync,
    F: Fn(Id<E>, &C, Timestamp) -> Result<E, Err> + Send + Sync + 'static,
    Err: std::error::Error + ErrorCode + Send + Sync + 'static,
{
    async fn handle(&self, command: C, context: &RequestContext) -> Result<Id<E>, AppError> {
        let id = self.ids.next::<E>();
        let now = self.clock.now();
        let entity =
            (self.construct)(id, &command, now).map_err(|error| AppError::domain(&error))?;
        let mut entity = Versioned::new(entity);
        self.store
            .commit(&mut entity, &context.commit_meta(now))
            .await?;
        Ok(id)
    }
}
