//! Coding agents on this computer: detection, MCP + Skill setup, headless runs.
//!
//! The same registry as the `crewconvo` command (clients/python/crewconvo/platforms.py), so the
//! server itself can set them up when it runs on the user's own computer. File locations and
//! flags come from each tool's official documentation.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Map, Value, json};

pub const MCP_NAME: &str = "crewconvo";
const SKILL: &str = include_str!("../../../../clients/python/crewconvo/SKILL.md");
const BEGIN: &str = "# >>> crewconvo (managed by `crewconvo setup`; edits here are overwritten) >>>";
const END: &str = "# <<< crewconvo <<<";

/// What one wake runs: program, arguments, text for its standard input.
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub stdin: Option<String>,
}

pub struct Outcome {
    pub ok: bool,
    pub lines: Vec<String>,
}

impl Outcome {
    fn ok(lines: Vec<String>) -> Self {
        Outcome { ok: true, lines }
    }
    fn fail(line: String) -> Self {
        Outcome { ok: false, lines: vec![line] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    ClaudeCode,
    Codex,
    Cursor,
    OpenCode,
    Antigravity,
}

pub const ALL: [Platform; 5] =
    [Platform::ClaudeCode, Platform::Codex, Platform::Cursor, Platform::OpenCode, Platform::Antigravity];

pub fn by_id(id: &str) -> Option<Platform> {
    ALL.into_iter().find(|p| p.id() == id)
}

pub fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn home() -> PathBuf {
    home_dir()
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Finds a program on PATH (and in the per-user folders installers use, which a program started
/// from the desktop does not always have on its PATH).
pub fn which(names: &[&str]) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.push(home().join(".local").join("bin"));
    if cfg!(windows) {
        if let Some(d) = env_path("APPDATA") {
            dirs.push(d.join("npm"));
        }
    } else {
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
    }
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ".com"] } else { &[""] };
    for name in names {
        for dir in &dirs {
            for ext in exts {
                let p = dir.join(format!("{name}{ext}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// A command that never opens a console window (the desktop app has none of its own).
pub fn quiet_command(program: &Path) -> Command {
    let mut c = Command::new(program);
    c.stdin(Stdio::null()).current_dir(home());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Runs a short setup command; None when it could not start or took longer than 30 seconds.
fn run(program: &Path, args: &[&str]) -> Option<(bool, String)> {
    let mut child = quiet_command(program).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > Duration::from_secs(30) => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return None,
        }
    }
    let out = child.wait_with_output().ok()?;
    let mut text = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    }
    Some((out.status.success(), text.chars().take(300).collect()))
}

fn write_file(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".crewconvo-tmp");
    let tmp = path.with_file_name(name);
    std::fs::write(&tmp, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)
}

/// JSON with `//` comment lines and trailing commas, as several of these tools allow.
fn relaxed_json(raw: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str(raw) {
        return Some(v);
    }
    let no_comments: String = raw.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    let chars: Vec<char> = no_comments.chars().collect();
    let mut out = String::with_capacity(chars.len());
    for (i, c) in chars.iter().enumerate() {
        if *c == ',' {
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        out.push(*c);
    }
    serde_json::from_str(&out).ok()
}

/// The file as a JSON object: empty when missing, None when it exists but cannot be read as one.
fn load_json(path: &Path) -> Option<Map<String, Value>> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Some(Map::new());
    };
    let raw = raw.trim_start_matches('\u{feff}');
    if raw.trim().is_empty() {
        return Some(Map::new());
    }
    match relaxed_json(raw)? {
        Value::Object(o) => Some(o),
        _ => None,
    }
}

fn save_json(path: &Path, data: Map<String, Value>) -> bool {
    let text = serde_json::to_string_pretty(&Value::Object(data)).unwrap_or_default() + "\n";
    write_file(path, &text).is_ok()
}

fn object_at<'a>(mut node: &'a mut Map<String, Value>, keys: &[&str]) -> &'a mut Map<String, Value> {
    for k in keys {
        let entry = node.entry(k.to_string()).or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        node = entry.as_object_mut().expect("object");
    }
    node
}

fn existing_object<'a>(mut node: &'a mut Map<String, Value>, keys: &[&str]) -> Option<&'a mut Map<String, Value>> {
    for k in keys {
        node = node.get_mut(*k).and_then(Value::as_object_mut)?;
    }
    Some(node)
}

