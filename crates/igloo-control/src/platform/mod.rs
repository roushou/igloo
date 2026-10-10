//! The platform's primitives as modules: each registers its commands, controllers and reactors
//! through one `Extension`.

mod build;
mod change;
mod checkout;
mod job;
mod mirror;
mod push;
mod repo;
mod sandbox;
mod seal;
mod snapshot;
mod storage;
mod trust;
mod usage;
mod worker;

#[cfg(test)]
mod tests;

pub use build::{
    BuildModule, BuildQueries, RecordBuildJob, RecordBuildRecorded, RecordBuildSandboxStopped,
    RecordBuildSeal, RecordBuildSealing, RecordBuildStarted, StartBuild,
};
pub use change::{
    ApproveChange, ChangeHeadError, ChangeHeads, ChangeModule, ChangeQueries, CloseChange,
    CommentChange, Heads, MergeChange, OpenChange, RequestChanges, ReviseChange,
};
pub use checkout::{RepoSnapshots, WarmRecipe};
pub use job::{
    CancelJob, FailJob, FailLostLease, FinishJob, JobModule, JobQueries, LeaseJob, RenewJobLease,
    StartJob, SubmitJob, SubmitJobError,
};
pub use mirror::{ForgeMirror, MirrorModule};
pub use push::{BranchPushes, Branches, PushedChange};
pub use repo::{
    DeleteSecret, ForgetWarmSnapshots, RecordImage, RecordWarmSnapshot, RecordWarmUse,
    RegisterRepo, RepoError, RepoModule, RepoQueries, SetDotfiles, SetSecret,
};
pub use sandbox::{
    CreateSandbox, PlacementError, RecordSandboxStatus, SandboxModule, SandboxQueries,
    ScheduleSandbox, StopSandbox,
};
pub use seal::{CompleteSeal, CreateSeal, CreateSealError, FailSeal, SealModule, SealQueries};
pub use snapshot::{
    ImageImporter, RegisterSnapshot, SnapshotError, SnapshotModule, Snapshots, StoreBlob,
};
pub use storage::{LayerCollector, SnapshotSize, StorageUsage, Sweep};
pub use trust::TrustSettings;
pub use usage::WorkerUsages;
pub use worker::{
    ConnectWorker, DisconnectWorker, DrainWorker, MarkWorkerLost, RegisterWorker, WorkerModule,
};
