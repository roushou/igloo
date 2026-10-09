//! Behaviour every adapter of a port must show. Memory and real adapters run the same suites.

#![allow(
    clippy::panic,
    reason = "conformance suites report failures by panicking"
)]

use std::sync::Arc;

use igloo_core::sandbox::{Sandbox, SandboxSpec};
use igloo_core::snapshot::{MediaType, SnapshotId};
use igloo_core::{Actor, Digest, Id, Timestamp, Version};
use uuid::Uuid;

use tokio::io::AsyncReadExt;

use igloo_core::repo::{BranchName, CommitId, RepoId, SecretName};

use super::{
    BlobError, BlobStore, Checkpoints, Claim, CommitMeta, EntityStore, EventLog, Expected,
    FileStatus, Forge, ForgeError, IdempotencyStore, ImageReference, ImageRegistry, KeyedRequest,
    LogStore, Platform, RegistryError, Remote, SecretStore, SecretValue, Sequence, StorageError,
    StoredResponse, Versioned,
};

const NOW: Timestamp = Timestamp::new(jiff::Timestamp::constant(1_767_225_600, 0));

/// The `EntityStore` contract, exercised with sandboxes, plus the events it must log.
pub(crate) struct EntityStoreConformance {
    pub(crate) store: Arc<dyn EntityStore<Sandbox>>,
    pub(crate) events: Arc<dyn EventLog>,
}

impl EntityStoreConformance {
    pub(crate) async fn run_all(&self) {
        self.missing_entities_load_as_none().await;
        self.commits_advance_the_version().await;
        self.stale_commits_conflict().await;
        self.events_are_logged_in_order_with_their_metadata().await;
        self.ids_list_every_entity().await;
    }

    async fn missing_entities_load_as_none(&self) {
        let loaded = self.store.load(id(100)).await.expect("load");
        assert!(loaded.is_none(), "an unknown id must load as None");
    }

    async fn commits_advance_the_version(&self) {
        let mut sandbox = Versioned::new(sandbox(200));
        self.store
            .commit(&mut sandbox, &meta())
            .await
            .expect("insert");
        assert_eq!(sandbox.version(), Version::from(1));
        let mut loaded = self.load(200).await;
        assert_eq!(loaded.version(), Version::from(1));
        loaded.entity_mut().stop();
        self.store
            .commit(&mut loaded, &meta())
            .await
            .expect("update");
        assert_eq!(
            loaded.version(),
            Version::from(3),
            "stop records two events"
        );
        let reloaded = self.load(200).await;
        assert_eq!(reloaded.version(), Version::from(3));
        assert_eq!(reloaded.entity(), loaded.entity());
    }

    async fn stale_commits_conflict(&self) {
        let mut sandbox = Versioned::new(sandbox(300));
        self.store
            .commit(&mut sandbox, &meta())
            .await
            .expect("insert");
        let mut first = self.load(300).await;
        let mut second = self.load(300).await;
        first.entity_mut().stop();
        self.store
            .commit(&mut first, &meta())
            .await
            .expect("first commit wins");
        second.entity_mut().stop();
        let result = self.store.commit(&mut second, &meta()).await;
        assert!(
            matches!(result, Err(StorageError::Conflict { .. })),
            "a stale commit must conflict: {result:?}"
        );
    }

    async fn events_are_logged_in_order_with_their_metadata(&self) {
        let before = self.last_sequence().await;
        let mut sandbox = Versioned::new(sandbox(400));
        sandbox.entity_mut().stop();
        self.store
            .commit(&mut sandbox, &meta())
            .await
            .expect("insert");
        let logged = self.events.read(before, 100).await.expect("read");
        let kinds: Vec<&str> = logged.iter().map(|event| event.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "igloo.sandbox.created",
                "igloo.sandbox.stop_requested",
                "igloo.sandbox.status_recorded"
            ]
        );
        let versions: Vec<u64> = logged.iter().map(|event| event.stream_version).collect();
        assert_eq!(versions, [1, 2, 3]);
        assert!(
            logged
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        assert!(
            logged
                .iter()
                .all(|event| event.subject == id(400).to_string())
        );
        assert!(logged.iter().all(|event| event.actor == meta().actor));
        let mut head = self.events.head();
        let reached = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            head.wait_for(|head| *head >= logged[2].sequence),
        )
        .await;
        assert!(
            matches!(reached, Ok(Ok(_))),
            "the head must reach the last commit"
        );
    }

    async fn last_sequence(&self) -> Sequence {
        let all = self
            .events
            .read(Sequence::START, 10_000)
            .await
            .expect("read");
        all.last().map_or(Sequence::START, |event| event.sequence)
    }

    async fn ids_list_every_entity(&self) {
        let ids = self.store.ids().await.expect("ids");
        for n in [200, 300, 400] {
            assert!(ids.contains(&id(n)), "{n} must be listed");
        }
    }

    async fn load(&self, n: u128) -> Versioned<Sandbox> {
        match self.store.load(id(n)).await {
            Ok(Some(sandbox)) => sandbox,
            other => panic!("sandbox {n} must load: {other:?}"),
        }
    }
}