/// Sets data[keys…] = value. False when the file is not plain JSON or cannot be written.
fn json_set(path: &Path, keys: &[&str], value: Value) -> bool {
    let Some(mut data) = load_json(path) else { return false };
    let (last, parents) = keys.split_last().expect("keys");
    object_at(&mut data, parents).insert(last.to_string(), value);
    save_json(path, data)
}

fn json_del(path: &Path, keys: &[&str]) {
    if !path.exists() {
        return;
    }
    let Some(mut data) = load_json(path) else { return };
    let (last, parents) = keys.split_last().expect("keys");
    let removed = existing_object(&mut data, parents).is_some_and(|n| n.remove(*last).is_some());
    if removed {
        save_json(path, data);
    }
}

/// Adds `rule` to the list at data[keys…] (permission allow-lists).
fn json_allow(path: &Path, keys: &[&str], rule: &str) -> bool {
    let Some(mut data) = load_json(path) else { return false };
    let (last, parents) = keys.split_last().expect("keys");
    let node = object_at(&mut data, parents);
    let list = node.entry(last.to_string()).or_insert_with(|| json!([]));
    if !list.is_array() {
        *list = json!([]);
    }
    let arr = list.as_array_mut().expect("array");
    if arr.iter().any(|v| v.as_str() == Some(rule)) {
        return true;
    }
    arr.push(json!(rule));
    save_json(path, data)
}

fn json_disallow(path: &Path, keys: &[&str], rule: &str) {
    if !path.exists() {
        return;
    }
    let Some(mut data) = load_json(path) else { return };
    let (last, parents) = keys.split_last().expect("keys");
    let changed = existing_object(&mut data, parents)
        .and_then(|n| n.get_mut(*last))
        .and_then(Value::as_array_mut)
        .is_some_and(|arr| {
            let before = arr.len();
            arr.retain(|v| v.as_str() != Some(rule));
            arr.len() != before
        });
    if changed {
        save_json(path, data);
    }
}

fn install_skill(dir: &Path) -> String {
    let target = dir.join("crewconvo").join("SKILL.md");
    match write_file(&target, SKILL) {
        Ok(()) => format!("Skill：{}", target.display()),
        Err(e) => format!("Skill 沒有寫入（{e}）"),
    }
}

fn remove_skill(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir.join("crewconvo"));
}

/// Removes the block this program manages from a Codex config.toml.
fn strip_block(text: &str) -> String {
    let mut out = text.to_string();
    while let (Some(a), Some(b)) = (out.find(BEGIN), out.find(END)) {
        if b < a {
            break;
        }
        let mut end = b + END.len();
        if out[end..].starts_with("\r\n") {
            end += 2;
        } else if out[end..].starts_with('\n') {
            end += 1;
        }
        out.replace_range(a..end, "");
    }
    out
}

fn bearer(token: &str) -> Value {
    json!({ "Authorization": format!("Bearer {token}") })
}

