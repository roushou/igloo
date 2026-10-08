//! Content-addressed filesystem snapshots.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::process::EnvVars;
use crate::{Digest, DigestError};

/// How a layer blob is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaType {
    /// A POSIX tar archive.
    #[serde(rename = "tar")]
    Tar,
    /// A gzip-compressed tar archive, as most OCI image layers are.
    #[serde(rename = "tar+gzip")]
    TarGzip,
}

/// One layer of a snapshot: a blob and how to read it. Deletions are OCI whiteout entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SnapshotLayer {
    digest: Digest,
    media_type: MediaType,
}

/// The ordered layers of a snapshot, lowest first, and the environment its processes start
/// with, such as an imported image's `PATH`.
///
/// Invariant: 1 to [`SnapshotManifest::MAX_LAYERS`] layers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotManifest {
    layers: Vec<SnapshotLayer>,
    env: EnvVars,
}

/// Why a manifest is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotManifestError {
    /// No layers.
    #[error("a snapshot needs at least one layer")]
    Empty,
    /// More than [`SnapshotManifest::MAX_LAYERS`] layers.
    #[error("a snapshot has at most {max} layers", max = SnapshotManifest::MAX_LAYERS)]
    TooManyLayers,
    /// Bytes that are not a canonical manifest.
    #[error("malformed snapshot manifest")]
    Malformed,
}

impl MediaType {
    /// The name used in manifests and the API.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tar => "tar",
            Self::TarGzip => "tar+gzip",
        }
    }
}

impl FromStr for MediaType {
    type Err = SnapshotManifestError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tar" => Ok(Self::Tar),
            "tar+gzip" => Ok(Self::TarGzip),
            _ => Err(SnapshotManifestError::Malformed),
        }
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl SnapshotLayer {
    /// A layer stored under `digest`, encoded as `media_type`.
    #[must_use]
    pub const fn new(digest: Digest, media_type: MediaType) -> Self {
        Self { digest, media_type }
    }

    /// The blob holding the layer.
    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// How the blob is encoded.
    #[must_use]
    pub const fn media_type(&self) -> MediaType {
        self.media_type
    }
}

impl SnapshotManifest {
    /// The maximum number of layers.
    pub const MAX_LAYERS: usize = 128;

    const V1: &'static str = "igloo.snapshot.v1";
    const V2: &'static str = "igloo.snapshot.v2";
    const V3: &'static str = "igloo.snapshot.v3";

    /// The layers, lowest first.
    #[must_use]
    pub fn layers(&self) -> &[SnapshotLayer] {
        &self.layers
    }

    /// The environment processes start with; a sandbox's own environment overrides it.
    #[must_use]
    pub const fn env(&self) -> &EnvVars {
        &self.env
    }

    /// This manifest with environment `env`.
    #[must_use]
    pub fn with_env(self, env: EnvVars) -> Self {
        Self { env, ..self }
    }

    /// This manifest with `layers` added on top, keeping its environment.
    pub fn extended(
        &self,
        layers: impl IntoIterator<Item = SnapshotLayer>,
    ) -> Result<Self, SnapshotManifestError> {
        let layers = self
            .layers
            .iter()
            .copied()
            .chain(layers)
            .collect::<Vec<_>>();
        Ok(Self::try_from(layers)?.with_env(self.env.clone()))
    }

    /// This manifest's environment over `layers` instead of its own.
    pub fn replaced(&self, layers: Vec<SnapshotLayer>) -> Result<Self, SnapshotManifestError> {
        Ok(Self::try_from(layers)?.with_env(self.env.clone()))
    }

