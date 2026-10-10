use async_trait::async_trait;
use igloo_core::worker::RuntimeKind;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::child::Supervised;
use super::pty::PtyProcess;
use super::{
    ExitOutcome, LocalSandbox, OutputChunk, Process, RuntimeError, SandboxRuntime, TerminalIo,
    TerminalProcess,
};

/// Runs processes directly on the host, in the sandbox's directory. No isolation: development
/// and tests only, which the configuration enforces.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessRuntime;

#[async_trait]
impl SandboxRuntime for ProcessRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Process
    }

    async fn start(&self, _sandbox: &LocalSandbox) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn exec(
        &self,
        sandbox: &LocalSandbox,
        process: Process,
        output: mpsc::Sender<OutputChunk>,
        cancel: CancellationToken,
    ) -> Result<ExitOutcome, RuntimeError> {
        let Some((program, args)) = process.argv.split_first() else {
            return Err(RuntimeError::Spawn(std::io::Error::other("empty argv")));
        };
        let workspace = sandbox.root.join("workspace");
        tokio::fs::create_dir_all(&workspace).await?;
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(&workspace)
            .envs(&sandbox.env)
            .envs(&process.env);
        Supervised::spawn(&mut command, None)?
            .wait(process.timeout, &output, &cancel)
            .await
    }

    async fn exec_terminal(
        &self,
        sandbox: &LocalSandbox,
        process: TerminalProcess,
        io: TerminalIo,
        cancel: CancellationToken,
    ) -> Result<ExitOutcome, RuntimeError> {
        let Some((program, args)) = process.argv.split_first() else {
            return Err(RuntimeError::Spawn(std::io::Error::other("empty argv")));
        };
        let workspace = sandbox.root.join("workspace");
        tokio::fs::create_dir_all(&workspace).await?;
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(&workspace)
            .envs(&sandbox.env)
            .envs(&process.env);
        PtyProcess::spawn(&mut command, process.size, None)?
            .run(io, &cancel)
            .await
    }

    async fn stop(&self, _sandbox: &LocalSandbox) -> Result<(), RuntimeError> {
        Ok(())
    }
}
