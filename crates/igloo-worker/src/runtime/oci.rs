use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use igloo_core::sandbox::{NetworkPolicy, SandboxId};
use igloo_core::worker::RuntimeKind;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::child::Supervised;
use super::netns::NetworkNamespaces;
use super::{ExitOutcome, LocalSandbox, OutputChunk, Process, RuntimeError, SandboxRuntime};

/// Runs every job as a container over its sandbox's root file system, through an OCI runtime
/// binary (youki, crun or runc).
///
/// Each job gets an OCI bundle under `<sandbox>/bundles/`: `/workspace` as working directory,
/// fresh PID, IPC, UTS, mount and cgroup namespaces, the sandbox's CPU and memory limits as
/// cgroup v2 resources (no swap), and the network: under [`NetworkPolicy::DenyAll`] the
/// sandbox's own network namespace, shared by its jobs, with only loopback, up; under
/// [`NetworkPolicy::AllowAll`] the host's network. Writes land in the sandbox's root, so later
/// jobs see them.
///
/// Invariant: every container this runtime started for a sandbox is deleted by the time its
/// job ends or the sandbox stops, and its network namespace once the sandbox stops.
pub struct OciRuntime {
    binary: PathBuf,
    state: PathBuf,
    namespaces: NetworkNamespaces,
    next: AtomicU64,
    containers: Mutex<HashMap<SandboxId, HashSet<String>>>,
}

impl OciRuntime {
    /// The capabilities left to container processes: the defaults of common container engines.
    const CAPABILITIES: [&'static str; 14] = [
        "CAP_AUDIT_WRITE",
        "CAP_CHOWN",
        "CAP_DAC_OVERRIDE",
        "CAP_FOWNER",
        "CAP_FSETID",
        "CAP_KILL",
        "CAP_MKNOD",
        "CAP_NET_BIND_SERVICE",
        "CAP_NET_RAW",
        "CAP_SETFCAP",
        "CAP_SETGID",
        "CAP_SETPCAP",
        "CAP_SETUID",
        "CAP_SYS_CHROOT",
    ];
    const PIDS_LIMIT: u32 = 4096;
    const PATH: &'static str = "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
    /// The home of the root user processes run as, unless the sandbox sets another.
    const HOME: &'static str = "HOME=/root";