    /// The canonical byte form whose blake3 hash is the [`SnapshotId`]: a version line, then
    /// `<media type> <digest>` per layer. A manifest with an environment is version 3, whose
    /// lines are `layer <media type> <digest>` and `env <KEY>=<escaped value>`, with `\` and
    /// newlines escaped as `\\` and `\n`.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let v3 = !self.env.is_empty();
        let mut text = format!("{}\n", if v3 { Self::V3 } else { Self::V2 });
        for layer in &self.layers {
            if v3 {
                text.push_str("layer ");
            }
            text.push_str(layer.media_type.as_str());
            text.push(' ');
            text.push_str(&layer.digest.to_string());
            text.push('\n');
        }
        for (key, value) in self.env.iter() {
            text.push_str("env ");
            text.push_str(key);
            text.push('=');
            text.push_str(&value.replace('\\', "\\\\").replace('\n', "\\n"));
            text.push('\n');
        }
        text.into_bytes()
    }

    fn parse_layer(line: &str, version: &str) -> Result<SnapshotLayer, SnapshotManifestError> {
        let (media_type, digest) = if version == Self::V1 {
            (MediaType::Tar, line)
        } else {
            let (media_type, digest) = line
                .split_once(' ')
                .ok_or(SnapshotManifestError::Malformed)?;
            (media_type.parse()?, digest)
        };
        let digest = digest
            .parse()
            .map_err(|_| SnapshotManifestError::Malformed)?;
        Ok(SnapshotLayer::new(digest, media_type))
    }

    fn unescape(value: &str) -> Result<String, SnapshotManifestError> {
        let mut out = String::with_capacity(value.len());
        let mut chars = value.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                _ => return Err(SnapshotManifestError::Malformed),
            }
        }
        Ok(out)
    }
}

impl TryFrom<&[u8]> for SnapshotManifest {
    type Error = SnapshotManifestError;

    /// Parses the canonical form, current or v1 (whose layers are all `tar`).
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        let text = std::str::from_utf8(bytes).map_err(|_| SnapshotManifestError::Malformed)?;
        let (version, body) = text
            .split_once('\n')
            .ok_or(SnapshotManifestError::Malformed)?;
        if version == Self::V3 {
            let mut layers = Vec::new();
            let mut env = Vec::new();
            for line in body.lines() {
                if let Some(layer) = line.strip_prefix("layer ") {
                    layers.push(Self::parse_layer(layer, version)?);
                } else if let Some(pair) = line.strip_prefix("env ") {
                    let (key, value) = pair
                        .split_once('=')
                        .ok_or(SnapshotManifestError::Malformed)?;
                    env.push((key.to_owned(), Self::unescape(value)?));
                } else {
                    return Err(SnapshotManifestError::Malformed);
                }
            }
            let env = EnvVars::from_pairs(env).map_err(|_| SnapshotManifestError::Malformed)?;
            return Ok(Self::try_from(layers)?.with_env(env));
        }
        if version != Self::V1 && version != Self::V2 {
            return Err(SnapshotManifestError::Malformed);
        }
        let layers = body
            .lines()
            .map(|line| Self::parse_layer(line, version))
            .collect::<Result<Vec<_>, _>>()?;
        Self::try_from(layers)
    }
}

impl TryFrom<Vec<SnapshotLayer>> for SnapshotManifest {
    type Error = SnapshotManifestError;

    fn try_from(layers: Vec<SnapshotLayer>) -> Result<Self, Self::Error> {
        if layers.is_empty() {
            Err(SnapshotManifestError::Empty)
        } else if layers.len() > Self::MAX_LAYERS {
            Err(SnapshotManifestError::TooManyLayers)
        } else {
            Ok(Self {
                layers,
                env: EnvVars::default(),
            })
        }
    }
}

impl From<SnapshotManifest> for Vec<SnapshotLayer> {
    fn from(manifest: SnapshotManifest) -> Self {
        manifest.layers
    }
}

/// Identifies a snapshot by the digest of its manifest's canonical bytes.
///
/// Invariant: equal manifests have equal IDs; a sealed snapshot never changes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotId(Digest);

