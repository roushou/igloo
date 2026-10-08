use std::sync::Arc;

use igloo_core::repo::{CommitId, Repo};
use igloo_core::snapshot::SnapshotId;
use igloo_core::{Actor, Entity, SystemComponent, ValidationErrors};

use super::pipeline::{Base, Pipeline};
use crate::app::{AppError, Command, CommandBus, RequestContext};
use crate::platform::{ImageImporter, RecordImage, Snapshots};
use crate::ports::{
    Forge, IdGenerator, IdGeneratorExt, ImageReference, InvalidReference, Platform,
};

/// A repository's pipeline at a commit, with the snapshot its checkout goes over.
#[derive(Clone, Debug)]
pub struct Environment {
    /// The pipeline.
    pub pipeline: Pipeline,
    /// The snapshot the checkout goes over: the imported image, a registered snapshot, or none.
    pub base: Option<SnapshotId>,
}

/// Reads repositories' pipelines and imports their base images, once per repository.
#[derive(Clone)]
pub struct Environments {
    forge: Arc<dyn Forge>,
    importer: ImageImporter,
    snapshots: Snapshots,
    bus: CommandBus,
    ids: Arc<dyn IdGenerator>,
}

impl Environments {
    /// Environments read through `forge`, importing images with `importer` into `snapshots`
    /// and recording them on repositories through `bus`.
    #[must_use]
    pub fn new(
        forge: Arc<dyn Forge>,
        importer: ImageImporter,
        snapshots: Snapshots,
        bus: CommandBus,
        ids: Arc<dyn IdGenerator>,
    ) -> Self {
        Self {
            forge,
            importer,
            snapshots,
            bus,
            ids,
        }
    }

    /// `repo`'s environment at `commit`, or why it has none: a missing or invalid pipeline.
    pub async fn at(
        &self,
        repo: &Repo,
        commit: &CommitId,
    ) -> Result<Result<Environment, String>, AppError> {
        let Some(text) = self
            .forge
            .read_file(repo.id(), commit, Pipeline::PATH)
            .await?
        else {
            return Ok(Err(format!("{} is missing", Pipeline::PATH)));
        };
        let pipeline = match std::str::from_utf8(&text)
            .map_err(|error| error.to_string())
            .and_then(|text| Pipeline::try_from(text).map_err(|errors| errors.to_string()))
        {
            Ok(pipeline) => pipeline,
            Err(reason) => return Ok(Err(format!("invalid pipeline: {reason}"))),
        };
        let base = match &pipeline.base {
            Base::None => None,
            Base::Snapshot(snapshot) => Some(*snapshot),
            Base::Image { image, platform } => {
                Some(self.image(repo, image, platform.as_deref()).await?)
            }
        };
        Ok(Ok(Environment { pipeline, base }))
    }

    /// The snapshot `image` was imported as for `repo`, importing it the first time.
    async fn image(
        &self,
        repo: &Repo,
        image: &str,
        platform: Option<&str>,
    ) -> Result<SnapshotId, AppError> {
        let invalid = |field: &str, error: InvalidReference| {
            AppError::Validation(ValidationErrors::single(field, error.to_string()))
        };
        let reference: ImageReference = image.parse().map_err(|error| invalid("image", error))?;
        let platform = match platform {
            Some(platform) => platform
                .parse()
                .map_err(|error| invalid("platform", error))?,
            None => Platform::host(),
        };
        let key = format!("{reference} {platform}");
        if let Some(snapshot) = repo.image(&key) {
            return Ok(snapshot);
        }
        let (layers, env) = self.importer.import(&reference, &platform).await?;
        let snapshot = self.snapshots.extend(None, layers, &env).await?;
        self.dispatch(RecordImage {
            repo: repo.id(),
            image: key,
            snapshot,
        })
        .await?;
        Ok(snapshot)
    }

    async fn dispatch<C: Command>(&self, command: C) -> Result<C::Output, AppError> {
        let context = RequestContext::new(
            Actor::System {
                component: SystemComponent::Controller,
            },
            self.ids.next(),
        );
        self.bus.dispatch(command, context).await
    }
}
