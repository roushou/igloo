//! The platform's primitives as modules: each registers its commands, controllers and reactors
//! through one `Extension`.

mod build;
mod change;
mod checkout;
mod job;
mod repo;
mod sandbox;
mod seal;
mod snapshot;
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
pub use repo::{
    DeleteSecret, RecordImage, RecordWarmSnapshot, RegisterRepo, RepoError, RepoModule,
    RepoQueries, SetSecret,
};
pub use sandbox::{
    CreateSandbox, PlacementError, RecordSandboxStatus, SandboxModule, SandboxQueries,
    ScheduleSandbox, StopSandbox,
};
pub use seal::{CompleteSeal, CreateSeal, CreateSealError, FailSeal, SealModule, SealQueries};
pub use snapshot::{
    ImageImporter, RegisterSnapshot, SnapshotError, SnapshotModule, Snapshots, StoreBlob,
};
pub use trust::TrustSettings;
pub use usage::WorkerUsages;
pub use worker::{
    ConnectWorker, DisconnectWorker, DrainWorker, MarkWorkerLost, RegisterWorker, WorkerModule,
};