/// The `Checkpoints` contract.
pub(crate) struct CheckpointsConformance {
    pub(crate) checkpoints: Arc<dyn Checkpoints>,
}

impl CheckpointsConformance {
    pub(crate) async fn run_all(&self) {
        let start = self.checkpoints.load("unknown").await.expect("load");
        assert_eq!(
            start,
            Sequence::START,
            "an unknown consumer starts at the beginning"
        );
        self.checkpoints
            .save("a", Sequence::from(5))
            .await
            .expect("save");
        self.checkpoints
            .save("b", Sequence::from(9))
            .await
            .expect("save");
        self.checkpoints
            .save("a", Sequence::from(7))
            .await
            .expect("save");
        assert_eq!(
            self.checkpoints.load("a").await.expect("load"),
            Sequence::from(7)
        );
        assert_eq!(
            self.checkpoints.load("b").await.expect("load"),
            Sequence::from(9)
        );
    }
}

/// The `BlobStore` contract.
pub(crate) struct BlobStoreConformance {
    pub(crate) blobs: Arc<dyn BlobStore>,
}

impl BlobStoreConformance {
    pub(crate) async fn run_all(&self) {
        let bytes = b"layer contents".to_vec();
        let digest = Digest::from_blake3(*blake3::hash(&bytes).as_bytes());
        assert!(!self.blobs.contains(digest).await.expect("contains"));
        assert!(self.blobs.get(digest).await.expect("get").is_none());

        self.put(digest, bytes.clone()).await.expect("put");
        assert!(self.blobs.contains(digest).await.expect("contains"));
        assert_eq!(self.read(digest).await, bytes);
        self.put(digest, bytes.clone())
            .await
            .expect("storing again is a no-op");

        let other = Digest::from_blake3([9; 32]);
        let result = self.put(other, bytes).await;
        assert!(
            matches!(result, Err(BlobError::DigestMismatch { .. })),
            "mismatched content must be rejected: {result:?}"
        );
        assert!(!self.blobs.contains(other).await.expect("contains"));
    }

    async fn put(&self, digest: Digest, bytes: Vec<u8>) -> Result<(), BlobError> {
        self.blobs
            .put(digest, Box::pin(std::io::Cursor::new(bytes)))
            .await
    }

    async fn read(&self, digest: Digest) -> Vec<u8> {
        let Some(mut reader) = self.blobs.get(digest).await.expect("get") else {
            panic!("{digest} must be stored");
        };
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.expect("read");
        bytes
    }
}

/// The `ImageRegistry` contract, over a registry serving [`Self::layers`] as `image` for
/// [`Self::platform`].
pub(crate) struct ImageRegistryConformance {
    pub(crate) registry: Arc<dyn ImageRegistry>,
    pub(crate) image: ImageReference,
}

impl ImageRegistryConformance {
    /// The platform the fixture image is published for.
    pub(crate) fn platform() -> Platform {
        "linux/amd64".parse().expect("platform")
    }

    /// The fixture image's environment.
    pub(crate) fn env() -> Vec<(String, String)> {
        vec![("PATH".to_owned(), "/opt/tool/bin:/usr/bin:/bin".to_owned())]
    }

    /// The fixture image's layers, lowest first.
    pub(crate) fn layers() -> Vec<(MediaType, Vec<u8>)> {
        vec![
            (MediaType::TarGzip, b"compressed base layer".to_vec()),
            (MediaType::Tar, b"plain top layer".to_vec()),
        ]
    }

