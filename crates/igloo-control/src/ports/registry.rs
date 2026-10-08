use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use igloo_core::Digest;
use igloo_core::snapshot::MediaType;
use tempfile::TempPath;

/// Where container images are pulled from.
#[async_trait]
pub trait ImageRegistry: Send + Sync {
    /// Downloads every layer of `image` for `platform`, lowest first, each verified against
    /// the registry's digest and rehashed with blake3, and reads the image's environment.
    /// Layer files are removed when dropped.
    async fn pull(
        &self,
        image: &ImageReference,
        platform: &Platform,
    ) -> Result<PulledImage, RegistryError>;
}

/// A downloaded image.
#[derive(Debug)]
pub struct PulledImage {
    /// Its layers, lowest first.
    pub layers: Vec<PulledLayer>,
    /// Its configured environment (`Env`), in order, as `KEY=VALUE` pairs.
    pub env: Vec<(String, String)>,
}

/// One downloaded image layer.
#[derive(Debug)]
pub struct PulledLayer {
    /// How the layer is encoded.
    pub media_type: MediaType,
    /// The blake3 digest of the file.
    pub digest: Digest,
    /// The downloaded file; deleted when dropped.
    pub file: TempPath,
}

/// An image reference: `[registry/]repository[:tag|@digest]`, Docker Hub when no registry is
/// named.
///
/// Invariant: `repository` is non-empty; `reference` is a tag or a `sha256:` digest.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageReference {
    registry: String,
    repository: String,
    reference: String,
}

/// An `os/architecture` pair, as used in image indexes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Platform {
    os: String,
    architecture: String,
}

/// Why an image could not be pulled.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// The image or tag does not exist.
    #[error("image not found")]
    NotFound,
    /// The image has no variant for the requested platform.
    #[error("the image has no {0} variant")]
    NoMatchingPlatform(String),
    /// A layer uses an encoding Igloo cannot store.
    #[error("unsupported layer media type {0}")]
    UnsupportedMediaType(String),
    /// Downloaded bytes do not match the registry's digest.
    #[error("layer {0} does not match its digest")]
    DigestMismatch(String),
    /// The registry could not be reached or answered unexpectedly.
    #[error("registry failure: {0}")]
    Transport(String),
}

/// Why an image reference or platform is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidReference(&'static str);

impl ImageReference {
    const DOCKER_HUB: &'static str = "docker.io";

    /// The registry host, such as `ghcr.io`; Docker Hub is `docker.io`.
    #[must_use]
    pub fn registry(&self) -> &str {
        &self.registry
    }

    /// The host serving the registry API: Docker Hub's lives at `registry-1.docker.io`.
    #[must_use]
    pub fn api_host(&self) -> &str {
        if self.registry == Self::DOCKER_HUB {
            "registry-1.docker.io"
        } else {
            &self.registry
        }
    }

    /// The repository, such as `library/rust`.
    #[must_use]
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// The tag or digest.
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }
}

impl FromStr for ImageReference {
    type Err = InvalidReference;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s.chars().any(char::is_whitespace) {
            return Err(InvalidReference(
                "an image reference has no spaces and is not empty",
            ));
        }
        let (name, reference) = if let Some((name, digest)) = s.split_once('@') {
            (name, digest.to_owned())
        } else {
            match s.rsplit_once(':') {
                Some((name, tag)) if !tag.contains('/') => (name, tag.to_owned()),
                _ => (s, "latest".to_owned()),
            }
        };
        let (registry, repository) = match name.split_once('/') {
            Some((host, rest))
                if host.contains('.') || host.contains(':') || host == "localhost" =>
            {
                (host.to_owned(), rest.to_owned())
            }
            _ => (Self::DOCKER_HUB.to_owned(), name.to_owned()),
        };
        let repository = if registry == Self::DOCKER_HUB && !repository.contains('/') {
            format!("library/{repository}")
        } else {
            repository
        };
        if repository.is_empty() || reference.is_empty() {
            return Err(InvalidReference("an image reference needs a repository"));
        }
        Ok(Self {
            registry,
            repository,
            reference,
        })
    }
}

impl fmt::Display for ImageReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let separator = if self.reference.contains(':') {
            '@'
        } else {
            ':'
        };
        write!(
            f,
            "{}/{}{separator}{}",
            self.registry, self.repository, self.reference
        )
    }
}

impl Platform {
    /// The platform of the machine running this code.
    #[must_use]
    pub fn host() -> Self {
        let architecture = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86_64" => "amd64",
            other => other,
        };
        Self {
            os: "linux".to_owned(),
            architecture: architecture.to_owned(),
        }
    }

    /// The operating system, such as `linux`.
    #[must_use]
    pub fn os(&self) -> &str {
        &self.os
    }

    /// The architecture, such as `arm64`.
    #[must_use]
    pub fn architecture(&self) -> &str {
        &self.architecture
    }
}

impl FromStr for Platform {
    type Err = InvalidReference;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split_once('/') {
            Some((os, architecture)) if !os.is_empty() && !architecture.is_empty() => Ok(Self {
                os: os.to_owned(),
                architecture: architecture.to_owned(),
            }),
            _ => Err(InvalidReference(
                "a platform is os/architecture, such as linux/arm64",
            )),
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.os, self.architecture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> (String, String, String) {
        let image: ImageReference = s.parse().expect("valid");
        (
            image.registry().to_owned(),
            image.repository().to_owned(),
            image.reference().to_owned(),
        )
    }

    #[test]
    fn docker_hub_short_names_expand() {
        assert_eq!(
            parse("rust:1.99"),
            ("docker.io".into(), "library/rust".into(), "1.99".into())
        );
        assert_eq!(
            parse("alpine"),
            ("docker.io".into(), "library/alpine".into(), "latest".into())
        );
    }

    #[test]
    fn other_registries_and_digests_are_kept() {
        assert_eq!(
            parse("ghcr.io/owner/tool:v1"),
            ("ghcr.io".into(), "owner/tool".into(), "v1".into())
        );
        assert_eq!(
            parse("localhost:5000/team/app@sha256:abc"),
            (
                "localhost:5000".into(),
                "team/app".into(),
                "sha256:abc".into()
            )
        );
        assert!("".parse::<ImageReference>().is_err());
        assert!("linux".parse::<Platform>().is_err());
    }
}
