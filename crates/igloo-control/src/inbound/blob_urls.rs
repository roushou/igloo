//! Presigned blob URLs: `{base}/v1/blobs/{digest}` and `{base}/v1/seals/{id}/layer`, with
//! `?expires=<unix seconds>&signature=<hex>`.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use igloo_core::Digest;
use igloo_core::seal::SealId;
use jiff::SignedDuration;
use reqwest::Url;

use crate::ports::Clock;

/// Mints and verifies expiring URLs for blob transfers that need no other credentials.
///
/// Invariant: a URL verifies only for the access and subject (a blob digest or a seal's layer)
/// it was signed for, and only until
/// its expiry. Expiries are rounded up to the end of the next whole window, so URLs minted
/// within one window are identical and stay valid for at least one window.
#[derive(Clone)]
pub struct BlobUrls {
    base: Url,
    key: BlobSigningKey,
    clock: Arc<dyn Clock>,
    window: SignedDuration,
}

/// The secret blob URLs are signed with.
///
/// Invariant: derived from a secret of at least [`BlobSigningKey::MIN_SECRET_BYTES`] bytes.
#[derive(Clone)]
pub struct BlobSigningKey([u8; 32]);

/// What a blob URL allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlobAccess {
    /// `GET`: downloading the blob.
    Download,
    /// `PUT`: uploading the blob.
    Upload,
}

/// The query parameters of a presigned URL.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct BlobSignature {
    /// When the URL expires, in seconds since the Unix epoch.
    pub expires: i64,
    /// The keyed blake3 hash, in hex.
    pub signature: String,
}

/// Why a presigned URL was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BlobUrlRejected {
    /// The URL is past its expiry.
    #[error("the URL has expired")]
    Expired,
    /// The signature does not match the access, digest and expiry.
    #[error("the URL signature is invalid")]
    Invalid,
}

/// Why a signing secret was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("must be at least {} bytes", BlobSigningKey::MIN_SECRET_BYTES)]
pub struct WeakSecret;

impl BlobUrls {
    /// How long minted URLs are valid at least, unless set otherwise.
    pub const DEFAULT_WINDOW: SignedDuration = SignedDuration::from_mins(15);

    /// URLs under `base` (the REST API's public URL), signed with `key`, timed by `clock`.
    #[must_use]
    pub fn new(base: Url, key: BlobSigningKey, clock: Arc<dyn Clock>) -> Self {
        Self {
            base,
            key,
            clock,
            window: Self::DEFAULT_WINDOW,
        }
    }

    /// Sets the validity window; it must be at least one second.
    #[must_use]
    pub fn with_window(mut self, window: SignedDuration) -> Self {
        self.window = window.max(SignedDuration::from_secs(1));
        self
    }

    /// A URL that downloads blob `digest`.
    #[must_use]
    pub fn download(&self, digest: Digest) -> Url {
        let digest = digest.to_string();
        self.sign(BlobAccess::Download, &digest, &["v1", "blobs", &digest])
    }

    /// A URL that uploads blob `digest`.
    #[must_use]
    pub fn upload(&self, digest: Digest) -> Url {
        let digest = digest.to_string();
        self.sign(BlobAccess::Upload, &digest, &["v1", "blobs", &digest])
    }

    /// A URL that uploads the layer completing seal `seal`.
    #[must_use]
    pub fn seal_upload(&self, seal: SealId) -> Url {
        let seal = seal.to_string();
        self.sign(BlobAccess::Upload, &seal, &["v1", "seals", &seal, "layer"])
    }

    /// Accepts `signature` if it was minted for `access` to `digest` and has not expired.
    pub fn verify(
        &self,
        access: BlobAccess,
        digest: Digest,
        signature: &BlobSignature,
    ) -> Result<(), BlobUrlRejected> {
        self.check(access, &digest.to_string(), signature)
    }

    /// Accepts `signature` if it was minted for uploading the layer of `seal` and has not
    /// expired.
    pub fn verify_seal_upload(
        &self,
        seal: SealId,
        signature: &BlobSignature,
    ) -> Result<(), BlobUrlRejected> {
        self.check(BlobAccess::Upload, &seal.to_string(), signature)
    }

    /// The base URL, key and clock URLs are minted with, for credentials of the same server.
    pub(super) fn signing(&self) -> (Url, BlobSigningKey, Arc<dyn Clock>) {
        (self.base.clone(), self.key.clone(), Arc::clone(&self.clock))
    }

    fn check(
        &self,
        access: BlobAccess,
        subject: &str,
        signature: &BlobSignature,
    ) -> Result<(), BlobUrlRejected> {
        let presented =
            blake3::Hash::from_hex(&signature.signature).map_err(|_| BlobUrlRejected::Invalid)?;
        // `blake3::Hash` compares in constant time.
        if presented != self.key.mac(access, subject, signature.expires) {
            return Err(BlobUrlRejected::Invalid);
        }
        if self.now() >= signature.expires {
            return Err(BlobUrlRejected::Expired);
        }
        Ok(())
    }

    fn sign(&self, access: BlobAccess, subject: &str, path: &[&str]) -> Url {
        let window = self.window.as_secs();
        let expires = (self.now().div_euclid(window) + 2) * window;
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map(|mut segments| {
                segments.pop_if_empty().extend(path);
            })
            .ok();
        url.query_pairs_mut()
            .append_pair("expires", &expires.to_string())
            .append_pair(
                "signature",
                &self.key.mac(access, subject, expires).to_hex(),
            );
        url
    }