    pub(crate) async fn run_all(&self) {
        let pulled = self
            .registry
            .pull(&self.image, &Self::platform())
            .await
            .expect("pull");
        assert_eq!(pulled.env, Self::env());
        let pulled = pulled.layers;
        let expected = Self::layers();
        assert_eq!(pulled.len(), expected.len(), "every layer, once");
        for (layer, (media_type, bytes)) in pulled.iter().zip(&expected) {
            assert_eq!(layer.media_type, *media_type);
            assert_eq!(
                layer.digest,
                Digest::from_blake3(*blake3::hash(bytes).as_bytes())
            );
            assert_eq!(&std::fs::read(&layer.file).expect("layer file"), bytes);
        }

        let other = "linux/s390x".parse().expect("platform");
        let result = self.registry.pull(&self.image, &other).await;
        assert!(
            matches!(result, Err(RegistryError::NoMatchingPlatform(_))),
            "a missing platform must be reported: {result:?}"
        );

        let missing = format!(
            "{}/{}:missing",
            self.image.registry(),
            self.image.repository()
        )
        .parse()
        .expect("reference");
        let result = self.registry.pull(&missing, &Self::platform()).await;
        assert!(
            matches!(result, Err(RegistryError::NotFound)),
            "a missing tag must be reported: {result:?}"
        );
    }
}

/// The `IdempotencyStore` contract.
pub(crate) struct IdempotencyStoreConformance {
    pub(crate) store: Arc<dyn IdempotencyStore>,
}

impl IdempotencyStoreConformance {
    pub(crate) async fn run_all(&self) {
        let human = Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(1)),
        };
        let other = Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(2)),
        };
        let request = |actor, key, fingerprint: u8, now: i64| KeyedRequest {
            actor,
            key,
            fingerprint: Digest::from_blake3([fingerprint; 32]),
            now: at(now),
            records_before: at(now - 1_000),
            claims_before: at(now - 100),
        };
        let response = StoredResponse {
            status: 201,
            body: b"{}".to_vec(),
        };

        assert_eq!(
            self.claim(&request(&human, "k", 1, 0)).await,
            Claim::Claimed
        );
        assert_eq!(
            self.claim(&request(&human, "k", 1, 10)).await,
            Claim::InProgress
        );
        assert_eq!(
            self.claim(&request(&human, "k", 2, 10)).await,
            Claim::Mismatch
        );
        assert_eq!(
            self.claim(&request(&other, "k", 2, 10)).await,
            Claim::Claimed,
            "keys are scoped to their actor"
        );
        self.store
            .complete(&human, "k", &response)
            .await
            .expect("complete");
        assert_eq!(
            self.claim(&request(&human, "k", 1, 20)).await,
            Claim::Completed(response.clone())
        );
        self.store.release(&human, "k").await.expect("release");
        assert_eq!(
            self.claim(&request(&human, "k", 1, 30)).await,
            Claim::Completed(response),
            "a completed record is not released"
        );
        assert_eq!(
            self.claim(&request(&human, "k", 2, 2_000)).await,
            Claim::Claimed,
            "a stale record is replaced"
        );

        assert_eq!(
            self.claim(&request(&human, "j", 1, 0)).await,
            Claim::Claimed
        );
        self.store.release(&human, "j").await.expect("release");
        assert_eq!(
            self.claim(&request(&human, "j", 2, 1)).await,
            Claim::Claimed
        );
        assert_eq!(
            self.claim(&request(&human, "j", 1, 500)).await,
            Claim::Claimed,
            "an abandoned claim goes stale"
        );
    }

    async fn claim(&self, request: &KeyedRequest<'_>) -> Claim {
        self.store.claim(request).await.expect("claim")
    }
}

fn at(seconds: i64) -> Timestamp {
    NOW.saturating_add(jiff::SignedDuration::from_secs(seconds))
}

/// The `Forge` contract, against a bare origin repository on disk.
pub(crate) struct ForgeConformance {
    forge: Arc<dyn Forge>,
    dir: std::path::PathBuf,
    origin: igloo_git::testing::Fixture,
    remote: Remote,
}

