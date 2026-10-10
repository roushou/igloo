//! The credential a workspace's sandbox uses to reach Igloo's git endpoint: narrower than the
//! API token, which never enters a sandbox.

use std::collections::BTreeMap;
use std::sync::Arc;

use igloo_core::sandbox::SandboxId;
use igloo_core::workspace::WorkspaceId;
use jiff::SignedDuration;
use reqwest::Url;

use super::blob_urls::{BlobSigningKey, BlobUrls};
use crate::ports::Clock;

/// Mints and verifies git credentials for workspaces' sandboxes.
///
/// Invariants: a credential names one workspace and one of its sandboxes and verifies only
/// until its expiry; verifying it says nothing beyond that: the git endpoint additionally
/// requires the workspace to exist, to still run that sandbox and to belong to the repository
/// requested, and acts as the workspace's owner. A credential is a bearer token for the git
/// endpoint only; the REST API does not accept it.
#[derive(Clone)]
pub struct WorkspaceCredentials {
    base: Url,
    key: BlobSigningKey,
    clock: Arc<dyn Clock>,
}

/// What a verified credential names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkspaceGrant {
    /// The workspace the credential was minted for.
    pub workspace: WorkspaceId,
    /// The sandbox it was minted for.
    pub sandbox: SandboxId,
}

impl WorkspaceCredentials {
    /// How long a minted credential is valid. A terminal mints a fresh one when it opens, so a
    /// long-lived session outlives it only after this long.
    pub const LIFETIME: SignedDuration = SignedDuration::from_hours(7 * 24);
    const PREFIX: &'static str = "iglws";

    /// Credentials for the git endpoint under `urls`' base URL, signed with `urls`' key.
    #[must_use]
    pub fn new(urls: &BlobUrls) -> Self {
        let (base, key, clock) = urls.signing();
        Self { base, key, clock }
    }

    /// A credential for `sandbox`, the sandbox of `workspace`.
    #[must_use]
    pub fn mint(&self, workspace: WorkspaceId, sandbox: SandboxId) -> String {
        let expires = self.now().saturating_add(Self::LIFETIME.as_secs());
        let subject = format!("{workspace}.{sandbox}.{expires}");
        let signature = self.key.credential_mac(&subject).to_hex();
        format!("{}.{subject}.{signature}", Self::PREFIX)
    }

    /// What `token` names, if it is a credential minted here that has not expired.
    #[must_use]
    pub fn verify(&self, token: &str) -> Option<WorkspaceGrant> {
        let rest = token.strip_prefix(Self::PREFIX)?.strip_prefix('.')?;
        let (subject, signature) = rest.rsplit_once('.')?;
        let presented = blake3::Hash::from_hex(signature).ok()?;
        // `blake3::Hash` compares in constant time.
        if presented != self.key.credential_mac(subject) {
            return None;
        }
        let mut parts = subject.split('.');
        let workspace = parts.next()?.parse().ok()?;
        let sandbox = parts.next()?.parse().ok()?;
        let expires: i64 = parts.next()?.parse().ok()?;
        (parts.next().is_none() && self.now() < expires)
            .then_some(WorkspaceGrant { workspace, sandbox })
    }

    /// The environment that makes git send a fresh credential for `sandbox` to Igloo's git
    /// endpoint, and to no other host: one `http.<endpoint>.extraHeader` setting passed through
    /// `GIT_CONFIG_*`.
    #[must_use]
    pub fn git_env(&self, workspace: WorkspaceId, sandbox: SandboxId) -> BTreeMap<String, String> {
        let endpoint = format!("{}/git/", self.base.as_str().trim_end_matches('/'));
        let token = self.mint(workspace, sandbox);
        BTreeMap::from([
            ("GIT_CONFIG_COUNT".to_owned(), "1".to_owned()),
            (
                "GIT_CONFIG_KEY_0".to_owned(),
                format!("http.{endpoint}.extraHeader"),
            ),
            (
                "GIT_CONFIG_VALUE_0".to_owned(),
                format!("Authorization: Bearer {token}"),
            ),
        ])
    }