    /// A runtime driving `binary`, keeping container state under `state`.
    #[must_use]
    pub fn new(binary: PathBuf, state: PathBuf) -> Self {
        Self {
            binary,
            namespaces: NetworkNamespaces::new(state.join("netns")),
            state,
            next: AtomicU64::new(0),
            containers: Mutex::new(HashMap::new()),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command.arg("--root").arg(&self.state);
        command
    }

    /// Kills and deletes `container`, ignoring one that is already gone.
    async fn delete(&self, container: &str) {
        let deleted = self
            .command()
            .args(["delete", "--force", container])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if let Err(error) = deleted {
            tracing::warn!(%container, %error, "deleting the container failed");
        }
    }

    fn track(&self, sandbox: SandboxId, container: &str, running: bool) {
        let mut containers = self
            .containers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let set = containers.entry(sandbox).or_default();
        if running {
            set.insert(container.to_owned());
        } else {
            set.remove(container);
        }
    }

    fn bundles(sandbox: &LocalSandbox) -> PathBuf {
        sandbox
            .root
            .parent()
            .map_or_else(|| sandbox.root.join("..bundles"), |dir| dir.join("bundles"))
    }

    /// The OCI runtime configuration of one job.
    fn config(&self, sandbox: &LocalSandbox, process: &Process, hostname: &str) -> Value {
        let mut env: Vec<String> = sandbox
            .env
            .iter()
            .chain(&process.env)
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        if !env.iter().any(|entry| entry.starts_with("PATH=")) {
            env.push(Self::PATH.to_owned());
        }
        if !env.iter().any(|entry| entry.starts_with("HOME=")) {
            env.push(Self::HOME.to_owned());
        }
        let mut namespaces = vec![
            json!({ "type": "pid" }),
            json!({ "type": "ipc" }),
            json!({ "type": "uts" }),
            json!({ "type": "mount" }),
            json!({ "type": "cgroup" }),
        ];
        let mut mounts = Self::mounts();
        match sandbox.network {
            NetworkPolicy::DenyAll => namespaces.push(json!({
                "type": "network",
                "path": self.namespaces.path(sandbox.id),
            })),
            NetworkPolicy::AllowAll => mounts.push(json!({
                "destination": "/etc/resolv.conf",
                "type": "bind",
                "source": "/etc/resolv.conf",
                "options": ["rbind", "ro"],
            })),
        }
        let memory = u64::from(sandbox.limits.memory_mib()) * 1024 * 1024;
        let quota = u64::from(sandbox.limits.millicpus()) * 100;
        json!({
            "ociVersion": "1.0.2",
            "process": {
                "terminal": false,
                "user": { "uid": 0, "gid": 0 },
                "args": process.argv,
                "env": env,
                "cwd": "/workspace",
                "noNewPrivileges": true,
                "capabilities": {
                    "bounding": Self::CAPABILITIES,
                    "effective": Self::CAPABILITIES,
                    "permitted": Self::CAPABILITIES,
                },
                "rlimits": [{ "type": "RLIMIT_NOFILE", "hard": 65536, "soft": 65536 }],
            },
            "root": { "path": sandbox.root, "readonly": false },
            "hostname": hostname,
            "mounts": mounts,
            "linux": {
                "namespaces": namespaces,
                "resources": {
                    "memory": { "limit": memory, "swap": memory },
                    "cpu": { "quota": quota, "period": 100_000 },
                    "pids": { "limit": Self::PIDS_LIMIT },
                    // Some runtimes leave swap unlimited when it equals the memory limit.
                    "unified": { "memory.swap.max": "0" },
                },
                "maskedPaths": [
                    "/proc/acpi", "/proc/kcore", "/proc/keys", "/proc/latency_stats",
                    "/proc/timer_list", "/proc/timer_stats", "/proc/sched_debug",
                    "/proc/scsi", "/sys/firmware",
                ],
                "readonlyPaths": [
                    "/proc/asound", "/proc/bus", "/proc/fs", "/proc/irq", "/proc/sys",
                    "/proc/sysrq-trigger",
                ],
            },
        })
    }

    fn mounts() -> Vec<Value> {
        vec![
            json!({ "destination": "/proc", "type": "proc", "source": "proc" }),
            json!({
                "destination": "/dev",
                "type": "tmpfs",
                "source": "tmpfs",
                "options": ["nosuid", "strictatime", "mode=755", "size=65536k"],
            }),
            json!({
                "destination": "/dev/pts",
                "type": "devpts",
                "source": "devpts",
                "options": ["nosuid", "noexec", "newinstance", "ptmxmode=0666", "mode=0620"],
            }),
            json!({
                "destination": "/dev/shm",
                "type": "tmpfs",
                "source": "shm",
                "options": ["nosuid", "noexec", "nodev", "mode=1777", "size=65536k"],
            }),
            json!({
                "destination": "/dev/mqueue",
                "type": "mqueue",
                "source": "mqueue",
                "options": ["nosuid", "noexec", "nodev"],
            }),
            json!({
                "destination": "/sys",
                "type": "sysfs",
                "source": "sysfs",
                "options": ["nosuid", "noexec", "nodev", "ro"],
            }),
            json!({
                "destination": "/sys/fs/cgroup",
                "type": "cgroup",
                "source": "cgroup",
                "options": ["nosuid", "noexec", "nodev", "relatime", "ro"],
            }),
        ]
    }
}

#[async_trait]
impl SandboxRuntime for OciRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Oci
    }

    async fn start(&self, sandbox: &LocalSandbox) -> Result<(), RuntimeError> {
        tokio::fs::create_dir_all(sandbox.root.join("workspace")).await?;
        tokio::fs::create_dir_all(Self::bundles(sandbox)).await?;
        tokio::fs::create_dir_all(&self.state).await?;
        if sandbox.network == NetworkPolicy::DenyAll {
            self.namespaces.create(sandbox.id).await?;
        }
        Ok(())
    }

    async fn exec(
        &self,
        sandbox: &LocalSandbox,
        process: Process,
        output: mpsc::Sender<OutputChunk>,
        cancel: CancellationToken,
    ) -> Result<ExitOutcome, RuntimeError> {
        if process.argv.is_empty() {
            return Err(RuntimeError::Spawn(std::io::Error::other("empty argv")));
        }
        let number = self.next.fetch_add(1, Ordering::Relaxed);
        let container = format!("igloo-{}-{number}", sandbox.id);
        let bundle = Self::bundles(sandbox).join(number.to_string());
        tokio::fs::create_dir_all(&bundle).await?;
        let config = self.config(sandbox, &process, "sandbox");
        let bytes = serde_json::to_vec_pretty(&config).map_err(std::io::Error::other)?;
        tokio::fs::write(bundle.join("config.json"), bytes).await?;
        // A container left by an earlier worker run under the same name would block this one.
        self.delete(&container).await;

        let mut run = self.command();
        run.arg("run").arg("--bundle").arg(&bundle).arg(&container);
        let mut kill = self.command();
        kill.args(["kill", &container, "KILL"]);
        self.track(sandbox.id, &container, true);
        let outcome = match Supervised::spawn(&mut run, Some(kill)) {
            Ok(child) => child.wait(process.timeout, &output, &cancel).await,
            Err(error) => Err(error),
        };
        self.delete(&container).await;
        self.track(sandbox.id, &container, false);
        Self::remove_dir(&bundle).await;
        outcome
    }

    async fn stop(&self, sandbox: &LocalSandbox) -> Result<(), RuntimeError> {
        let containers = self
            .containers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&sandbox.id)
            .unwrap_or_default();
        for container in containers {
            self.delete(&container).await;
        }
        Self::remove_dir(&Self::bundles(sandbox)).await;
        self.namespaces.remove(sandbox.id)?;
        Ok(())
    }
}

