//! Docker implementation: one container + one named volume (`/workspace`) per agent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bollard::Docker;
use bollard::exec::{StartExecOptions, StartExecResults};
use bollard::models::{ContainerCreateBody, ExecConfig, HostConfig, VolumeCreateRequest};
use bollard::query_parameters::{
    CreateContainerOptions, RemoveContainerOptions, RemoveVolumeOptions, StartContainerOptions, StopContainerOptions,
};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::{AgentId, ExecOutput, ExecRequest, NetworkPolicy, Sandbox, SandboxError, SandboxLimits};

pub const WORKSPACE: &str = "/workspace";
const UID: &str = "1000:1000";
const OUTPUT_CAP: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct DockerSandboxConfig {
    pub image: String,
    pub max: SandboxLimits,
    pub idle_stop: Duration,
    pub max_exec_secs: u64,
}

impl Default for DockerSandboxConfig {
    fn default() -> Self {
        DockerSandboxConfig {
            image: "python:3.12-slim".into(),
            max: SandboxLimits { cpus: 2.0, memory_mb: 2048, pids: 512, disk_mb: 4096, network: NetworkPolicy::Open },
            idle_stop: Duration::from_secs(15 * 60),
            max_exec_secs: 600,
        }
    }
}

impl DockerSandboxConfig {
    pub fn from_env() -> Self {
        let mut c = DockerSandboxConfig::default();
        if let Ok(img) = std::env::var("SANDBOX_IMAGE")
            && !img.trim().is_empty()
        {
            c.image = img;
        }
        if let Some(m) = std::env::var("SANDBOX_IDLE_STOP_SECS").ok().and_then(|v| v.parse().ok()) {
            c.idle_stop = Duration::from_secs(m);
        }
        if std::env::var("SANDBOX_ALLOW_NETWORK").map(|v| v == "false" || v == "0").unwrap_or(false) {
            c.max.network = NetworkPolicy::None;
        }
        c
    }
}

pub struct DockerSandbox {
    docker: Option<Docker>,
    cfg: DockerSandboxConfig,
    last_used: Arc<Mutex<HashMap<AgentId, Instant>>>,
    init_lock: tokio::sync::Mutex<()>,
}

fn container_name(agent: AgentId) -> String {
    format!("crewconvo-agent-{}", agent.simple())
}
fn volume_name(agent: AgentId) -> String {
    format!("crewconvo-ws-{}", agent.simple())
}
fn derr(e: bollard::errors::Error) -> SandboxError {
    SandboxError::Docker(e.to_string())
}
fn is_404(e: &bollard::errors::Error) -> bool {
    matches!(e, bollard::errors::Error::DockerResponseServerError { status_code: 404, .. })
}
fn is_304_or_409(e: &bollard::errors::Error) -> bool {
    matches!(e, bollard::errors::Error::DockerResponseServerError { status_code: 304 | 409, .. })
}

