//! Per-agent sandboxes (spec 9.3). The model loop runs in the platform process; only tool
//! commands run inside the agent's container, so API keys never enter the sandbox.
//!
//! Docker isolation is not strong enough for untrusted code from strangers; the [`Sandbox`]
//! trait exists so the implementation can be swapped for gVisor or microVMs.

mod docker;
pub mod tools;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub use docker::{DockerSandbox, DockerSandboxConfig};

pub type AgentId = Uuid;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("沙箱無法使用：{0}")]
    Unavailable(String),
    #[error("沙箱錯誤：{0}")]
    Docker(String),
    #[error("指令逾時（{0} 秒）")]
    Timeout(u64),
    #[error("{0}")]
    Invalid(String),
    #[error("workspace 已超過磁碟配額（{used} / {quota} bytes），請先刪除檔案")]
    QuotaExceeded { used: u64, quota: u64 },
}

/// Network policy of a sandbox. Allow-listing specific hosts needs an egress firewall and is
/// not offered; `open` is an explicit opt-in.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    #[default]
    None,
    Open,
}

/// Per-agent limits (`agents.sandbox_config`), clamped to the global maximums.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SandboxLimits {
    pub cpus: f64,
    pub memory_mb: i64,
    pub pids: i64,
    pub disk_mb: i64,
    pub network: NetworkPolicy,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        SandboxLimits { cpus: 1.0, memory_mb: 512, pids: 256, disk_mb: 1024, network: NetworkPolicy::None }
    }
}

impl SandboxLimits {
    pub fn from_json(v: &serde_json::Value) -> Self {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }

    pub fn clamped(&self, max: &SandboxLimits) -> SandboxLimits {
        SandboxLimits {
            cpus: self.cpus.clamp(0.1, max.cpus),
            memory_mb: self.memory_mb.clamp(64, max.memory_mb),
            pids: self.pids.clamp(16, max.pids),
            disk_mb: self.disk_mb.clamp(16, max.disk_mb),
            network: if max.network == NetworkPolicy::Open { self.network } else { NetworkPolicy::None },
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecRequest {
    pub command: String,
    pub stdin: Option<Vec<u8>>,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecOutput {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

#[async_trait]
pub trait Sandbox: Send + Sync {
    async fn ensure_started(&self, agent: AgentId, limits: &SandboxLimits) -> Result<(), SandboxError>;
    async fn exec(&self, agent: AgentId, limits: &SandboxLimits, cmd: ExecRequest) -> Result<ExecOutput, SandboxError>;
    async fn stop(&self, agent: AgentId) -> Result<(), SandboxError>;
    /// Removes the container and the agent's workspace volume (irreversible).
    async fn destroy(&self, agent: AgentId) -> Result<(), SandboxError>;
    /// Whether the backend is reachable.
    async fn available(&self) -> bool;
}
