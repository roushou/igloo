use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use tower::util::BoxCloneSyncService;
use tower::{Layer, Service, ServiceBuilder, ServiceExt};
use tracing::Instrument;

use super::{AppError, Command, CommandHandler, InstallError, RequestContext};
use crate::ports::{Authorization, PolicyEngine};

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type Route<C> = BoxCloneSyncService<Dispatch<C>, <C as Command>::Output, AppError>;

/// The only write path: routes each command type to its handler through the same layers
/// (tracing, timeout, authorization).
///
/// Cheap to clone. A handle obtained before the platform is built dispatches once it is.
#[derive(Clone)]
pub struct CommandBus {
    routes: Arc<OnceLock<HashMap<TypeId, Box<dyn Any + Send + Sync>>>>,
}

/// One command on its way through the bus.
pub struct Dispatch<C> {
    /// The command.
    pub command: C,
    /// Who sent it.
    pub context: RequestContext,
}

/// Collects handlers, then freezes them into the [`CommandBus`].
pub struct CommandBusBuilder {
    bus: CommandBus,
    routes: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    policy: Arc<dyn PolicyEngine>,
    timeout: Duration,
}

impl CommandBus {
    /// Runs `command` through its handler's pipeline.
    pub async fn dispatch<C: Command>(
        &self,
        command: C,
        context: RequestContext,
    ) -> Result<C::Output, AppError> {
        let route = self
            .routes
            .get()
            .and_then(|routes| routes.get(&TypeId::of::<C>()))
            .and_then(|route| route.downcast_ref::<Route<C>>())
            .cloned()
            .ok_or(AppError::Unregistered { command: C::NAME })?;
        route.oneshot(Dispatch { command, context }).await
    }
}

impl CommandBusBuilder {
    /// The default time a command may take.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

    /// A builder authorizing through `policy` and bounding each command by `timeout`.
    #[must_use]
    pub fn new(policy: Arc<dyn PolicyEngine>, timeout: Duration) -> Self {
        Self {
            bus: CommandBus {
                routes: Arc::new(OnceLock::new()),
            },
            routes: HashMap::new(),
            policy,
            timeout,
        }
    }

    /// A handle that dispatches once [`CommandBusBuilder::build`] runs; for reconcilers and
    /// reactors created before the bus is complete.
    #[must_use]
    pub fn handle(&self) -> CommandBus {
        self.bus.clone()
    }

    /// Registers the handler for `C`.
    pub fn register<C: Command>(
        &mut self,
        handler: impl CommandHandler<C>,
    ) -> Result<(), InstallError> {
        let type_id = TypeId::of::<C>();
        if self.routes.contains_key(&type_id) {
            return Err(InstallError::DuplicateCommand(C::NAME));
        }
        let service = ServiceBuilder::new()
            .layer(TraceLayer)
            .layer(TimeoutLayer(self.timeout))
            .layer(AuthorizeLayer(Arc::clone(&self.policy)))
            .service(HandlerService(Arc::new(handler)));
        let route: Route<C> = BoxCloneSyncService::new(service);
        self.routes.insert(type_id, Box::new(route));
        Ok(())
    }

    /// Freezes the routes. Every handle dispatches from now on.
    #[must_use]
    pub fn build(self) -> CommandBus {
        // A builder is consumed by its only `build`, so the cell is always empty here.
        let _ = self.bus.routes.set(self.routes);
        self.bus
    }
}

/// Calls the registered handler.
struct HandlerService<H>(Arc<H>);

impl<H> Clone for HandlerService<H> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<C: Command, H: CommandHandler<C>> Service<Dispatch<C>> for HandlerService<H> {
    type Response = C::Output;
    type Error = AppError;
    type Future = BoxFuture<Result<C::Output, AppError>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), AppError>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, dispatch: Dispatch<C>) -> Self::Future {
        let handler = Arc::clone(&self.0);
        Box::pin(async move { handler.handle(dispatch.command, &dispatch.context).await })
    }
}

/// Opens a span naming the command, actor and correlation id.
struct TraceLayer;

#[derive(Clone)]
struct TraceService<S>(S);

impl<S> Layer<S> for TraceLayer {
    type Service = TraceService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TraceService(inner)
    }
}

impl<C, S> Service<Dispatch<C>> for TraceService<S>
where
    C: Command,
    S: Service<Dispatch<C>, Response = C::Output, Error = AppError>,
    S::Future: Send + 'static,
{
    type Response = C::Output;
    type Error = AppError;
    type Future = BoxFuture<Result<C::Output, AppError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), AppError>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, dispatch: Dispatch<C>) -> Self::Future {
        let span = tracing::info_span!(
            "command",
            name = C::NAME,
            actor = ?dispatch.context.actor,
            correlation_id = %dispatch.context.correlation_id,
        );
        Box::pin(self.0.call(dispatch).instrument(span))
    }
}

/// Fails the command with `command.timeout` when it runs too long.
struct TimeoutLayer(Duration);

#[derive(Clone)]
struct TimeoutService<S> {
    inner: S,
    limit: Duration,
}

impl<S> Layer<S> for TimeoutLayer {
    type Service = TimeoutService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TimeoutService {
            inner,
            limit: self.0,
        }
    }
}

impl<C, S> Service<Dispatch<C>> for TimeoutService<S>
where
    C: Command,
    S: Service<Dispatch<C>, Response = C::Output, Error = AppError>,
    S::Future: Send + 'static,
{
    type Response = C::Output;
    type Error = AppError;
    type Future = BoxFuture<Result<C::Output, AppError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), AppError>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, dispatch: Dispatch<C>) -> Self::Future {
        let future = self.inner.call(dispatch);
        let limit = C::TIMEOUT.unwrap_or(self.limit);
        Box::pin(async move {
            tokio::time::timeout(limit, future)
                .await
                .map_err(|_| AppError::Timeout { command: C::NAME })?
        })
    }
}

/// Asks the policy engine before the handler runs.
struct AuthorizeLayer(Arc<dyn PolicyEngine>);

#[derive(Clone)]
struct AuthorizeService<S> {
    inner: S,
    policy: Arc<dyn PolicyEngine>,
}

impl<S> Layer<S> for AuthorizeLayer {
    type Service = AuthorizeService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthorizeService {
            inner,
            policy: Arc::clone(&self.0),
        }
    }
}

impl<C, S> Service<Dispatch<C>> for AuthorizeService<S>
where
    C: Command,
    S: Service<Dispatch<C>, Response = C::Output, Error = AppError>,
    S::Future: Send + 'static,
{
    type Response = C::Output;
    type Error = AppError;
    type Future = BoxFuture<Result<C::Output, AppError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), AppError>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, dispatch: Dispatch<C>) -> Self::Future {
        let decision = self.policy.authorize(&Authorization {
            actor: &dispatch.context.actor,
            command: C::NAME,
        });
        match decision {
            Ok(()) => Box::pin(self.inner.call(dispatch)),
            Err(denied) => Box::pin(async move { Err(AppError::Forbidden(denied)) }),
        }
    }
}