impl OciRuntime {
    async fn remove_dir(path: &Path) {
        if let Err(error) = tokio::fs::remove_dir_all(path).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %path.display(), %error, "removing a bundle failed");
        }
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    //! Against a real OCI runtime, as root: set `IGLOO_TEST_OCI=1`, `IGLOO_TEST_OCI_RUNTIME`
    //! (default `crun`) and `IGLOO_TEST_BUSYBOX` to a static busybox binary.

    use std::collections::BTreeMap;
    use std::time::Duration;

    use igloo_core::Id;
    use igloo_core::sandbox::ResourceLimits;
    use uuid::Uuid;

    use super::*;

    static SANDBOXES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    struct Fixture {
        dir: tempfile::TempDir,
        runtime: OciRuntime,
        sandbox: LocalSandbox,
    }

    impl Fixture {
        /// A sandbox whose root holds busybox, or `None` when the tests are not enabled.
        async fn start(network: NetworkPolicy) -> Option<Self> {
            std::env::var_os("IGLOO_TEST_OCI")?;
            let busybox =
                std::env::var("IGLOO_TEST_BUSYBOX").unwrap_or_else(|_| "/bin/busybox".into());
            let binary = std::env::var("IGLOO_TEST_OCI_RUNTIME").unwrap_or_else(|_| "crun".into());
            let dir = tempfile::tempdir().expect("dir");
            let root = dir.path().join("sandbox/rootfs");
            for sub in ["bin", "proc", "dev", "sys", "tmp", "etc"] {
                std::fs::create_dir_all(root.join(sub)).expect("mkdir");
            }
            std::fs::copy(&busybox, root.join("bin/busybox")).expect("busybox");
            for tool in ["sh", "ls", "cat", "dd", "echo", "pwd", "sleep", "test"] {
                std::os::unix::fs::symlink("busybox", root.join("bin").join(tool)).expect("link");
            }
            let runtime = OciRuntime::new(PathBuf::from(binary), dir.path().join("state"));
            let sandbox = LocalSandbox {
                id: Id::from_uuid(Uuid::from_u128(u128::from(
                    SANDBOXES.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                ))),
                generation: igloo_core::Generation::INITIAL,
                root,
                env: BTreeMap::new(),
                limits: ResourceLimits::new(1000, 128).expect("limits"),
                network,
            };
            runtime.start(&sandbox).await.expect("start");
            Some(Self {
                dir,
                runtime,
                sandbox,
            })
        }

        /// Runs `script` with `sh -c`; returns its outcome and stdout.
        async fn run(&self, script: &str) -> (ExitOutcome, String) {
            self.run_cancellable(script, CancellationToken::new()).await
        }

        async fn run_cancellable(
            &self,
            script: &str,
            cancel: CancellationToken,
        ) -> (ExitOutcome, String) {
            let (output, mut chunks) = mpsc::channel(64);
            let process = Process {
                argv: vec!["sh".into(), "-c".into(), script.into()],
                env: BTreeMap::new(),
                timeout: Duration::from_secs(30),
            };
            let outcome = self
                .runtime
                .exec(&self.sandbox, process, output, cancel)
                .await
                .expect("exec");
            let mut stdout = Vec::new();
            while let Ok(chunk) = chunks.try_recv() {
                if chunk.stream == igloo_core::process::OutputStream::Stdout {
                    stdout.extend(chunk.data);
                }
            }
            (outcome, String::from_utf8_lossy(&stdout).into_owned())
        }
    }

    #[tokio::test]
    async fn a_job_sees_only_its_sandbox_and_keeps_its_writes() {
        let Some(fixture) = Fixture::start(NetworkPolicy::DenyAll).await else {
            return;
        };
        let host = fixture.dir.path().join("host-only");
        std::fs::write(&host, "secret").expect("host file");
        let script = format!("test ! -e {} && pwd && echo kept > notes", host.display());
        let (outcome, stdout) = fixture.run(&script).await;
        assert_eq!(outcome, ExitOutcome::Exited(0), "{stdout}");
        assert_eq!(stdout.trim(), "/workspace");
        let (outcome, stdout) = fixture.run("cat notes").await;
        assert_eq!(outcome, ExitOutcome::Exited(0));
        assert_eq!(stdout.trim(), "kept");
    }

    #[tokio::test]
    async fn deny_all_leaves_only_loopback_and_allow_all_shares_the_host_network() {
        let Some(denied) = Fixture::start(NetworkPolicy::DenyAll).await else {
            return;
        };
        let up = "for i in /sys/class/net/*; do [ \"$(cat $i/operstate)\" = up ] && basename $i; done; true";
        let (_, interfaces) = denied.run(up).await;
        assert_eq!(interfaces.trim(), "", "no interface but loopback is up");
        let (_, flags) = denied.run("cat /sys/class/net/lo/flags").await;
        assert_eq!(flags.trim(), "0x9", "loopback is up");
        let Some(allowed) = Fixture::start(NetworkPolicy::AllowAll).await else {
            return;
        };
        let (_, interfaces) = allowed.run(up).await;
        assert!(
            !interfaces.trim().is_empty(),
            "the host's interfaces are visible"
        );
    }

    #[tokio::test]
    async fn a_job_over_its_memory_limit_is_killed() {
        let Some(fixture) = Fixture::start(NetworkPolicy::DenyAll).await else {
            return;
        };
        let (outcome, stdout) = fixture
            .run("dd if=/dev/zero of=/dev/null bs=512M count=1; echo $?")
            .await;
        assert_eq!(outcome, ExitOutcome::Exited(0));
        assert_eq!(stdout.trim(), "137", "dd is killed");
    }

    #[tokio::test]
    async fn stopping_the_sandbox_removes_its_containers_and_bundles() {
        let Some(fixture) = Fixture::start(NetworkPolicy::DenyAll).await else {
            return;
        };
        let stopping = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            fixture.runtime.stop(&fixture.sandbox).await.expect("stop");
        };
        let ((outcome, _), ()) = tokio::join!(fixture.run("sleep 30"), stopping);
        assert!(
            matches!(outcome, ExitOutcome::Exited(code) if code != 0),
            "the job is killed: {outcome:?}"
        );
        let listed = fixture
            .runtime
            .command()
            .arg("list")
            .output()
            .await
            .expect("list");
        assert!(
            !String::from_utf8_lossy(&listed.stdout).contains("igloo-"),
            "no container is left"
        );
        assert!(!OciRuntime::bundles(&fixture.sandbox).exists());
    }

    #[tokio::test]
    async fn a_cancelled_job_is_killed() {
        let Some(fixture) = Fixture::start(NetworkPolicy::DenyAll).await else {
            return;
        };
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let cancelling = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            trigger.cancel();
        };
        let ((outcome, _), ()) =
            tokio::join!(fixture.run_cancellable("sleep 30", cancel), cancelling);
        assert_eq!(outcome, ExitOutcome::Cancelled);
    }
}
