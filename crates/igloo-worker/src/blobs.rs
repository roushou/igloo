use async_trait::async_trait;
use igloo_core::Digest;
use igloo_core::snapshot::SnapshotLayer;
use igloo_worker_protocol::{ProtocolError, v1};
use reqwest::Url;
use tokio::io::AsyncWriteExt;

/// Where the worker downloads snapshot layers from.
#[async_trait]
pub trait BlobSource: Send + Sync {
    /// Writes the bytes of `layer` to `file`. Callers verify them against its digest.
    async fn download(
        &self,
        layer: &RemoteLayer,
        file: &mut tokio::fs::File,
    ) -> Result<(), BlobSourceError>;

    /// Uploads `file` to the presigned `url`.
    async fn upload(&self, url: &Url, file: tokio::fs::File) -> Result<(), BlobSourceError>;
}

/// A snapshot layer and the presigned URL it downloads from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteLayer {
    layer: SnapshotLayer,
    url: Url,
}

/// Why a blob could not be downloaded.
#[derive(Debug, thiserror::Error)]
pub enum BlobSourceError {
    /// No blob is stored under the digest.
    #[error("no blob {0}")]
    NotFound(Digest),
    /// The transfer failed.
    #[error("blob transfer failed: {0}")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Downloads layers from their presigned URLs; the URLs carry the credentials.
pub struct HttpBlobSource {
    client: reqwest::Client,
}

impl RemoteLayer {
    /// `layer`, downloadable from `url`.
    #[must_use]
    pub const fn new(layer: SnapshotLayer, url: Url) -> Self {
        Self { layer, url }
    }

    /// The layer.
    #[must_use]
    pub const fn layer(&self) -> &SnapshotLayer {
        &self.layer
    }

    /// Where to download it.
    #[must_use]
    pub const fn url(&self) -> &Url {
        &self.url
    }
}

impl TryFrom<&v1::SnapshotLayer> for RemoteLayer {
    type Error = ProtocolError;

    fn try_from(message: &v1::SnapshotLayer) -> Result<Self, Self::Error> {
        let url = message
            .url
            .parse::<Url>()
            .map_err(|error| ProtocolError::Invalid {
                field: "url",
                reason: error.to_string(),
            })?;
        Ok(Self::new(SnapshotLayer::try_from(message)?, url))
    }
}

impl HttpBlobSource {
    /// A source over HTTP and HTTPS. Installs `ring` as the process's TLS crypto provider
    /// unless one is installed already.
    pub fn new() -> Result<Self, BlobSourceError> {
        // Fails only when a provider is already installed, which serves equally well.
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
        let client = reqwest::Client::builder()
            .build()
            .map_err(|error| BlobSourceError::Transport(Box::new(error)))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl BlobSource for HttpBlobSource {
    async fn download(
        &self,
        layer: &RemoteLayer,
        file: &mut tokio::fs::File,
    ) -> Result<(), BlobSourceError> {
        let mut response = self
            .client
            .get(layer.url().clone())
            .send()
            .await
            .map_err(|error| BlobSourceError::Transport(Box::new(error)))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(BlobSourceError::NotFound(layer.layer().digest()));
        }
        if let Err(error) = response.error_for_status_ref() {
            return Err(BlobSourceError::Transport(Box::new(error)));
        }
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| BlobSourceError::Transport(Box::new(error)))?
        {
            file.write_all(&chunk)
                .await
                .map_err(|error| BlobSourceError::Transport(Box::new(error)))?;
        }
        file.flush()
            .await
            .map_err(|error| BlobSourceError::Transport(Box::new(error)))
    }

    async fn upload(&self, url: &Url, file: tokio::fs::File) -> Result<(), BlobSourceError> {
        let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file));
        self.client
            .put(url.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| BlobSourceError::Transport(Box::new(error)))?;
        Ok(())
    }
}