impl DockerSandbox {
    pub fn new(cfg: DockerSandboxConfig) -> Arc<Self> {
        let docker = Docker::connect_with_local_defaults().ok();
        let sb = Arc::new(DockerSandbox {
            docker,
            cfg,
            last_used: Arc::new(Mutex::new(HashMap::new())),
            init_lock: tokio::sync::Mutex::new(()),
        });
        // Idle reaper: stop containers unused for `idle_stop` (volumes are kept).
        let weak = Arc::downgrade(&sb);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                let Some(sb) = weak.upgrade() else { return };
                let idle: Vec<AgentId> = {
                    let map = sb.last_used.lock().unwrap();
                    map.iter().filter(|(_, t)| t.elapsed() > sb.cfg.idle_stop).map(|(a, _)| *a).collect()
                };
                for a in idle {
                    tracing::info!(agent = %a, "stopping idle sandbox");
                    let _ = sb.stop(a).await;
                }
            }
        });
        sb
    }

    pub fn config(&self) -> &DockerSandboxConfig {
        &self.cfg
    }

    fn docker(&self) -> Result<&Docker, SandboxError> {
        self.docker.as_ref().ok_or_else(|| SandboxError::Unavailable("無法連線到 Docker".into()))
    }

    fn touch(&self, agent: AgentId) {
        self.last_used.lock().unwrap().insert(agent, Instant::now());
    }

    async fn container_running(&self, name: &str) -> Result<Option<bool>, SandboxError> {
        match self.docker()?.inspect_container(name, None).await {
            Ok(info) => Ok(Some(info.state.and_then(|s| s.running).unwrap_or(false))),
            Err(e) if is_404(&e) => Ok(None),
            Err(e) => Err(derr(e)),
        }
    }

    /// Makes the fresh volume writable by the unprivileged sandbox user. Runs a one-shot
    /// container that only has CAP_CHOWN and no network.
    async fn init_volume(&self, agent: AgentId) -> Result<(), SandboxError> {
        let d = self.docker()?;
        let name = format!("{}-init", container_name(agent));
        let _ = d.remove_container(&name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
        let body = ContainerCreateBody {
            image: Some(self.cfg.image.clone()),
            user: Some("0:0".into()),
            entrypoint: Some(vec!["sh".into(), "-c".into()]),
            cmd: Some(vec![format!("mkdir -p {WORKSPACE}/memory && chown -R {UID} {WORKSPACE}")]),
            network_disabled: Some(true),
            host_config: Some(HostConfig {
                // nocopy: no copy-up from the image, which would reset the volume owner to root.
                binds: Some(vec![format!("{}:{WORKSPACE}:nocopy", volume_name(agent))]),
                cap_drop: Some(vec!["ALL".into()]),
                cap_add: Some(vec!["CHOWN".into()]),
                security_opt: Some(vec!["no-new-privileges".into()]),
                readonly_rootfs: Some(true),
                network_mode: Some("none".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        d.create_container(Some(CreateContainerOptions { name: Some(name.clone()), ..Default::default() }), body)
            .await
            .map_err(derr)?;
        d.start_container(&name, None::<StartContainerOptions>).await.map_err(derr)?;
        // Poll until the one-shot container has exited, then check its exit code.
        let deadline = Instant::now() + Duration::from_secs(60);
        let exit = loop {
            let info = d.inspect_container(&name, None).await.map_err(derr)?;
            let state = info.state.unwrap_or_default();
            if !state.running.unwrap_or(false)
                && state.finished_at.as_deref().map(|f| !f.starts_with("0001")).unwrap_or(false)
            {
                break state.exit_code.unwrap_or(-1);
            }
            if Instant::now() > deadline {
                break -1;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let _ = d.remove_container(&name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
        if exit != 0 {
            return Err(SandboxError::Docker(format!("workspace 初始化失敗（exit {exit}）")));
        }
        Ok(())
    }

    async fn create(&self, agent: AgentId, limits: &SandboxLimits) -> Result<(), SandboxError> {
        let d = self.docker()?;
        let vol = volume_name(agent);
        let fresh = match d.inspect_volume(&vol).await {
            Ok(_) => false,
            Err(e) if is_404(&e) => {
                d.create_volume(VolumeCreateRequest {
                    name: Some(vol.clone()),
                    labels: Some(HashMap::from([("crewconvo.agent".to_string(), agent.to_string())])),
                    ..Default::default()
                })
                .await
                .map_err(derr)?;
                true
            }
            Err(e) => return Err(derr(e)),
        };
        if fresh {
            self.init_volume(agent).await?;
        }
        let net = if limits.network == NetworkPolicy::Open { "bridge" } else { "none" };
        let body = ContainerCreateBody {
            image: Some(self.cfg.image.clone()),
            user: Some(UID.into()),
            entrypoint: Some(vec!["sleep".into()]),
            cmd: Some(vec!["infinity".into()]),
            working_dir: Some(WORKSPACE.into()),
            env: Some(vec!["HOME=/workspace".into(), "PYTHONDONTWRITEBYTECODE=1".into()]),
            labels: Some(HashMap::from([("crewconvo.agent".to_string(), agent.to_string())])),
            network_disabled: Some(limits.network == NetworkPolicy::None),
            host_config: Some(HostConfig {
                binds: Some(vec![format!("{vol}:{WORKSPACE}:nocopy")]),
                cap_drop: Some(vec!["ALL".into()]),
                security_opt: Some(vec!["no-new-privileges".into()]),
                readonly_rootfs: Some(true),
                privileged: Some(false),
                tmpfs: Some(HashMap::from([("/tmp".to_string(), "rw,noexec,nosuid,size=64m".to_string())])),
                network_mode: Some(net.into()),
                memory: Some(limits.memory_mb * 1024 * 1024),
                nano_cpus: Some((limits.cpus * 1e9) as i64),
                pids_limit: Some(limits.pids),
                init: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };
        d.create_container(
            Some(CreateContainerOptions { name: Some(container_name(agent)), ..Default::default() }),
            body,
        )
        .await
        .map_err(derr)?;
        Ok(())
    }

    async fn run(
        &self,
        agent: AgentId,
        command: Vec<String>,
        stdin: Option<Vec<u8>>,
        timeout: u64,
    ) -> Result<ExecOutput, SandboxError> {
        let d = self.docker()?;
        let exec = d
            .create_exec(
                &container_name(agent),
                ExecConfig {
                    attach_stdin: Some(stdin.is_some()),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(command),
                    user: Some(UID.into()),
                    working_dir: Some(WORKSPACE.into()),
                    ..Default::default()
                },
            )
            .await
            .map_err(derr)?;
        let started = d
            .start_exec(&exec.id, Some(StartExecOptions { detach: false, tty: false, output_capacity: None }))
            .await
            .map_err(derr)?;
        let StartExecResults::Attached { mut output, mut input } = started else {
            return Err(SandboxError::Docker("exec not attached".into()));
        };
        if let Some(data) = stdin {
            input.write_all(&data).await.map_err(|e| SandboxError::Docker(e.to_string()))?;
            input.shutdown().await.map_err(|e| SandboxError::Docker(e.to_string()))?;
        }
        drop(input);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut truncated = false;
        let collect = async {
            while let Some(chunk) = output.next().await {
                match chunk {
                    Ok(bollard::container::LogOutput::StdOut { message }) => {
                        if stdout.len() < OUTPUT_CAP {
                            stdout.extend_from_slice(&message);
                        } else {
                            truncated = true;
                        }
                    }
                    Ok(bollard::container::LogOutput::StdErr { message }) => {
                        if stderr.len() < OUTPUT_CAP {
                            stderr.extend_from_slice(&message);
                        } else {
                            truncated = true;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => return Err(derr(e)),
                }
            }
            Ok(())
        };
        match tokio::time::timeout(Duration::from_secs(timeout + 5), collect).await {
            Ok(r) => r?,
            Err(_) => return Err(SandboxError::Timeout(timeout)),
        }
        if stdout.len() > OUTPUT_CAP || stderr.len() > OUTPUT_CAP {
            truncated = true;
        }
        stdout.truncate(OUTPUT_CAP);
        stderr.truncate(OUTPUT_CAP);
        let exit_code = d.inspect_exec(&exec.id).await.map_err(derr)?.exit_code.unwrap_or(-1);
        Ok(ExecOutput {
            exit_code,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            truncated,
        })
    }

    /// Bytes used under /workspace.
    async fn usage(&self, agent: AgentId) -> Result<u64, SandboxError> {
        let out = self.run(agent, vec!["du".into(), "-sb".into(), WORKSPACE.into()], None, 30).await?;
        Ok(out.stdout.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0))
    }
}

#[async_trait]
impl Sandbox for DockerSandbox {
    async fn ensure_started(&self, agent: AgentId, limits: &SandboxLimits) -> Result<(), SandboxError> {
        let limits = limits.clamped(&self.cfg.max);
        let _g = self.init_lock.lock().await;
        let name = container_name(agent);
        match self.container_running(&name).await? {
            Some(true) => {}
            Some(false) => match self.docker()?.start_container(&name, None::<StartContainerOptions>).await {
                Ok(()) => {}
                Err(e) if is_304_or_409(&e) => {}
                Err(e) => return Err(derr(e)),
            },
            None => {
                self.create(agent, &limits).await?;
                self.docker()?.start_container(&name, None::<StartContainerOptions>).await.map_err(derr)?;
            }
        }
        self.touch(agent);
        Ok(())
    }

    async fn exec(&self, agent: AgentId, limits: &SandboxLimits, req: ExecRequest) -> Result<ExecOutput, SandboxError> {
        let limits = limits.clamped(&self.cfg.max);
        self.ensure_started(agent, &limits).await?;
        let quota = (limits.disk_mb as u64) * 1024 * 1024;
        let used = self.usage(agent).await?;
        let cmd = req.command.trim_start();
        let cleanup = ["rm ", "du ", "ls", "find ", "cat ", "head ", "tail "].iter().any(|p| cmd.starts_with(p));
        if used > quota && !cleanup {
            return Err(SandboxError::QuotaExceeded { used, quota });
        }
        let t = req.timeout_secs.clamp(1, self.cfg.max_exec_secs);
        let wrapped = vec![
            "timeout".into(),
            "-s".into(),
            "KILL".into(),
            format!("{t}"),
            "bash".into(),
            "-c".into(),
            req.command.clone(),
        ];
        let out = self.run(agent, wrapped, req.stdin, t).await;
        self.touch(agent);
        let mut out = out?;
        if out.exit_code == 137 {
            out.stderr.push_str(&format!("\n（指令被終止：可能逾時 {t} 秒或超出記憶體上限）"));
        }
        Ok(out)
    }

    async fn stop(&self, agent: AgentId) -> Result<(), SandboxError> {
        self.last_used.lock().unwrap().remove(&agent);
        match self
            .docker()?
            .stop_container(&container_name(agent), Some(StopContainerOptions { t: Some(5), ..Default::default() }))
            .await
        {
            Ok(()) => Ok(()),
            Err(e) if is_404(&e) || is_304_or_409(&e) => Ok(()),
            Err(e) => Err(derr(e)),
        }
    }

    async fn destroy(&self, agent: AgentId) -> Result<(), SandboxError> {
        let d = self.docker()?;
        self.last_used.lock().unwrap().remove(&agent);
        match d
            .remove_container(
                &container_name(agent),
                Some(RemoveContainerOptions { force: true, v: false, link: false }),
            )
            .await
        {
            Ok(()) => {}
            Err(e) if is_404(&e) => {}
            Err(e) => return Err(derr(e)),
        }
        match d.remove_volume(&volume_name(agent), Some(RemoveVolumeOptions { force: true })).await {
            Ok(()) => Ok(()),
            Err(e) if is_404(&e) => Ok(()),
            Err(e) => Err(derr(e)),
        }
    }

    async fn available(&self) -> bool {
        match &self.docker {
            Some(d) => d.ping().await.is_ok(),
            None => false,
        }
    }
}