    fn now(&self) -> i64 {
        jiff::Timestamp::from(self.clock.now()).as_second()
    }
}

impl BlobSigningKey {
    /// The shortest secret accepted.
    pub const MIN_SECRET_BYTES: usize = 32;
    const CONTEXT: &'static str = "igloo blob urls v1";
    const CREDENTIAL_CONTEXT: &'static str = "igloo workspace credentials v1";

    /// A keyed hash of `subject` under a key derived for workspace credentials, so no blob URL
    /// signature is a credential.
    pub(super) fn credential_mac(&self, subject: &str) -> blake3::Hash {
        let key = blake3::derive_key(Self::CREDENTIAL_CONTEXT, &self.0);
        blake3::keyed_hash(&key, subject.as_bytes())
    }

    fn mac(&self, access: BlobAccess, subject: &str, expires: i64) -> blake3::Hash {
        blake3::keyed_hash(
            &self.0,
            format!("{access}\n{subject}\n{expires}").as_bytes(),
        )
    }
}

impl FromStr for BlobSigningKey {
    type Err = WeakSecret;

    /// Derives a key from a secret of at least [`Self::MIN_SECRET_BYTES`] bytes.
    fn from_str(secret: &str) -> Result<Self, Self::Err> {
        if secret.len() < Self::MIN_SECRET_BYTES {
            return Err(WeakSecret);
        }
        Ok(Self(blake3::derive_key(Self::CONTEXT, secret.as_bytes())))
    }
}

impl fmt::Debug for BlobSigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BlobSigningKey(..)")
    }
}

impl fmt::Display for BlobAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Download => "GET",
            Self::Upload => "PUT",
        })
    }
}

#[cfg(test)]
mod tests {
    use igloo_core::Timestamp;

    use super::*;
    use crate::adapters::memory::FixedClock;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn at(seconds: i64) -> Timestamp {
        Timestamp::new(jiff::Timestamp::from_second(seconds).expect("timestamp"))
    }

    fn urls(clock: &Arc<FixedClock>) -> BlobUrls {
        BlobUrls::new(
            "http://igloo.test/api/".parse().expect("base"),
            SECRET.parse().expect("key"),
            Arc::clone(clock) as Arc<dyn Clock>,
        )
    }

    fn signature(url: &Url) -> BlobSignature {
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        BlobSignature {
            expires: query["expires"].parse().expect("expires"),
            signature: query["signature"].clone(),
        }
    }

    #[test]
    fn urls_verify_for_their_access_and_digest_until_they_expire() {
        let clock = Arc::new(FixedClock::new(at(1_000)));
        let urls = urls(&clock);
        let digest = Digest::from_blake3([1; 32]);
        let url = urls.download(digest);
        assert_eq!(url.path(), format!("/api/v1/blobs/{digest}"), "{url}");
        let signed = signature(&url);
        assert_eq!(urls.verify(BlobAccess::Download, digest, &signed), Ok(()));
        assert_eq!(
            urls.verify(BlobAccess::Upload, digest, &signed),
            Err(BlobUrlRejected::Invalid)
        );
        let other = Digest::from_blake3([2; 32]);
        assert_eq!(
            urls.verify(BlobAccess::Download, other, &signed),
            Err(BlobUrlRejected::Invalid)
        );
        let altered = BlobSignature {
            expires: signed.expires + 60,
            ..signed.clone()
        };
        assert_eq!(
            urls.verify(BlobAccess::Download, digest, &altered),
            Err(BlobUrlRejected::Invalid)
        );

        clock.set(at(signed.expires));
        assert_eq!(
            urls.verify(BlobAccess::Download, digest, &signed),
            Err(BlobUrlRejected::Expired)
        );
    }

    #[test]
    fn urls_are_stable_within_a_window_and_valid_for_at_least_one() {
        let clock = Arc::new(FixedClock::new(at(900)));
        let urls = urls(&clock);
        let digest = Digest::from_blake3([1; 32]);
        let first = urls.download(digest);
        clock.set(at(1_799));
        assert_eq!(urls.download(digest), first);
        assert!(signature(&first).expires >= 1_799 + 900);
    }

    #[test]
    fn seal_upload_urls_open_only_their_seal() {
        let clock = Arc::new(FixedClock::new(at(1_000)));
        let urls = urls(&clock);
        let seal: SealId = igloo_core::Id::from_uuid(uuid::Uuid::from_u128(3));
        let url = urls.seal_upload(seal);
        assert_eq!(url.path(), format!("/api/v1/seals/{seal}/layer"));
        let signed = signature(&url);
        assert_eq!(urls.verify_seal_upload(seal, &signed), Ok(()));
        let other: SealId = igloo_core::Id::from_uuid(uuid::Uuid::from_u128(4));
        assert_eq!(
            urls.verify_seal_upload(other, &signed),
            Err(BlobUrlRejected::Invalid)
        );
    }

    #[test]
    fn short_secrets_are_refused() {
        assert_eq!(
            "short".parse::<BlobSigningKey>().map(|_| ()),
            Err(WeakSecret)
        );
    }
}