impl ForgeConformance {
    /// An origin with one commit on `main` under `dir`.
    pub(crate) fn new(forge: Arc<dyn Forge>, dir: &std::path::Path) -> Self {
        let origin = igloo_git::testing::Fixture::new(dir);
        let remote = Remote {
            repo: Id::from_uuid(Uuid::from_u128(1)),
            location: origin.location(),
            token: None,
        };
        origin.commit("first", &[("file", Some("first"))]);
        Self {
            forge,
            dir: dir.to_owned(),
            origin,
            remote,
        }
    }

    pub(crate) async fn run_all(&self) {
        let main = Self::branch("main");
        let first = self.forge.fetch(&self.remote, &main).await.expect("fetch");
        assert_eq!(first, self.origin.head("main"));
        let missing = self
            .forge
            .fetch(&self.remote, &Self::branch("missing"))
            .await;
        assert!(
            matches!(missing, Err(ForgeError::BranchNotFound(_))),
            "{missing:?}"
        );

        let change = Self::branch("igloo/changes/1");
        self.push(&first, &change, Expected::Absent)
            .await
            .expect("create a branch");
        assert_eq!(self.origin.head("igloo/changes/1"), first);
        self.push(&first, &change, Expected::Absent)
            .await
            .expect("pushing the commit a branch is at changes nothing");

        let second = self.origin.commit("second", &[("file", Some("second"))]);
        self.mirrored(&main, &first).await;
        assert_eq!(
            self.forge.fetch(&self.remote, &main).await.expect("fetch"),
            second
        );
        assert_eq!(
            self.forge
                .mirrored(self.remote.repo, &main)
                .await
                .expect("mirrored"),
            Some(second.clone()),
            "a fetch moves the mirrored head"
        );
        let again = self.push(&second, &change, Expected::Absent).await;
        assert!(matches!(again, Err(ForgeError::Moved(_))), "{again:?}");
        self.push(&second, &change, Expected::Any)
            .await
            .expect("overwrite");
        let stale = self.push(&first, &main, Expected::At(first.clone())).await;
        assert!(
            matches!(stale, Err(ForgeError::Moved(_))),
            "a push expecting the old head is refused: {stale:?}"
        );
        assert_eq!(
            self.forge.fetch(&self.remote, &main).await.expect("fetch"),
            second
        );
        self.push(&first, &main, Expected::At(second.clone()))
            .await
            .expect("a push expecting the current head");
        assert_eq!(self.origin.head("main"), first);

        let unknown: CommitId = "0".repeat(40).parse().expect("commit");
        let result = self.push(&unknown, &change, Expected::Any).await;
        assert!(
            matches!(result, Err(ForgeError::CommitNotFound(_))),
            "{result:?}"
        );
        self.mirror_operations(&main, &second).await;
        self.bundles().await;
        self.squashes().await;
        self.diffs().await;
        self.deletes().await;
    }

    /// A branch is deleted only while it is where the caller saw it; deleting it again succeeds.
    async fn deletes(&self) {
        let head = self
            .forge
            .fetch(&self.remote, &Self::branch("main"))
            .await
            .expect("fetch");
        let done = Self::branch("igloo/done");
        self.push(&head, &done, Expected::Absent)
            .await
            .expect("create");
        self.forge
            .delete(&self.remote, &done, &head)
            .await
            .expect("delete");
        assert!(
            matches!(
                self.forge.fetch(&self.remote, &done).await,
                Err(ForgeError::BranchNotFound(_))
            ),
            "the branch is gone"
        );
        self.forge
            .delete(&self.remote, &done, &head)
            .await
            .expect("deleting a deleted branch succeeds");

        let moved = Self::branch("igloo/moved");
        self.push(&head, &moved, Expected::Absent)
            .await
            .expect("create");
        let elsewhere = self.origin.commit("elsewhere", &[("elsewhere", Some("1"))]);
        self.forge
            .fetch(&self.remote, &Self::branch("main"))
            .await
            .expect("fetch");
        self.push(&elsewhere, &moved, Expected::Any)
            .await
            .expect("move");
        let refused = self.forge.delete(&self.remote, &moved, &head).await;
        assert!(matches!(refused, Err(ForgeError::Moved(_))), "{refused:?}");
        assert_eq!(
            self.origin.head("igloo/moved"),
            elsewhere,
            "a moved branch stays"
        );
    }