impl SnapshotId {
    /// The underlying digest.
    #[must_use]
    pub const fn as_digest(&self) -> &Digest {
        &self.0
    }
}

impl From<Digest> for SnapshotId {
    fn from(digest: Digest) -> Self {
        Self(digest)
    }
}

impl fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for SnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for SnapshotId {
    type Err = DigestError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(byte: u8, media_type: MediaType) -> SnapshotLayer {
        SnapshotLayer::new(Digest::from_blake3([byte; 32]), media_type)
    }

    #[test]
    fn an_environment_round_trips_in_version_3_and_survives_extension() {
        let env =
            EnvVars::from_pairs([("PATH", "/usr/bin:/bin"), ("MOTD", "a\\b\nc")]).expect("env");
        let manifest = SnapshotManifest::try_from(vec![layer(1, MediaType::TarGzip)])
            .expect("manifest")
            .with_env(env.clone());
        let bytes = manifest.canonical_bytes();
        assert!(bytes.starts_with(b"igloo.snapshot.v3\n"));
        assert_eq!(
            SnapshotManifest::try_from(bytes.as_slice()).expect("parse"),
            manifest
        );
        let extended = manifest
            .extended([layer(2, MediaType::Tar)])
            .expect("extend");
        assert_eq!(extended.env(), &env);
        let replaced = manifest
            .replaced(vec![layer(3, MediaType::Tar)])
            .expect("replace");
        assert_eq!(replaced.env(), &env);
        assert_eq!(replaced.layers(), [layer(3, MediaType::Tar)]);
        let bare = SnapshotManifest::try_from(vec![layer(1, MediaType::Tar)]).expect("bare");
        assert!(
            bare.canonical_bytes().starts_with(b"igloo.snapshot.v2\n"),
            "manifests without an environment keep their version 2 id"
        );
    }

    #[test]
    fn canonical_bytes_list_layers_in_order_and_parse_back() {
        let low = layer(1, MediaType::TarGzip);
        let high = layer(2, MediaType::Tar);
        let manifest = SnapshotManifest::try_from(vec![low, high]).expect("valid");
        let text = String::from_utf8(manifest.canonical_bytes()).expect("utf-8");
        assert_eq!(
            text,
            format!(
                "igloo.snapshot.v2\ntar+gzip {}\ntar {}\n",
                low.digest(),
                high.digest()
            )
        );
        assert_eq!(
            SnapshotManifest::try_from(manifest.canonical_bytes().as_slice()),
            Ok(manifest)
        );
    }

    #[test]
    fn v1_manifests_stay_readable() {
        let digest = Digest::from_blake3([3; 32]);
        let v1 = format!("igloo.snapshot.v1\n{digest}\n");
        let manifest = SnapshotManifest::try_from(v1.as_bytes()).expect("v1");
        assert_eq!(
            manifest.layers(),
            [SnapshotLayer::new(digest, MediaType::Tar)]
        );
    }

    #[test]
    fn malformed_and_empty_manifests_are_rejected() {
        assert_eq!(
            SnapshotManifest::try_from(Vec::new()),
            Err(SnapshotManifestError::Empty)
        );
        assert_eq!(
            SnapshotManifest::try_from(b"not a manifest".as_slice()),
            Err(SnapshotManifestError::Malformed)
        );
        let unknown = format!("igloo.snapshot.v2\nzip {}\n", Digest::from_blake3([4; 32]));
        assert_eq!(
            SnapshotManifest::try_from(unknown.as_bytes()),
            Err(SnapshotManifestError::Malformed)
        );
    }

    #[test]
    fn extending_adds_layers_on_top() {
        let base = SnapshotManifest::try_from(vec![layer(1, MediaType::TarGzip)]).expect("base");
        let run = base.extended([layer(2, MediaType::Tar)]).expect("run");
        assert_eq!(run.layers().len(), 2);
        assert_eq!(run.layers()[1], layer(2, MediaType::Tar));
    }
}
