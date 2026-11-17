//! Sandbox tools exposed to builtin agents: `bash`, `read_file`, `write_file`, `list_dir`.

use ac_core::tool::ToolSpec;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AgentId, ExecRequest, Sandbox, SandboxError, SandboxLimits};

pub const SANDBOX_TOOLS: &[&str] = &["bash", "read_file", "write_file", "list_dir"];
pub const READ_CAP: usize = 256 * 1024;
pub const WRITE_CAP: usize = 4 * 1024 * 1024;

pub fn sandbox_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bash".into(),
            description: "在你自己的沙箱（/workspace）執行 bash 指令。預設沒有對外網路。輸出上限 64KB。".into(),
            input_schema: json!({"type":"object","properties":{
                "command":{"type":"string"},
                "timeout_secs":{"type":"integer","minimum":1,"maximum":600}
            },"required":["command"]}),
        },
        ToolSpec {
            name: "read_file".into(),
            description: "讀取沙箱中的文字檔（路徑需在 /workspace 下，上限 256KB）。".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        },
        ToolSpec {
            name: "write_file".into(),
            description: "寫入沙箱中的檔案（路徑需在 /workspace 下，會建立上層目錄、覆寫既有檔案）。".into(),
            input_schema: json!({"type":"object","properties":{
                "path":{"type":"string"},
                "content":{"type":"string"}
            },"required":["path","content"]}),
        },
        ToolSpec {
            name: "list_dir".into(),
            description: "列出沙箱目錄內容（預設 /workspace）。".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}}}),
        },
    ]
}

/// Resolves a path under /workspace, rejecting traversal and odd characters.
pub fn workspace_path(p: &str) -> Result<String, SandboxError> {
    let p = p.trim();
    let rel = p.strip_prefix("/workspace").unwrap_or(p).trim_start_matches('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => return Err(SandboxError::Invalid("路徑不能包含 ..".into())),
            s if s.contains('\0') => return Err(SandboxError::Invalid("路徑包含非法字元".into())),
            s => parts.push(s),
        }
    }
    if p.starts_with('/') && !p.starts_with("/workspace") {
        return Err(SandboxError::Invalid("只能存取 /workspace 下的路徑".into()));
    }
    Ok(if parts.is_empty() { "/workspace".into() } else { format!("/workspace/{}", parts.join("/")) })
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, SandboxError> {
    serde_json::from_value(if args.is_null() { json!({}) } else { args })
        .map_err(|e| SandboxError::Invalid(format!("參數錯誤：{e}")))
}

pub async fn call_sandbox_tool(
    sb: &dyn Sandbox,
    agent: AgentId,
    limits: &SandboxLimits,
    name: &str,
    args: Value,
) -> Result<Value, SandboxError> {
    match name {
        "bash" => {
            #[derive(Deserialize)]
            struct A {
                command: String,
                timeout_secs: Option<u64>,
            }
            let a: A = parse(args)?;
            let out = sb
                .exec(
                    agent,
                    limits,
                    ExecRequest { command: a.command, stdin: None, timeout_secs: a.timeout_secs.unwrap_or(120) },
                )
                .await?;
            Ok(serde_json::to_value(out).unwrap_or_default())
        }
        "read_file" => {
            #[derive(Deserialize)]
            struct A {
                path: String,
            }
            let a: A = parse(args)?;
            let path = workspace_path(&a.path)?;
            let cmd = format!(
                "test -f {p} || {{ echo 'not a file' >&2; exit 2; }}; head -c {cap} {p} | base64 -w0",
                p = sh_quote(&path),
                cap = READ_CAP + 1
            );
            let out = sb.exec(agent, limits, ExecRequest { command: cmd, stdin: None, timeout_secs: 30 }).await?;
            if out.exit_code != 0 {
                return Err(SandboxError::Invalid(format!("無法讀取 {path}：{}", out.stderr.trim())));
            }
            let mut bytes = B64.decode(out.stdout.trim()).map_err(|e| SandboxError::Docker(e.to_string()))?;
            let truncated = bytes.len() > READ_CAP;
            bytes.truncate(READ_CAP);
            Ok(json!({"path": path, "content": String::from_utf8_lossy(&bytes), "truncated": truncated}))
        }
        "write_file" => {
            #[derive(Deserialize)]
            struct A {
                path: String,
                content: String,
            }
            let a: A = parse(args)?;
            if a.content.len() > WRITE_CAP {
                return Err(SandboxError::Invalid("檔案太大（上限 4MB）".into()));
            }
            let path = workspace_path(&a.path)?;
            if path == "/workspace" {
                return Err(SandboxError::Invalid("請指定檔名".into()));
            }
            let cmd = format!("mkdir -p \"$(dirname {p})\" && cat > {p}", p = sh_quote(&path));
            let out = sb
                .exec(
                    agent,
                    limits,
                    ExecRequest { command: cmd, stdin: Some(a.content.clone().into_bytes()), timeout_secs: 30 },
                )
                .await?;
            if out.exit_code != 0 {
                return Err(SandboxError::Invalid(format!("無法寫入 {path}：{}", out.stderr.trim())));
            }
            Ok(json!({"path": path, "bytes": a.content.len()}))
        }
        "list_dir" => {
            #[derive(Deserialize)]
            struct A {
                path: Option<String>,
            }
            let a: A = parse(args)?;
            let path = workspace_path(a.path.as_deref().unwrap_or("/workspace"))?;
            let cmd = format!("ls -la --time-style=+%Y-%m-%dT%H:%M {}", sh_quote(&path));
            let out = sb.exec(agent, limits, ExecRequest { command: cmd, stdin: None, timeout_secs: 30 }).await?;
            if out.exit_code != 0 {
                return Err(SandboxError::Invalid(format!("無法列出 {path}：{}", out.stderr.trim())));
            }
            Ok(json!({"path": path, "listing": out.stdout}))
        }
        other => Err(SandboxError::Invalid(format!("未知的沙箱工具：{other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_in_workspace() {
        assert_eq!(workspace_path("notes.md").unwrap(), "/workspace/notes.md");
        assert_eq!(workspace_path("/workspace/a/./b").unwrap(), "/workspace/a/b");
        assert_eq!(workspace_path("").unwrap(), "/workspace");
        assert!(workspace_path("../etc/passwd").is_err());
        assert!(workspace_path("/etc/passwd").is_err());
        assert!(workspace_path("/workspace/../x").is_err());
    }
}
