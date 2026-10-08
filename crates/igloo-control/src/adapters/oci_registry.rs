use std::fmt::Write as _;
use std::path::PathBuf;

use async_trait::async_trait;
use igloo_core::Digest;
use igloo_core::snapshot::MediaType;
use reqwest::header::{ACCEPT, AUTHORIZATION, WWW_AUTHENTICATE};
use reqwest::{Response, StatusCode};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt;

use crate::ports::{
    ImageReference, ImageRegistry, Platform, PulledImage, PulledLayer, RegistryError,
};

/// Pulls public images from OCI distribution registries (Docker Hub, GHCR, ...), following
/// the anonymous bearer-token challenge.
pub struct OciRegistry {
    http: reqwest::Client,
    downloads: PathBuf,
    scheme: &'static str,
}

/// An image index (multi-platform) or an image manifest.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    config: Option<Descriptor>,
    #[serde(default)]
    manifests: Vec<Descriptor>,
    #[serde(default)]
    layers: Vec<Descriptor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Descriptor {
    #[serde(default)]
    media_type: String,
    digest: String,
    #[serde(default)]
    platform: Option<DescriptorPlatform>,
}

#[derive(Deserialize)]
struct DescriptorPlatform {
    os: String,
    architecture: String,
}

/// A token response: registries send `token`, `access_token` or both.
/// An image configuration: only its environment is used.
#[derive(Deserialize)]
struct ImageConfig {
    #[serde(default)]
    config: Option<ContainerConfig>,
}

#[derive(Deserialize)]
struct ContainerConfig {
    #[serde(rename = "Env", default)]
    env: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Token {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
}

/// One registry conversation: the image and the bearer token once obtained.
struct Pull<'a> {
    registry: &'a OciRegistry,
    image: &'a ImageReference,
    token: Option<String>,
}

impl OciRegistry {
    const MANIFEST_TYPES: &'static str = "application/vnd.oci.image.index.v1+json, \
        application/vnd.docker.distribution.manifest.list.v2+json, \
        application/vnd.oci.image.manifest.v1+json, \
        application/vnd.docker.distribution.manifest.v2+json";

    /// A client over HTTPS, downloading layers into `downloads`. Installs `ring` as the
    /// process's TLS crypto provider unless one is installed already.
    pub fn new(downloads: PathBuf) -> Result<Self, RegistryError> {
        // Fails only when a provider is already installed, which serves equally well.
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
        Ok(Self {
            http: reqwest::Client::builder().build().map_err(transport)?,
            downloads,
            scheme: "https",
        })
    }

    /// A client over plain HTTP, for registries on a trusted local network.
    pub fn insecure(downloads: PathBuf) -> Result<Self, RegistryError> {
        Ok(Self {
            scheme: "http",
            ..Self::new(downloads)?
        })
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().fold(String::new(), |mut hex, byte| {
            // Writing to a `String` cannot fail.
            let _ = write!(hex, "{byte:02x}");
            hex
        })
    }

    fn media_type(layer: &str) -> Result<MediaType, RegistryError> {
        match layer {
            "application/vnd.oci.image.layer.v1.tar+gzip"
            | "application/vnd.docker.image.rootfs.diff.tar.gzip" => Ok(MediaType::TarGzip),
            "application/vnd.oci.image.layer.v1.tar" => Ok(MediaType::Tar),
            other => Err(RegistryError::UnsupportedMediaType(other.to_owned())),
        }
    }
}