impl Platform {
    pub fn id(self) -> &'static str {
        match self {
            Platform::ClaudeCode => "claude-code",
            Platform::Codex => "codex",
            Platform::Cursor => "cursor",
            Platform::OpenCode => "opencode",
            Platform::Antigravity => "antigravity",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Platform::ClaudeCode => "Claude Code",
            Platform::Codex => "Codex",
            Platform::Cursor => "Cursor",
            Platform::OpenCode => "OpenCode",
            Platform::Antigravity => "Google Antigravity",
        }
    }

    /// Used in suggested handles.
    pub fn short(self) -> &'static str {
        match self {
            Platform::ClaudeCode => "claude",
            Platform::Codex => "codex",
            Platform::Cursor => "cursor",
            Platform::OpenCode => "opencode",
            Platform::Antigravity => "antigravity",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Platform::ClaudeCode => "Anthropic 的 coding agent",
            Platform::Codex => "OpenAI Codex CLI",
            Platform::Cursor => "Cursor 編輯器與 Cursor CLI",
            Platform::OpenCode => "opencode.ai",
            Platform::Antigravity => "App、IDE 與 agy CLI",
        }
    }

    /// What has to be true before this one can be woken, when that is not obvious.
    pub fn wake_note(self) -> &'static str {
        match self {
            Platform::Cursor => "需要已登入的 Cursor CLI（agent）；用 CURSOR_API_KEY 或先互動登入一次",
            Platform::Antigravity => "需要 agy CLI 並先互動登入一次",
            _ => "",
        }
    }

    fn codex_dir() -> PathBuf {
        env_path("CODEX_HOME").unwrap_or_else(|| home().join(".codex"))
    }

    fn opencode_dir() -> PathBuf {
        env_path("OPENCODE_CONFIG_DIR").unwrap_or_else(|| home().join(".config").join("opencode"))
    }

    fn opencode_file() -> PathBuf {
        let d = Self::opencode_dir();
        if d.join("opencode.jsonc").exists() { d.join("opencode.jsonc") } else { d.join("opencode.json") }
    }

    fn gemini_cfg() -> PathBuf {
        home().join(".gemini").join("config")
    }

    pub fn detect(self) -> bool {
        match self {
            Platform::ClaudeCode => which(&["claude"]).is_some(),
            Platform::Codex => which(&["codex"]).is_some() || Self::codex_dir().exists(),
            Platform::Cursor => home().join(".cursor").exists() || which(&["cursor", "cursor-agent"]).is_some(),
            Platform::OpenCode => which(&["opencode"]).is_some() || Self::opencode_dir().exists(),
            Platform::Antigravity => {
                if which(&["agy", "antigravity"]).is_some()
                    || Self::gemini_cfg().exists()
                    || home().join(".gemini").join("antigravity").exists()
                {
                    return true;
                }
                if cfg!(windows) {
                    env_path("LOCALAPPDATA").is_some_and(|d| d.join("Programs").join("Antigravity").exists())
                } else if cfg!(target_os = "macos") {
                    Path::new("/Applications/Antigravity.app").exists()
                } else {
                    false
                }
            }
        }
    }

    /// The command-line program that can run it without a window, if there is one.
    pub fn runner(self) -> Option<PathBuf> {
        match self {
            Platform::ClaudeCode => which(&["claude"]),
            Platform::Codex => which(&["codex"]),
            Platform::Cursor => which(&["cursor-agent"]).or_else(|| {
                // The CLI is now called `agent`; only trust that generic name when Cursor is installed.
                if home().join(".cursor").exists() { which(&["agent"]) } else { None }
            }),
            Platform::OpenCode => which(&["opencode"]),
            Platform::Antigravity => which(&["agy"]),
        }
    }

    /// Adds the CrewConvo MCP server and Skill to this coding agent.
    pub fn install(self, server: &str, token: &str) -> Outcome {
        let url = format!("{server}/mcp");
        match self {
            Platform::ClaudeCode => {
                let Some(exe) = which(&["claude"]) else {
                    return Outcome::fail("找不到 claude 指令".into());
                };
                let _ = run(&exe, &["mcp", "remove", MCP_NAME, "-s", "user"]);
                let header = format!("Authorization: Bearer {token}");
                let add =
                    ["mcp", "add", "--transport", "http", "--scope", "user", MCP_NAME, &url, "--header", &header];
                match run(&exe, &add) {
                    Some((true, _)) => {}
                    Some((false, msg)) => return Outcome::fail(format!("claude mcp add 失敗：{msg}")),
                    None => return Outcome::fail("claude mcp add 沒有回應".into()),
                }
                Outcome::ok(vec![
                    format!("MCP server「{MCP_NAME}」已加入（claude mcp add --scope user）"),
                    install_skill(&home().join(".claude").join("skills")),
                ])
            }
            Platform::Codex => {
                let path = Self::codex_dir().join("config.toml");
                let text = std::fs::read_to_string(&path).map(|t| strip_block(&t)).unwrap_or_default();
                let own = format!("[mcp_servers.{MCP_NAME}]");
                if text.lines().any(|l| l.trim_start().starts_with(&own)) {
                    return Outcome::fail(format!("{} 已經有你自己寫的 {own}，請先移除它再試一次", path.display()));
                }
                let block = format!(
                    "{BEGIN}\n[mcp_servers.{MCP_NAME}]\nurl = \"{url}\"\nhttp_headers = {{ \"Authorization\" = \"Bearer {token}\" }}\ndefault_tools_approval_mode = \"approve\"\n{END}\n"
                );
                let head = if text.trim().is_empty() {
                    String::new()
                } else {
                    format!("{}\n\n", text.trim_end_matches(['\n', '\r']))
                };
                if let Err(e) = write_file(&path, &(head + &block)) {
                    return Outcome::fail(format!("無法寫入 {}：{e}", path.display()));
                }
                Outcome::ok(vec![
                    format!("MCP server 已寫入 {}", path.display()),
                    install_skill(&home().join(".agents").join("skills")),
                ])
            }
            Platform::Cursor => {
                let dir = home().join(".cursor");
                let path = dir.join("mcp.json");
                if !json_set(&path, &["mcpServers", MCP_NAME], json!({ "url": url, "headers": bearer(token) })) {
                    return Outcome::fail(format!("{} 不是有效的 JSON，請手動加入 MCP server", path.display()));
                }
                let mut lines =
                    vec![format!("MCP server 已寫入 {}", path.display()), install_skill(&dir.join("skills"))];
                let rule = format!("Mcp({MCP_NAME}:*)");
                if json_allow(&dir.join("cli-config.json"), &["permissions", "allow"], &rule) {
                    lines.push("Cursor CLI：已允許 crewconvo 的 MCP 工具（~/.cursor/cli-config.json）".into());
                }
                Outcome::ok(lines)
            }
            Platform::OpenCode => {
                let path = Self::opencode_file();
                let Some(data) = load_json(&path) else {
                    return Outcome::fail(format!("{} 無法解析，請手動加入 MCP server", path.display()));
                };
                let mcp = data.get("mcp").and_then(Value::as_object);
                let v2 = mcp.is_some_and(|m| m.get("servers").is_some_and(Value::is_object));
                let empty = mcp.is_none_or(|m| m.is_empty());
                if !v2
                    && empty
                    && let Some(exe) = which(&["opencode"])
                {
                    // V2 has a non-interactive `mcp add`; V1's is interactive and fails without a terminal.
                    let header = format!("Authorization=Bearer {token}");
                    let add = ["mcp", "add", MCP_NAME, "--url", &url, "--header", &header, "--global"];
                    if let Some((true, _)) = run(&exe, &add) {
                        return Outcome::ok(vec![
                            "MCP server 已加入（opencode mcp add --global）".into(),
                            install_skill(&Self::opencode_dir().join("skills")),
                        ]);
                    }
                }
                let written = if v2 {
                    let entry = json!({ "type": "remote", "url": url, "oauth": false, "headers": bearer(token) });
                    json_set(&path, &["mcp", "servers", MCP_NAME], entry)
                } else {
                    let entry = json!({
                        "type": "remote", "url": url, "enabled": true, "oauth": false, "headers": bearer(token)
                    });
                    json_set(&path, &["mcp", MCP_NAME], entry)
                };
                if !written {
                    return Outcome::fail(format!("無法寫入 {}", path.display()));
                }
                Outcome::ok(vec![
                    format!("MCP server 已寫入 {}（{} 格式）", path.display(), if v2 { "V2" } else { "V1" }),
                    install_skill(&Self::opencode_dir().join("skills")),
                ])
            }
            Platform::Antigravity => {
                let cfg = Self::gemini_cfg();
                let path = cfg.join("mcp_config.json");
                let entry = json!({ "serverUrl": url, "headers": bearer(token) });
                if !json_set(&path, &["mcpServers", MCP_NAME], entry) {
                    return Outcome::fail(format!("{} 不是有效的 JSON，請手動加入 MCP server", path.display()));
                }
                let mut lines =
                    vec![format!("MCP server 已寫入 {}", path.display()), install_skill(&cfg.join("skills"))];
                let settings = home().join(".gemini").join("antigravity-cli").join("settings.json");
                let rule = format!("mcp({MCP_NAME}/*)");
                if self.runner().is_some() && json_allow(&settings, &["permissions", "allow"], &rule) {
                    lines.push(format!("agy CLI：已允許 crewconvo 的 MCP 工具（{}）", settings.display()));
                }
                Outcome::ok(lines)
            }
        }
    }

    /// Removes only what [`Platform::install`] wrote.
    pub fn uninstall(self) {
        match self {
            Platform::ClaudeCode => {
                if let Some(exe) = which(&["claude"]) {
                    let _ = run(&exe, &["mcp", "remove", MCP_NAME, "-s", "user"]);
                }
                remove_skill(&home().join(".claude").join("skills"));
            }
            Platform::Codex => {
                let path = Self::codex_dir().join("config.toml");
                if let Ok(text) = std::fs::read_to_string(&path) {
                    let new = strip_block(&text);
                    if new != text {
                        let _ = write_file(&path, &new);
                    }
                }
                remove_skill(&home().join(".agents").join("skills"));
            }
            Platform::Cursor => {
                let dir = home().join(".cursor");
                json_del(&dir.join("mcp.json"), &["mcpServers", MCP_NAME]);
                let rule = format!("Mcp({MCP_NAME}:*)");
                json_disallow(&dir.join("cli-config.json"), &["permissions", "allow"], &rule);
                remove_skill(&dir.join("skills"));
            }
            Platform::OpenCode => {
                let path = Self::opencode_file();
                json_del(&path, &["mcp", "servers", MCP_NAME]);
                json_del(&path, &["mcp", MCP_NAME]);
                remove_skill(&Self::opencode_dir().join("skills"));
            }
            Platform::Antigravity => {
                let cfg = Self::gemini_cfg();
                json_del(&cfg.join("mcp_config.json"), &["mcpServers", MCP_NAME]);
                let settings = home().join(".gemini").join("antigravity-cli").join("settings.json");
                json_disallow(&settings, &["permissions", "allow"], &format!("mcp({MCP_NAME}/*)"));
                remove_skill(&cfg.join("skills"));
            }
        }
    }

    /// How to run one wake. `tools` are the MCP tool names this agent may use.
    pub fn invocation(self, prompt: &str, tools: &[String]) -> Option<Invocation> {
        let program = self.runner()?;
        // Windows passes arguments to .cmd launchers through cmd.exe, which cannot carry line breaks.
        let is_script = program
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
        let as_arg = || {
            if is_script { prompt.replace(['\r', '\n'], " ").replace(['"', '%'], "'") } else { prompt.to_string() }
        };
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let (args, stdin) = match self {
            Platform::ClaudeCode => {
                let allowed = if tools.is_empty() {
                    format!("mcp__{MCP_NAME}")
                } else {
                    tools.iter().map(|t| format!("mcp__{MCP_NAME}__{t}")).collect::<Vec<_>>().join(",")
                };
                (s(&["-p", "--allowedTools", &allowed, "--output-format", "text"]), Some(prompt.to_string()))
            }
            // `-` reads the whole prompt from stdin; the default sandbox stays read-only.
            Platform::Codex => (s(&["exec", "--skip-git-repo-check", "-"]), Some(prompt.to_string())),
            Platform::Cursor => (s(&["-p", &as_arg(), "--approve-mcps", "--output-format", "text"]), None),
            Platform::OpenCode => (s(&["run", &as_arg()]), None),
            Platform::Antigravity => (s(&["-p", &as_arg(), "--output-format", "text"]), None),
        };
        Some(Invocation { program, args, stdin })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_the_managed_block() {
        let text = format!("model = \"x\"\n\n{BEGIN}\n[mcp_servers.crewconvo]\nurl = \"u\"\n{END}\n[other]\na = 1\n");
        assert_eq!(strip_block(&text), "model = \"x\"\n\n[other]\na = 1\n");
        assert_eq!(strip_block("a = 1\n"), "a = 1\n");
    }

    #[test]
    fn reads_json_with_comments_and_trailing_commas() {
        let v = relaxed_json("{\n  // note\n  \"a\": [1, 2,],\n}").unwrap();
        assert_eq!(v, json!({"a": [1, 2]}));
    }

    #[test]
    fn json_edits_keep_other_settings() {
        let dir = std::env::temp_dir().join(format!("cc-platforms-{}", std::process::id()));
        let path = dir.join("mcp.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "{\"mcpServers\":{\"other\":{\"url\":\"x\"}},\"keep\":true}").unwrap();
        assert!(json_set(&path, &["mcpServers", MCP_NAME], json!({"url": "u"})));
        assert!(json_allow(&path, &["permissions", "allow"], "rule"));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["other"]["url"], "x");
        assert_eq!(v["mcpServers"][MCP_NAME]["url"], "u");
        assert_eq!(v["permissions"]["allow"], json!(["rule"]));
        json_del(&path, &["mcpServers", MCP_NAME]);
        json_disallow(&path, &["permissions", "allow"], "rule");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["keep"], true);
        assert!(v["mcpServers"].get(MCP_NAME).is_none());
        assert_eq!(v["permissions"]["allow"], json!([]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