    fn now(&self) -> i64 {
        jiff::Timestamp::from(self.clock.now()).as_second()
    }
}

#[cfg(test)]
mod tests {
    use igloo_core::Id;
    use uuid::Uuid;

    use super::*;
    use crate::testing::{BLOB_SECRET, blob_urls};

    struct Fixed(std::sync::atomic::AtomicI64);

    impl Clock for Fixed {
        fn now(&self) -> igloo_core::Timestamp {
            igloo_core::Timestamp::from(
                jiff::Timestamp::from_second(self.0.load(std::sync::atomic::Ordering::SeqCst))
                    .expect("in range"),
            )
        }
    }

    fn ids() -> (WorkspaceId, SandboxId) {
        (
            Id::from_uuid(Uuid::from_u128(1)),
            Id::from_uuid(Uuid::from_u128(2)),
        )
    }

    fn credentials(clock: Arc<Fixed>) -> WorkspaceCredentials {
        WorkspaceCredentials::new(&blob_urls(clock))
    }

    #[test]
    fn a_credential_verifies_as_what_it_was_minted_for_until_it_expires() {
        let clock = Arc::new(Fixed(1_800_000_000.into()));
        let credentials = credentials(Arc::clone(&clock));
        let (workspace, sandbox) = ids();
        let token = credentials.mint(workspace, sandbox);
        assert_eq!(
            credentials.verify(&token),
            Some(WorkspaceGrant { workspace, sandbox })
        );

        let lifetime = WorkspaceCredentials::LIFETIME.as_secs();
        clock.0.store(
            1_800_000_000 + lifetime - 1,
            std::sync::atomic::Ordering::SeqCst,
        );
        assert!(credentials.verify(&token).is_some());
        clock.0.store(
            1_800_000_000 + lifetime,
            std::sync::atomic::Ordering::SeqCst,
        );
        assert_eq!(credentials.verify(&token), None, "expired");
    }

    #[test]
    fn forged_altered_and_foreign_credentials_do_not_verify() {
        let clock = Arc::new(Fixed(1_800_000_000.into()));
        let credentials = credentials(Arc::clone(&clock));
        let (workspace, sandbox) = ids();
        let token = credentials.mint(workspace, sandbox);

        let other: SandboxId = Id::from_uuid(Uuid::from_u128(3));
        let swapped = token.replace(&sandbox.to_string(), &other.to_string());
        let extended = token.replace(".1800", ".9800");
        for bad in [
            "",
            "iglws",
            "iglws.a.b.c",
            "Bearer x",
            swapped.as_str(),
            extended.as_str(),
            &token[..token.len() - 1],
        ] {
            assert_eq!(credentials.verify(bad), None, "{bad}");
        }

        let key = format!("{BLOB_SECRET}-another");
        let other_server = WorkspaceCredentials {
            key: key.parse().expect("key"),
            ..credentials.clone()
        };
        assert_eq!(other_server.verify(&token), None, "signed with another key");
    }

    #[test]
    fn git_receives_the_credential_for_igloos_endpoint_only() {
        let clock = Arc::new(Fixed(1_800_000_000.into()));
        let credentials = credentials(clock);
        let (workspace, sandbox) = ids();
        let env = credentials.git_env(workspace, sandbox);

        let ask = |url: &str| {
            let mut command = std::process::Command::new("git");
            command
                .args(["config", "--get-urlmatch", "http.extraHeader", url])
                .env_clear()
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .envs(&env);
            let output = command.output().expect("git");
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        };
        let header = ask("http://igloo.test/git/repo_1.git/info/refs");
        let token = credentials.mint(workspace, sandbox);
        assert_eq!(header, format!("Authorization: Bearer {token}"));
        assert_eq!(ask("https://github.com/roushou/igloo.git"), "");
        assert_eq!(ask("http://igloo.test/v1/repos"), "", "not the API");
        assert_eq!(ask("http://igloo.test.evil.example/git/x.git"), "");
    }
}