#[async_trait]
impl ImageRegistry for OciRegistry {
    async fn pull(
        &self,
        image: &ImageReference,
        platform: &Platform,
    ) -> Result<PulledImage, RegistryError> {
        tokio::fs::create_dir_all(&self.downloads)
            .await
            .map_err(transport)?;
        let mut pull = Pull {
            registry: self,
            image,
            token: None,
        };
        let mut document = pull.manifest(image.reference()).await?;
        if !document.manifests.is_empty() {
            let chosen = document
                .manifests
                .iter()
                .find(|entry| {
                    entry.platform.as_ref().is_some_and(|candidate| {
                        candidate.os == platform.os()
                            && candidate.architecture == platform.architecture()
                    })
                })
                .ok_or_else(|| RegistryError::NoMatchingPlatform(platform.to_string()))?
                .digest
                .clone();
            document = pull.manifest(&chosen).await?;
        }
        if document.layers.is_empty() {
            return Err(RegistryError::Transport(format!(
                "manifest of type {:?} lists no layers",
                document.media_type
            )));
        }
        let mut layers = Vec::new();
        for layer in &document.layers {
            let media_type = Self::media_type(&layer.media_type)?;
            layers.push(pull.layer(&layer.digest, media_type).await?);
        }
        let env = match &document.config {
            Some(config) => pull.env(&config.digest).await?,
            None => Vec::new(),
        };
        Ok(PulledImage { layers, env })
    }
}

impl Pull<'_> {
    fn url(&self, path: &str) -> String {
        format!(
            "{}://{}/v2/{}/{path}",
            self.registry.scheme,
            self.image.api_host(),
            self.image.repository()
        )
    }

    async fn manifest(&mut self, reference: &str) -> Result<Document, RegistryError> {
        let url = self.url(&format!("manifests/{reference}"));
        let response = self.get(&url, Some(OciRegistry::MANIFEST_TYPES)).await?;
        let bytes = response.bytes().await.map_err(transport)?;
        serde_json::from_slice(&bytes).map_err(transport)
    }

    /// The `Env` of the image configuration blob `digest`, verified against its sha256.
    async fn env(&mut self, digest: &str) -> Result<Vec<(String, String)>, RegistryError> {
        let response = self
            .get(&self.url(&format!("blobs/{digest}")), None)
            .await?;
        let bytes = response.bytes().await.map_err(transport)?;
        let actual = format!("sha256:{}", OciRegistry::hex(&Sha256::digest(&bytes)));
        if actual != digest {
            return Err(RegistryError::DigestMismatch(digest.to_owned()));
        }
        let config: ImageConfig = serde_json::from_slice(&bytes).map_err(transport)?;
        Ok(config
            .config
            .and_then(|config| config.env)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|pair| {
                pair.split_once('=')
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
            })
            .collect())
    }

    /// Streams one layer to a file, verifying its sha256 and computing its blake3.
    async fn layer(
        &mut self,
        digest: &str,
        media_type: MediaType,
    ) -> Result<PulledLayer, RegistryError> {
        let expected = digest
            .strip_prefix("sha256:")
            .ok_or_else(|| RegistryError::UnsupportedMediaType(digest.to_owned()))?
            .to_owned();
        let mut response = self
            .get(&self.url(&format!("blobs/{digest}")), None)
            .await?;
        let file = tempfile::NamedTempFile::new_in(&self.registry.downloads).map_err(transport)?;
        let (file, path) = file.into_parts();
        let mut writer = tokio::fs::File::from_std(file);
        let mut sha256 = Sha256::new();
        let mut blake3 = blake3::Hasher::new();
        while let Some(chunk) = response.chunk().await.map_err(transport)? {
            sha256.update(&chunk);
            blake3.update(&chunk);
            writer.write_all(&chunk).await.map_err(transport)?;
        }
        writer.flush().await.map_err(transport)?;
        let actual = OciRegistry::hex(&sha256.finalize());
        if actual != expected {
            return Err(RegistryError::DigestMismatch(digest.to_owned()));
        }
        Ok(PulledLayer {
            media_type,
            digest: Digest::from_blake3(*blake3.finalize().as_bytes()),
            file: path,
        })
    }

    /// GETs `url`, answering one bearer-token challenge if the registry asks.
    async fn get(&mut self, url: &str, accept: Option<&str>) -> Result<Response, RegistryError> {
        let response = self.send(url, accept).await?;
        if response.status() != StatusCode::UNAUTHORIZED || self.token.is_some() {
            return Self::checked(response);
        }
        let challenge = response
            .headers()
            .get(WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| RegistryError::Transport("401 without a challenge".to_owned()))?
            .to_owned();
        self.token = Some(self.authenticate(&challenge).await?);
        Self::checked(self.send(url, accept).await?)
    }

    async fn send(&self, url: &str, accept: Option<&str>) -> Result<Response, RegistryError> {
        let mut request = self.registry.http.get(url);
        if let Some(accept) = accept {
            request = request.header(ACCEPT, accept);
        }
        if let Some(token) = &self.token {
            request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        request.send().await.map_err(transport)
    }

    /// Fetches an anonymous token from the challenge's realm.
    async fn authenticate(&self, challenge: &str) -> Result<String, RegistryError> {
        let parameters = challenge.strip_prefix("Bearer ").ok_or_else(|| {
            RegistryError::Transport(format!("unsupported challenge {challenge}"))
        })?;
        let mut realm = None;
        let mut query = Vec::new();
        for parameter in parameters.split(',') {
            let Some((key, value)) = parameter.trim().split_once('=') else {
                continue;
            };
            let value = value.trim_matches('"').to_owned();
            if key == "realm" {
                realm = Some(value);
            } else {
                query.push((key.to_owned(), value));
            }
        }
        let realm =
            realm.ok_or_else(|| RegistryError::Transport("challenge without realm".to_owned()))?;
        let mut url = reqwest::Url::parse(&realm).map_err(transport)?;
        url.query_pairs_mut().extend_pairs(query);
        let response = self
            .registry
            .http
            .get(url)
            .send()
            .await
            .map_err(transport)?;
        let bytes = Self::checked(response)?.bytes().await.map_err(transport)?;
        let token: Token = serde_json::from_slice(&bytes).map_err(transport)?;
        token
            .token
            .or(token.access_token)
            .ok_or_else(|| RegistryError::Transport("token response without a token".to_owned()))
    }

    fn checked(response: Response) -> Result<Response, RegistryError> {
        match response.status() {
            status if status.is_success() => Ok(response),
            StatusCode::NOT_FOUND => Err(RegistryError::NotFound),
            status => Err(RegistryError::Transport(format!(
                "registry answered {status}"
            ))),
        }
    }
}

