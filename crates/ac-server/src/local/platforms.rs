//! Coding agents on this computer: detection, MCP + Skill setup, headless runs.
//!
//! The same registry as the `crewconvo` command (clients/python/crewconvo/platforms.py), so the
//! server itself can set them up when it runs on the user's own computer. File locations and
//! flags come from each tool's official documentation.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::access::Access;

pub const MCP_NAME: &str = "crewconvo";
const SKILL: &str = include_str!("../../../../clients/python/crewconvo/SKILL.md");
const BEGIN: &str = "# >>> crewconvo (managed by `crewconvo setup`; edits here are overwritten) >>>";
const END: &str = "# <<< crewconvo <<<";

/// What one wake runs: program, arguments, text for its standard input.
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub stdin: Option<String>,
    /// Standard output is a stream of JSON events (Codex `--json`), not the agent's final text.
    pub json_events: bool,
    /// The tool stays in a conversation with us for the whole run, so messages that arrive
    /// while it works can be handed to it. None: it gets the prompt, runs, and exits.
    pub live: Option<Live>,
}

/// How to talk to a tool that takes more input while it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Live {
    /// Claude Code `--input-format stream-json`: each user message is one JSON line on stdin.
    ClaudeStream,
    /// `codex app-server`: JSON-RPC on stdin/stdout (`thread/start`, `turn/start`, `turn/steer`).
    CodexServer { sandbox: &'static str, resume: Option<String> },
}

/// Settings of this server that change how the tools are started.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Use the live protocols where there is one (`LOCAL_LIVE_MESSAGES`, default on).
    pub live: bool,
    /// `[windows] sandbox` to run Codex with instead of what its own config says
    /// (`LOCAL_CODEX_WINDOWS_SANDBOX`); None leaves Codex's choice alone.
    pub codex_windows_sandbox: Option<String>,
}

