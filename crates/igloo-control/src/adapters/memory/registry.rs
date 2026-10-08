use std::collections::HashMap;
use std::io::Write;

use async_trait::async_trait;
use igloo_core::Digest;
use igloo_core::snapshot::MediaType;

use crate::ports::{
    ImageReference, ImageRegistry, Platform, PulledImage, PulledLayer, RegistryError,
};

/// An image's layers, lowest first, with their encodings.
type Layers = Vec<(MediaType, Vec<u8>)>;

/// An image's environment, in order.
type Env = Vec<(String, String)>;

/// Images held in memory, keyed by reference and platform. Pulls write each layer to a
/// temporary file.
#[derive(Debug, Default)]
pub struct MemoryRegistry {
    images: HashMap<(ImageReference, Platform), (Layers, Env)>,
}

impl MemoryRegistry {
    /// Adds `image` for `platform` with `layers`, lowest first, and environment `env`.
    #[must_use]
    pub fn with_image(
        mut self,
        image: &ImageReference,
        platform: Platform,
        layers: Layers,
        env: Env,
    ) -> Self {
        self.images.insert((image.clone(), platform), (layers, env));
        self
    }
}

#[async_trait]
impl ImageRegistry for MemoryRegistry {
    async fn pull(
        &self,
        image: &ImageReference,
        platform: &Platform,
    ) -> Result<PulledImage, RegistryError> {
        let (layers, env) = match self.images.get(&(image.clone(), platform.clone())) {
            Some(image) => image,
            None if self.images.keys().any(|(known, _)| known == image) => {
                return Err(RegistryError::NoMatchingPlatform(platform.to_string()));
            }
            None => return Err(RegistryError::NotFound),
        };
        let layers = layers
            .iter()
            .map(|(media_type, bytes)| {
                let mut file = tempfile::NamedTempFile::new().map_err(transport)?;
                file.write_all(bytes).map_err(transport)?;
                Ok(PulledLayer {
                    media_type: *media_type,
                    digest: Digest::from_blake3(*blake3::hash(bytes).as_bytes()),
                    file: file.into_temp_path(),
                })
            })
            .collect::<Result<_, RegistryError>>()?;
        Ok(PulledImage {
            layers,
            env: env.clone(),
        })
    }
}

fn transport(error: impl std::fmt::Display) -> RegistryError {
    RegistryError::Transport(error.to_string())
}