/// A transport failure, keeping its message.
fn transport(error: impl std::fmt::Display) -> RegistryError {
    RegistryError::Transport(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::future::Ready;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use axum::response::{IntoResponse, Response};
    use axum::routing::get;
    use igloo_core::snapshot::MediaType;
    use serde_json::json;
    use sha2::{Digest as _, Sha256};

    use super::OciRegistry;
    use crate::app::{AppError, TaskSupervisor};
    use crate::ports::ImageRegistry as _;
    use crate::ports::conformance::ImageRegistryConformance;

    /// An OCI registry serving the conformance fixture as `igloo/fixture:1`, behind a bearer
    /// token.
    #[derive(Clone)]
    struct FakeRegistry {
        address: String,
        blobs: Arc<Vec<(String, Vec<u8>)>>,
        index: Arc<serde_json::Value>,
        manifest: Arc<(String, serde_json::Value)>,
    }

    impl FakeRegistry {
        const TOKEN: &'static str = "fixture-token";

        fn new(address: String) -> Self {
            let sha256 =
                |bytes: &[u8]| format!("sha256:{}", OciRegistry::hex(&Sha256::digest(bytes)));
            let layers = ImageRegistryConformance::layers();
            let descriptors: Vec<_> = layers
                .iter()
                .map(|(media_type, bytes)| {
                    let media_type = match media_type {
                        MediaType::TarGzip => "application/vnd.docker.image.rootfs.diff.tar.gzip",
                        MediaType::Tar => "application/vnd.oci.image.layer.v1.tar",
                    };
                    json!({ "mediaType": media_type, "digest": sha256(bytes), "size": bytes.len() })
                })
                .collect();
            let env: Vec<String> = ImageRegistryConformance::env()
                .into_iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect();
            let config = json!({ "config": { "Env": env } }).to_string().into_bytes();
            let config_digest = sha256(&config);
            let manifest = json!({
                "schemaVersion": 2,
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
                "config": {
                    "mediaType": "application/vnd.oci.image.config.v1+json",
                    "digest": config_digest,
                },
                "layers": descriptors,
            });
            let manifest_digest = sha256(manifest.to_string().as_bytes());
            let platform = ImageRegistryConformance::platform();
            let index = json!({
                "schemaVersion": 2,
                "mediaType": "application/vnd.oci.image.index.v1+json",
                "manifests": [
                    { "digest": "sha256:0000", "platform": { "os": "linux", "architecture": "riscv64" } },
                    {
                        "digest": manifest_digest,
                        "platform": { "os": platform.os(), "architecture": platform.architecture() },
                    },
                ],
            });
            Self {
                address,
                blobs: Arc::new(
                    layers
                        .into_iter()
                        .map(|(_, bytes)| (sha256(&bytes), bytes))
                        .chain([(config_digest, config)])
                        .collect(),
                ),
                index: Arc::new(index),
                manifest: Arc::new((manifest_digest, manifest)),
            }
        }

        fn router(self) -> Router {
            Router::new()
                .route("/token", get(Self::token))
                .route("/v2/{*path}", get(Self::serve))
                .with_state(self)
        }

        fn token() -> Ready<impl IntoResponse> {
            std::future::ready(axum::Json(
                json!({ "token": Self::TOKEN, "access_token": Self::TOKEN }),
            ))
        }

        fn serve(
            State(registry): State<Self>,
            Path(path): Path<String>,
            mut headers: HeaderMap,
        ) -> Ready<Response> {
            let authorization = headers.remove(header::AUTHORIZATION);
            std::future::ready(registry.respond(&path, authorization.as_ref()))
        }

        fn respond(&self, path: &str, authorization: Option<&HeaderValue>) -> Response {
            let registry = self;
            let authorized = authorization
                .is_some_and(|value| value == format!("Bearer {}", Self::TOKEN).as_str());
            if !authorized {
                let challenge = format!(
                    "Bearer realm=\"http://{}/token\",service=\"fake\",scope=\"repository:igloo/fixture:pull\"",
                    registry.address
                );
                return (
                    StatusCode::UNAUTHORIZED,
                    [(header::WWW_AUTHENTICATE, challenge)],
                )
                    .into_response();
            }
            let Some(path) = path.strip_prefix("igloo/fixture/") else {
                return StatusCode::NOT_FOUND.into_response();
            };
            match path.split_once('/') {
                Some(("manifests", "1")) => {
                    axum::Json(registry.index.as_ref().clone()).into_response()
                }
                Some(("manifests", digest)) if digest == registry.manifest.0 => {
                    axum::Json(registry.manifest.1.clone()).into_response()
                }
                Some(("blobs", digest)) => registry
                    .blobs
                    .iter()
                    .find(|(known, _)| known == digest)
                    .map_or_else(
                        || StatusCode::NOT_FOUND.into_response(),
                        |(_, bytes)| bytes.clone().into_response(),
                    ),
                _ => StatusCode::NOT_FOUND.into_response(),
            }
        }
    }

    #[tokio::test]
    async fn pulls_through_the_token_flow_and_passes_conformance() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address").to_string();
        let router = FakeRegistry::new(address.clone()).router();
        let supervisor = TaskSupervisor::new();
        supervisor.spawn("fake-registry", move |cancel| async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(cancel.cancelled_owned())
                .await
                .map_err(AppError::infrastructure)
        });
        let downloads = tempfile::tempdir().expect("downloads");
        ImageRegistryConformance {
            registry: Arc::new(OciRegistry::insecure(downloads.path().to_owned()).expect("client")),
            image: format!("{address}/igloo/fixture:1")
                .parse()
                .expect("reference"),
        }
        .run_all()
        .await;
        supervisor
            .shutdown(Duration::from_secs(1))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    #[ignore = "pulls from Docker Hub"]
    async fn pulls_a_public_image_from_docker_hub() {
        let downloads = tempfile::tempdir().expect("downloads");
        let registry = OciRegistry::new(downloads.path().to_owned()).expect("client");
        let layers = registry
            .pull(
                &"busybox:1.37".parse().expect("reference"),
                &"linux/amd64".parse().expect("platform"),
            )
            .await
            .expect("pull");
        assert!(!layers.layers.is_empty());
    }
}