/// Which session of the tool a wake runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionArg<'a> {
    /// A new session nobody will continue (the tool has no way to, or reuse is turned off).
    Unrecorded,
    /// A new session to be recorded. Claude Code takes the id we choose; Codex makes its own
    /// and reports it in its event stream.
    New(&'a str),
    /// Continue this session.
    Resume(&'a str),
}

/// What a Codex `--json` run printed: the session it ran in and the agent's last message.
#[derive(Debug, Default, PartialEq)]
pub struct CodexEvents {
    pub thread_id: Option<String>,
    pub last_message: Option<String>,
    pub errors: Vec<String>,
}

/// Reads `codex exec --json` output: `thread.started` carries the session id, each
/// `item.completed` of type `agent_message` a message of the agent. Other lines are ignored.
pub fn codex_events(stdout: &str) -> CodexEvents {
    let mut out = CodexEvents::default();
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        match v["type"].as_str().unwrap_or_default() {
            "thread.started" => {
                if let Some(id) = v["thread_id"].as_str() {
                    out.thread_id = Some(id.to_string());
                }
            }
            "item.completed" if v["item"]["type"] == "agent_message" => {
                if let Some(text) = v["item"]["text"].as_str() {
                    out.last_message = Some(text.to_string());
                }
            }
            "error" | "turn.failed" => {
                let msg = v["message"].as_str().or_else(|| v["error"]["message"].as_str());
                if let Some(m) = msg {
                    out.errors.push(m.to_string());
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether a failed run failed because the session it was asked to continue no longer exists
/// (the user deleted it, or the tool cleaned it up). Claude Code prints
/// "No conversation found with session ID: <id>", Codex "no rollout found for thread id <id>".
pub fn session_missing(output: &str) -> bool {
    let t = output.to_lowercase();
    let about = ["session", "conversation", "thread", "rollout"].iter().any(|w| t.contains(w));
    let gone = ["not found", "no conversation found", "no rollout found", "no session found", "does not exist"]
        .iter()
        .any(|w| t.contains(w));
    about && gone
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

/// The Claude Code command-line program: `claude` on PATH, or the copy that ships inside the
/// Claude desktop app on Windows (`%LOCALAPPDATA%\Claude-Data\<id>\claude-code\<version>\<build>\claude.exe`).
pub fn claude_exe() -> Option<PathBuf> {
    if let Some(p) = which(&["claude"]) {
        return Some(p);
    }
    if !cfg!(windows) {
        return None;
    }
    let subdirs = |dir: PathBuf| -> Vec<PathBuf> {
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()
    };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for profile in subdirs(env_path("LOCALAPPDATA")?.join("Claude-Data")) {
        for version in subdirs(profile.join("claude-code")) {
            for build in subdirs(version) {
                let exe = build.join("claude.exe");
                if let Ok(modified) = exe.metadata().and_then(|m| m.modified()) {
                    found.push((modified, exe));
                }
            }
        }
    }
    // Several versions can be present; the newest one is the one the app uses.
    found.into_iter().max_by_key(|(t, _)| *t).map(|(_, p)| p)
}

fn child_dirs(dir: PathBuf) -> Vec<PathBuf> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()
}

fn newest(files: Vec<PathBuf>) -> Option<PathBuf> {
    files
        .into_iter()
        .filter_map(|p| p.metadata().and_then(|m| m.modified()).ok().map(|t| (t, p)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

/// The Codex command-line program: `codex` on PATH, or the copy inside the Codex desktop app
/// (`%LOCALAPPDATA%\OpenAI\Codex\bin\<build>\codex.exe`) or the Codex extension for VS Code.
pub fn codex_exe() -> Option<PathBuf> {
    if let Some(p) = which(&["codex"]) {
        return Some(p);
    }
    if !cfg!(windows) {
        return None;
    }
    let mut files = Vec::new();
    if let Some(local) = env_path("LOCALAPPDATA") {
        for build in child_dirs(local.join("OpenAI").join("Codex").join("bin")) {
            files.push(build.join("codex.exe"));
        }
    }
    if let Some(found) = newest(files) {
        return Some(found);
    }
    let mut files = Vec::new();
    for ext in child_dirs(home().join(".vscode").join("extensions")) {
        let is_codex = ext.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("openai.chatgpt-"));
        if is_codex {
            files.push(ext.join("bin").join("windows-x86_64").join("codex.exe"));
        }
    }
    newest(files)
}

/// Whether a coding agent can actually be woken, not just whether it is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    /// Command-line program found and signed in.
    Ready,
    /// Command-line program found, not signed in.
    NeedsLogin,
    /// The app or its settings are there, but no command-line program: usable from inside
    /// the app, cannot be woken.
    NoCli,
    /// Command-line program found; there is no checked way to tell whether it is signed in.
    Unknown,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Status {
    pub state: Readiness,
    pub explanation: String,
    /// For `needs_login`: what to run in a terminal.
    pub login_command: Option<String>,
}

/// `claude auth status` prints JSON with `loggedIn` (exit code 0 signed in, 1 not).
pub fn claude_status(stdout: &str) -> Readiness {
    match serde_json::from_str::<Value>(stdout.trim()).ok().and_then(|v| v.get("loggedIn")?.as_bool()) {
        Some(true) => Readiness::Ready,
        Some(false) => Readiness::NeedsLogin,
        None => Readiness::Unknown,
    }
}

/// `codex login status` prints "Logged in using …" (exit code 0) or "Not logged in" (exit code 1).
pub fn codex_status(output: &str, success: bool) -> Readiness {
    let t = output.to_lowercase();
    if t.contains("not logged in") {
        Readiness::NeedsLogin
    } else if success && t.contains("logged in") {
        Readiness::Ready
    } else {
        // Something else went wrong (a broken config, a changed message): do not guess.
        Readiness::Unknown
    }
}

/// Runs a status command for at most 8 seconds: (exit code was 0, stdout, stderr).
fn probe(program: &Path, args: &[&str]) -> Option<(bool, String, String)> {
    let mut child = quiet_command(program).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > Duration::from_secs(8) => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
    let out = child.wait_with_output().ok()?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    Some((out.status.success(), text(&out.stdout), text(&out.stderr)))
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

    /// Checks now whether this one can be woken. Starts the tool's own status command (never
    /// a model request); takes up to 8 seconds, so callers keep the answer for a while.
    pub fn status(self) -> Status {
        let Some(exe) = self.runner() else {
            let extra = self.wake_note();
            return Status {
                state: Readiness::NoCli,
                explanation: format!(
                    "找不到它的指令列工具：可以在它裡面主動使用 CrewConvo，但被 @ 時無法自動回覆。{extra}"
                ),
                login_command: None,
            };
        };
        let (state, login) = match self {
            Platform::ClaudeCode => {
                let state = probe(&exe, &["auth", "status"]).map(|(_, out, _)| claude_status(&out));
                (state.unwrap_or(Readiness::Unknown), "auth login")
            }
            Platform::Codex => {
                let state = probe(&exe, &["login", "status"])
                    .map(|(ok, out, err)| codex_status(&format!("{out}\n{err}"), ok));
                (state.unwrap_or(Readiness::Unknown), "login")
            }
            // No status command of these has been checked against the real tool.
            Platform::Cursor | Platform::OpenCode | Platform::Antigravity => (Readiness::Unknown, ""),
        };
        let name = self.name();
        match state {
            Readiness::Ready => Status {
                state,
                explanation: "指令列工具已登入，被 @ 時會在這台電腦上被叫起來回覆。".into(),
                login_command: None,
            },
            Readiness::NeedsLogin => Status {
                state,
                explanation: format!(
                    "指令列版的 {name} 還沒有登入，所以被 @ 時無法回覆。在終端機執行下面的指令登入一次，再按「重新檢查」。"
                ),
                login_command: Some(format!("\"{}\" {login}", exe.display())),
            },
            _ => {
                let extra = self.wake_note();
                Status {
                    state: Readiness::Unknown,
                    explanation: format!("找到指令列工具，但無法確認它是否已登入；沒有登入的話被 @ 時不會回覆。{extra}"),
                    login_command: None,
                }
            }
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
            Platform::ClaudeCode => claude_exe().is_some(),
            Platform::Codex => codex_exe().is_some() || Self::codex_dir().exists(),
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
            Platform::ClaudeCode => claude_exe(),
            Platform::Codex => codex_exe(),
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
                let Some(exe) = claude_exe() else {
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
                if let Some(exe) = claude_exe() {
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

    /// Whether a later wake can continue a session of this tool. Only the tools whose flags
    /// for it have been checked; the others always start a new session.
    pub fn keeps_sessions(self) -> bool {
        matches!(self, Platform::ClaudeCode | Platform::Codex)
    }

    /// Whether what this tool may do to files can be limited with flags that have been checked.
    /// The others run as they always did; only the working directory applies to them.
    pub fn limits_access(self) -> bool {
        matches!(self, Platform::ClaudeCode | Platform::Codex)
    }

    /// How to run one wake. `tools` are the MCP tool names this agent may use; `access` is
    /// what it may do to the files in its working directory.
    pub fn invocation(
        self,
        prompt: &str,
        tools: &[String],
        session: SessionArg<'_>,
        access: Access,
        opts: &RunOptions,
    ) -> Option<Invocation> {
        let program = self.runner()?;
        let mut inv = self.invocation_with(program, prompt, tools, session, access);
        if opts.live {
            self.make_live(&mut inv, session, access);
        }
        if self == Platform::Codex
            && let Some(mode) = &opts.codex_windows_sandbox
        {
            // Right after the subcommand (`exec` / `app-server`), where both take `-c`.
            inv.args.splice(1..1, ["-c".to_string(), format!("windows.sandbox={mode}")]);
        }
        Some(inv)
    }

    /// Switches an invocation to the protocol that keeps the tool listening while it works.
    /// Does nothing for tools that have none.
    fn make_live(self, inv: &mut Invocation, session: SessionArg<'_>, access: Access) {
        match self {
            Platform::ClaudeCode => {
                // `--verbose` is required by Claude Code for stream-json output with `-p`.
                let Some(i) = inv.args.iter().position(|a| a == "--output-format") else { return };
                inv.args.splice(
                    i..i + 2,
                    ["--input-format", "stream-json", "--output-format", "stream-json", "--verbose"].map(String::from),
                );
                inv.live = Some(Live::ClaudeStream);
            }
            Platform::Codex => {
                // `codex exec` cannot be told anything once it runs; the app server can. The
                // sandbox and the session go into its requests instead of the command line.
                let mut args = vec!["app-server".to_string()];
                if access == Access::Full {
                    args.extend(["-c", "sandbox_workspace_write.network_access=true"].map(String::from));
                }
                inv.args = args;
                inv.json_events = false;
                inv.live = Some(Live::CodexServer {
                    sandbox: if access.writes() { "workspace-write" } else { "read-only" },
                    resume: match session {
                        SessionArg::Resume(id) => Some(id.to_string()),
                        _ => None,
                    },
                });
            }
            _ => {}
        }
    }

    fn invocation_with(
        self,
        program: PathBuf,
        prompt: &str,
        tools: &[String],
        session: SessionArg<'_>,
        access: Access,
    ) -> Invocation {
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
                let mut allowed = if tools.is_empty() {
                    format!("mcp__{MCP_NAME}")
                } else {
                    tools.iter().map(|t| format!("mcp__{MCP_NAME}__{t}")).collect::<Vec<_>>().join(",")
                };
                // Without a terminal, a tool that is not allowed is refused instead of asked about.
                allowed.push_str(",Read,Glob,Grep");
                if access == Access::Full {
                    allowed.push_str(",Bash");
                }
                let mut args = s(&["-p", "--allowedTools", &allowed, "--output-format", "text"]);
                // Edits are not put on the allow list: `acceptEdits` accepts them inside the
                // working directory only, an allowed `Edit` would accept them anywhere.
                if access.writes() {
                    args.extend(s(&["--permission-mode", "acceptEdits"]));
                }
                // Denied outright, so nothing in the user's own Claude Code settings can allow it.
                let denied = match access {
                    Access::Read => "Edit,Write,MultiEdit,NotebookEdit,Bash",
                    Access::Edit => "Bash",
                    Access::Full => "",
                };
                if !denied.is_empty() {
                    args.extend(s(&["--disallowedTools", denied]));
                }
                match session {
                    SessionArg::Unrecorded => {}
                    SessionArg::New(id) => args.extend(s(&["--session-id", id])),
                    SessionArg::Resume(id) => args.extend(s(&["--resume", id])),
                }
                (args, Some(prompt.to_string()))
            }
            // `-` reads the whole prompt from stdin.
            Platform::Codex => {
                // The sandbox is Codex's own: read-only cannot write anywhere; workspace-write
                // can write in the working directory and run commands there, without network
                // unless that is switched on. It goes before `resume`, which does not take `-s`.
                let mut args = s(&["exec", "-s", if access.writes() { "workspace-write" } else { "read-only" }]);
                if access == Access::Full {
                    args.extend(s(&["-c", "sandbox_workspace_write.network_access=true"]));
                }
                args.extend(match session {
                    SessionArg::Unrecorded => s(&["--skip-git-repo-check", "-"]),
                    // `--json` is the only way to learn the id of the session Codex creates.
                    SessionArg::New(_) => s(&["--skip-git-repo-check", "--json", "-"]),
                    SessionArg::Resume(id) => s(&["resume", "--skip-git-repo-check", "--json", id, "-"]),
                });
                (args, Some(prompt.to_string()))
            }
            Platform::Cursor => (s(&["-p", &as_arg(), "--approve-mcps", "--output-format", "text"]), None),
            Platform::OpenCode => (s(&["run", &as_arg()]), None),
            Platform::Antigravity => (s(&["-p", &as_arg(), "--output-format", "text"]), None),
        };
        let json_events = self == Platform::Codex && session != SessionArg::Unrecorded;
        Invocation { program, args, stdin, json_events, live: None }
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
    fn reads_the_session_and_last_message_from_codex_events() {
        let lines = [
            "warning line",
            r#"{"type":"thread.started","thread_id":"01a1"}"#,
            r#"{"type":"turn.started"}"#,
            r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"first"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","text":"x"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"done"}}"#,
            r#"{"type":"turn.completed","usage":{}}"#,
        ];
        let ev = codex_events(&lines.join("\n"));
        assert_eq!(ev.thread_id.as_deref(), Some("01a1"));
        assert_eq!(ev.last_message.as_deref(), Some("done"));
        assert!(ev.errors.is_empty());
        assert_eq!(codex_events("plain text"), CodexEvents::default());
    }

    fn args(p: Platform, session: SessionArg<'_>, access: Access) -> Vec<String> {
        let tools = vec!["post_message".to_string(), "react".to_string()];
        p.invocation_with(PathBuf::from("tool"), "prompt", &tools, session, access).args
    }

    #[test]
    fn live_protocols_keep_the_access_flags_and_the_session() {
        let tools = vec!["post_message".to_string()];
        let mut c = Platform::ClaudeCode.invocation_with(
            PathBuf::from("claude"),
            "p",
            &tools,
            SessionArg::Resume("s-1"),
            Access::Edit,
        );
        Platform::ClaudeCode.make_live(&mut c, SessionArg::Resume("s-1"), Access::Edit);
        assert_eq!(c.live, Some(Live::ClaudeStream));
        assert_eq!(value_of(&c.args, "--input-format"), Some("stream-json"));
        assert_eq!(value_of(&c.args, "--output-format"), Some("stream-json"));
        assert!(c.args.contains(&"--verbose".to_string()) && !c.args.contains(&"text".to_string()));
        assert_eq!(value_of(&c.args, "--permission-mode"), Some("acceptEdits"));
        assert_eq!(value_of(&c.args, "--disallowedTools"), Some("Bash"));
        assert_eq!(value_of(&c.args, "--resume"), Some("s-1"));

        let mut x =
            Platform::Codex.invocation_with(PathBuf::from("codex"), "p", &tools, SessionArg::Resume("t-1"), Access::Full);
        Platform::Codex.make_live(&mut x, SessionArg::Resume("t-1"), Access::Full);
        assert_eq!(x.args, ["app-server", "-c", "sandbox_workspace_write.network_access=true"]);
        assert_eq!(x.live, Some(Live::CodexServer { sandbox: "workspace-write", resume: Some("t-1".into()) }));
        assert!(!x.json_events && x.stdin.as_deref() == Some("p"));

        let mut r = Platform::Codex.invocation_with(PathBuf::from("codex"), "p", &tools, SessionArg::New("n"), Access::Read);
        Platform::Codex.make_live(&mut r, SessionArg::New("n"), Access::Read);
        assert_eq!(r.args, ["app-server"]);
        assert_eq!(r.live, Some(Live::CodexServer { sandbox: "read-only", resume: None }));

        // Tools without such a protocol run as before.
        let mut o = Platform::OpenCode.invocation_with(PathBuf::from("o"), "p", &tools, SessionArg::Unrecorded, Access::Read);
        let before = o.args.clone();
        Platform::OpenCode.make_live(&mut o, SessionArg::Unrecorded, Access::Read);
        assert!(o.live.is_none() && o.args == before);
    }

    fn value_of<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
    }

    #[test]
    fn claude_code_gets_exactly_the_tools_of_its_level() {
        let mcp = "mcp__crewconvo__post_message,mcp__crewconvo__react";
        let read = args(Platform::ClaudeCode, SessionArg::New("id-1"), Access::Read);
        assert_eq!(value_of(&read, "--allowedTools"), Some(format!("{mcp},Read,Glob,Grep").as_str()));
        assert_eq!(value_of(&read, "--disallowedTools"), Some("Edit,Write,MultiEdit,NotebookEdit,Bash"));
        assert_eq!(value_of(&read, "--permission-mode"), None);
        assert_eq!(value_of(&read, "--session-id"), Some("id-1"));

        let edit = args(Platform::ClaudeCode, SessionArg::Resume("id-1"), Access::Edit);
        assert_eq!(value_of(&edit, "--allowedTools"), Some(format!("{mcp},Read,Glob,Grep").as_str()));
        assert_eq!(value_of(&edit, "--permission-mode"), Some("acceptEdits"));
        assert_eq!(value_of(&edit, "--disallowedTools"), Some("Bash"));
        assert_eq!(value_of(&edit, "--resume"), Some("id-1"));

        let full = args(Platform::ClaudeCode, SessionArg::Unrecorded, Access::Full);
        assert_eq!(value_of(&full, "--allowedTools"), Some(format!("{mcp},Read,Glob,Grep,Bash").as_str()));
        assert_eq!(value_of(&full, "--permission-mode"), Some("acceptEdits"));
        assert_eq!(value_of(&full, "--disallowedTools"), None);
        for a in [&read, &edit, &full] {
            assert!(!a.iter().any(|x| x.contains("dangerously") || x == "bypassPermissions"));
        }
    }

    #[test]
    fn codex_gets_the_sandbox_of_its_level() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            args(Platform::Codex, SessionArg::Unrecorded, Access::Read),
            s(&["exec", "-s", "read-only", "--skip-git-repo-check", "-"])
        );
        assert_eq!(
            args(Platform::Codex, SessionArg::New("x"), Access::Edit),
            s(&["exec", "-s", "workspace-write", "--skip-git-repo-check", "--json", "-"])
        );
        // `resume` takes no `-s`: the sandbox has to come before it.
        assert_eq!(
            args(Platform::Codex, SessionArg::Resume("abc"), Access::Full),
            s(&[
                "exec",
                "-s",
                "workspace-write",
                "-c",
                "sandbox_workspace_write.network_access=true",
                "resume",
                "--skip-git-repo-check",
                "--json",
                "abc",
                "-"
            ])
        );
        for access in [Access::Read, Access::Edit, Access::Full] {
            let a = args(Platform::Codex, SessionArg::Resume("abc"), access);
            assert!(!a.iter().any(|x| x.contains("danger")));
        }
    }

    #[test]
    fn the_other_tools_run_as_before_whatever_the_level() {
        for p in [Platform::Cursor, Platform::OpenCode, Platform::Antigravity] {
            assert!(!p.limits_access());
            assert_eq!(args(p, SessionArg::Unrecorded, Access::Read), args(p, SessionArg::Unrecorded, Access::Full));
        }
        assert!(Platform::ClaudeCode.limits_access() && Platform::Codex.limits_access());
    }

    #[test]
    fn reads_the_login_state_each_tool_prints() {
        assert_eq!(claude_status("{\n  \"loggedIn\": true,\n  \"authMethod\": \"claude.ai\"\n}"), Readiness::Ready);
        assert_eq!(claude_status("{\"loggedIn\": false, \"authMethod\": \"none\"}"), Readiness::NeedsLogin);
        assert_eq!(claude_status("Usage: claude auth"), Readiness::Unknown);
        assert_eq!(claude_status(""), Readiness::Unknown);
        assert_eq!(codex_status("\nLogged in using ChatGPT\n", true), Readiness::Ready);
        assert_eq!(codex_status("Logged in using an API key - sk-***", true), Readiness::Ready);
        assert_eq!(codex_status("Not logged in\n", false), Readiness::NeedsLogin);
        assert_eq!(codex_status("Error loading config.toml: bad", false), Readiness::Unknown);
        assert_eq!(codex_status("", true), Readiness::Unknown);
    }

    #[test]
    fn recognises_a_session_that_no_longer_exists() {
        assert!(session_missing("No conversation found with session ID: 019a0000-0000-7000-8000-000000000000"));
        assert!(session_missing(
            "Error: thread/resume: thread/resume failed: no rollout found for thread id 019a (code -32600)"
        ));
        assert!(!session_missing("Error: rate limit reached"));
        assert!(!session_missing("command not found: rg"));
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