    /// `mirrored` reads the mirror as last fetched: not the origin's newer head, nothing for a
    /// missing branch or a repository never fetched.
    async fn mirrored(&self, main: &BranchName, fetched: &CommitId) {
        let repo = self.remote.repo;
        assert_eq!(
            self.forge.mirrored(repo, main).await.expect("mirrored"),
            Some(fetched.clone()),
            "the origin moved, the mirror did not"
        );
        assert_eq!(
            self.forge
                .mirrored(repo, &Self::branch("missing"))
                .await
                .expect("mirrored"),
            None
        );
        let unfetched = Id::from_uuid(Uuid::from_u128(2));
        assert_eq!(
            self.forge
                .mirrored(unfetched, main)
                .await
                .expect("mirrored"),
            None
        );
    }

    /// A diff lists added, modified, renamed and deleted files with their counts and the patch git
    /// prints for each, and flags binary files.
    async fn diffs(&self) {
        let main = Self::branch("main");
        let repo = self.remote.repo;
        let body = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";
        let base = self.origin.commit(
            "diff base",
            &[
                ("d/keep", Some("1\n2\n")),
                ("d/gone", Some("bye\n")),
                ("d/old name", Some(body)),
                ("d/blob", Some("a\0b")),
            ],
        );
        let head = self.origin.commit(
            "diff head",
            &[
                ("d/keep", Some("1\n2\n3\n")),
                ("d/gone", None),
                ("d/old name", None),
                (
                    "d/new name",
                    Some("one\ntwo\nthree\nfour\nfive\nsix\nseven\nEIGHT\n"),
                ),
                ("d/added", Some("hello\n")),
                ("d/blob", Some("a\0c")),
            ],
        );
        self.forge.fetch(&self.remote, &main).await.expect("fetch");

        let files = self.forge.diff(repo, &base, &head).await.expect("diff");
        let summary: Vec<_> = files
            .iter()
            .map(|file| {
                (
                    file.path.as_str(),
                    file.previous_path.as_deref(),
                    file.status,
                    (file.additions, file.deletions),
                    file.binary,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("d/added", None, FileStatus::Added, (1, 0), false),
                ("d/blob", None, FileStatus::Modified, (0, 0), true),
                ("d/gone", None, FileStatus::Deleted, (0, 1), false),
                ("d/keep", None, FileStatus::Modified, (1, 0), false),
                (
                    "d/new name",
                    Some("d/old name"),
                    FileStatus::Renamed,
                    (1, 1),
                    false
                ),
            ]
        );
        let patch = |path: &str| {
            files
                .iter()
                .find(|file| file.path == path)
                .map(|file| file.patch.as_str())
                .expect("file")
        };
        let keep = patch("d/keep");
        assert!(keep.starts_with("diff --git a/d/keep b/d/keep\n"), "{keep}");
        assert!(keep.ends_with("@@ -1,2 +1,3 @@\n 1\n 2\n+3\n"), "{keep}");
        assert!(patch("d/blob").is_empty(), "a binary file has no patch");
        assert!(patch("d/gone").contains("-bye\n"));

        assert_eq!(self.forge.diff(repo, &head, &head).await.expect("diff"), []);
        let unknown: CommitId = "1".repeat(40).parse().expect("commit");
        let missing = self.forge.diff(repo, &base, &unknown).await;
        assert!(
            matches!(missing, Err(ForgeError::CommitNotFound(_))),
            "{missing:?}"
        );
    }

    /// A squash is one commit with the head's tree on the base, the same for equal inputs.
    async fn squashes(&self) {
        let main = Self::branch("main");
        let repo = self.remote.repo;
        let onto = self.forge.fetch(&self.remote, &main).await.expect("fetch");
        self.origin.commit("one", &[("squashed/1", Some("1"))]);
        let head = self.origin.commit("two", &[("squashed/2", Some("2"))]);
        self.forge.fetch(&self.remote, &main).await.expect("fetch");

        let squash = self
            .forge
            .squash(repo, &onto, &head, "Squashed", at(0))
            .await
            .expect("squash");
        let again = self
            .forge
            .squash(repo, &onto, &head, "Squashed", at(0))
            .await
            .expect("squash again");
        assert_eq!(again, squash, "equal inputs give the same commit");
        assert_eq!(
            self.forge
                .changed_paths(repo, &squash, &head)
                .await
                .expect("diff"),
            Vec::<String>::new(),
            "the squash holds the head's tree"
        );
        assert_eq!(
            self.forge.commits(repo, &onto, &squash).await.expect("log"),
            [(squash.clone(), "Squashed".to_owned())],
            "one commit on the base"
        );
        let empty = self
            .forge
            .squash(repo, &onto, &onto, "Nothing", at(0))
            .await;
        assert!(matches!(empty, Err(ForgeError::Git(_))), "{empty:?}");
    }

    /// Commits made elsewhere arrive as a bundle and leave through a push.
    async fn bundles(&self) {
        let base = self.origin.git(&["rev-parse", "HEAD"]);
        self.forge
            .fetch(&self.remote, &Self::branch("main"))
            .await
            .expect("fetch");
        self.origin
            .git(&["commit", "--quiet", "--allow-empty", "-m", "bundled"]);
        let head: CommitId = self
            .origin
            .git(&["rev-parse", "HEAD"])
            .parse()
            .expect("commit");
        let bundle = self.dir.join("task.bundle");
        let path = bundle.display().to_string();
        self.origin.git(&[
            "bundle",
            "create",
            "--quiet",
            &path,
            &format!("{base}..HEAD"),
        ]);
        let imported = self
            .forge
            .import(self.remote.repo, &bundle)
            .await
            .expect("import");
        assert_eq!(imported, head);
        let branch = Self::branch("igloo/tasks/1");
        self.push(&imported, &branch, Expected::Any)
            .await
            .expect("push the imported commit");
        assert_eq!(self.origin.head("igloo/tasks/1"), head);
        std::fs::write(&bundle, "not a bundle").expect("write");
        let garbage = self.forge.import(self.remote.repo, &bundle).await;
        assert!(matches!(garbage, Err(ForgeError::Git(_))), "{garbage:?}");
    }

    async fn mirror_operations(&self, main: &BranchName, second: &CommitId) {
        let with_extra = self
            .origin
            .commit("with extra", &[("extra", Some("extra"))]);
        let without = self.origin.commit("without extra", &[("extra", None)]);
        self.forge.fetch(&self.remote, main).await.expect("fetch");

        let tree = self.dir.join("checkout");
        self.forge
            .checkout(self.remote.repo, &with_extra, &tree)
            .await
            .expect("checkout");
        assert_eq!(std::fs::read(tree.join("extra")).expect("extra"), b"extra");
        assert_eq!(
            self.origin
                .git(&["-C", &tree.display().to_string(), "rev-parse", "HEAD"]),
            with_extra.to_string(),
            "the checkout has its commit's .git"
        );

        assert_eq!(
            self.forge
                .read_file(self.remote.repo, second, "file")
                .await
                .expect("read"),
            Some(b"second".to_vec())
        );
        assert_eq!(
            self.forge
                .read_file(self.remote.repo, second, "missing")
                .await
                .expect("read"),
            None
        );
        assert_eq!(
            self.forge
                .deleted_paths(self.remote.repo, &with_extra, &without)
                .await
                .expect("diff"),
            ["extra"]
        );
        let commits = self
            .forge
            .commits(self.remote.repo, second, &without)
            .await
            .expect("log");
        assert_eq!(
            commits,
            [
                (with_extra.clone(), "with extra".to_owned()),
                (without.clone(), "without extra".to_owned()),
            ]
        );
        assert_eq!(
            self.forge
                .changed_paths(self.remote.repo, second, &with_extra)
                .await
                .expect("diff"),
            ["extra"]
        );
        assert_eq!(
            self.forge
                .merge_base(self.remote.repo, &with_extra, &without)
                .await
                .expect("merge base"),
            Some(with_extra.clone())
        );
        let unknown: CommitId = "1".repeat(40).parse().expect("commit");
        let missing = self
            .forge
            .checkout(self.remote.repo, &unknown, &tree.with_extension("none"))
            .await;
        assert!(
            matches!(missing, Err(ForgeError::CommitNotFound(_))),
            "{missing:?}"
        );
    }

    async fn push(
        &self,
        commit: &CommitId,
        branch: &BranchName,
        expected: Expected,
    ) -> Result<(), ForgeError> {
        self.forge
            .push(&self.remote, commit, branch, expected)
            .await
    }

    fn branch(name: &str) -> BranchName {
        name.parse().expect("branch")
    }
}

/// The `SecretStore` contract.
pub(crate) struct SecretStoreConformance {
    pub(crate) store: Arc<dyn SecretStore>,
}

impl SecretStoreConformance {
    pub(crate) async fn run_all(&self) {
        let (first, second): (RepoId, RepoId) = (
            Id::from_uuid(Uuid::from_u128(1)),
            Id::from_uuid(Uuid::from_u128(2)),
        );
        let name: SecretName = "API_TOKEN".parse().expect("name");
        let value = SecretValue::try_from("s3cr3t".to_owned()).expect("value");
        assert_eq!(self.store.get(first, &name).await.expect("get"), None);
        self.store.put(first, &name, &value).await.expect("put");
        assert_eq!(
            self.store.get(first, &name).await.expect("get"),
            Some(value)
        );
        assert_eq!(
            self.store.get(second, &name).await.expect("get"),
            None,
            "secrets belong to their repository"
        );
        let replaced = SecretValue::try_from("rotated".to_owned()).expect("value");
        self.store
            .put(first, &name, &replaced)
            .await
            .expect("replace");
        assert_eq!(
            self.store.get(first, &name).await.expect("get"),
            Some(replaced)
        );
        let other: SecretName = "ANOTHER".parse().expect("name");
        self.store
            .put(
                first,
                &other,
                &SecretValue::try_from(String::new()).expect("empty"),
            )
            .await
            .expect("put");
        assert_eq!(
            self.store.names(first).await.expect("names"),
            [other.clone(), name.clone()]
        );
        assert_eq!(self.store.names(second).await.expect("names"), []);
        assert!(self.store.delete(first, &name).await.expect("delete"));
        assert!(!self.store.delete(first, &name).await.expect("delete again"));
        assert_eq!(self.store.names(first).await.expect("names"), [other]);
    }
}

/// The `LogStore` contract.
pub(crate) struct LogStoreConformance {
    pub(crate) logs: Arc<dyn LogStore>,
}

impl LogStoreConformance {
    pub(crate) async fn run_all(&self) {
        use igloo_core::job::JobId;
        use igloo_core::process::OutputStream::{Stderr, Stdout};

        let job: JobId = Id::from_uuid(Uuid::from_u128(1));
        let other: JobId = Id::from_uuid(Uuid::from_u128(2));
        assert_eq!(self.logs.read(job, 0, 10).await.expect("read").len(), 0);
        self.logs
            .append(job, Stdout, 0, b"one\n")
            .await
            .expect("append");
        self.logs
            .append(job, Stderr, 0, b"oops\n")
            .await
            .expect("append");
        self.logs
            .append(job, Stdout, 4, b"two\n")
            .await
            .expect("append");
        self.logs
            .append(job, Stdout, 0, b"one\n")
            .await
            .expect("resent chunks are ignored");
        self.logs
            .append(other, Stdout, 0, b"else\n")
            .await
            .expect("append");

        let entries = self.logs.read(job, 0, 10).await.expect("read");
        let chunks: Vec<(_, &[u8])> = entries
            .iter()
            .map(|entry| (entry.stream, entry.data.as_slice()))
            .collect();
        assert_eq!(
            chunks,
            [
                (Stdout, &b"one\n"[..]),
                (Stderr, b"oops\n"),
                (Stdout, b"two\n")
            ]
        );
        assert!(
            entries
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        let rest = self
            .logs
            .read(job, entries[0].sequence, 1)
            .await
            .expect("read");
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].sequence, entries[1].sequence);
    }
}

fn id(n: u128) -> Id<Sandbox> {
    Id::from_uuid(Uuid::from_u128(n))
}

fn sandbox(n: u128) -> Sandbox {
    let spec = SandboxSpec::builder()
        .snapshot(SnapshotId::from(Digest::from_blake3([1; 32])))
        .build();
    match Sandbox::new(id(n), spec, NOW) {
        Ok(sandbox) => sandbox,
        Err(error) => panic!("a running spec is valid: {error}"),
    }
}

fn meta() -> CommitMeta {
    CommitMeta {
        actor: Actor::Human {
            user: Id::from_uuid(Uuid::from_u128(1)),
        },
        time: NOW,
        correlation_id: Id::from_uuid(Uuid::from_u128(2)),
        causation_id: None,
    }
}
