//! Which folder on this computer a conversation's coding agents work in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A project is a folder. Conversations bound to the same folder are the same project; there
/// is no list of projects besides that. `names` only holds the names the user chose — a
/// project without one is called after its folder.
#[derive(Default, Clone, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    projects: HashMap<Uuid, String>,
    #[serde(default)]
    names: HashMap<String, String>,
}

pub struct Projects {
    file: PathBuf,
    state: Mutex<Saved>,
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// What a project is called when the user gave it no name: the last part of its path.
pub fn default_name(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or_default();
    if last.is_empty() { trimmed.to_string() } else { last.to_string() }
}

/// A channel name made from free text: channel names cannot contain spaces, `@` or `#`.
pub fn channel_name(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        let c = if c.is_whitespace() || c == '@' || c == '#' { '-' } else { c };
        // No runs of dashes where several characters were replaced.
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    let out: String = out.trim_matches('-').chars().take(60).collect();
    if out.is_empty() { "project".to_string() } else { out }
}

/// The first of `base`, `base-2`, `base-3`… that is not taken.
pub fn free_name(base: &str, taken: &[String]) -> String {
    let is_taken = |n: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(n));
    if !is_taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base}-{i}")).find(|n| !is_taken(n)).unwrap_or_else(|| base.to_string())
}

/// The user's catch-all folders themselves (not folders inside them): everything they ever
/// downloaded or saved is in there, which is not a project.
pub fn catch_all(path: &Path, home: &Path) -> bool {
    ["Downloads", "Desktop", "Documents", "Pictures", "Music", "Videos", "OneDrive"]
        .iter()
        .any(|name| same_dir(path, &home.join(name)))
}

/// Things worth knowing before agents are let into a folder. They do not stop anything.
pub fn warnings(path: &Path, home: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if catch_all(path, home) {
        out.push(format!(
            "「{}」是放各種東西的資料夾，不是單一專案：Coding Agent 會看到（有修改權限時也能改）裡面所有的檔案。建議改選專案自己的子資料夾。",
            default_name(&path.to_string_lossy())
        ));
    }
    if !path.join(".git").exists() {
        out.push(
            "這個資料夾不是 git 專案：Coding Agent 會直接修改裡面的檔案，沒有各自的工作副本，也無法列出改了什麼或捨棄。建議先在資料夾裡執行 git init 並提交一次。"
                .into(),
        );
    }
    out
}

/// Whether a conversation is listed under "Coding Agent" in the sidebar instead of among the
/// ordinary channels and direct messages: it has a project folder, or it is a channel with a
/// coding agent of this computer in it. The everyone-channel `general` stays where it is, and
/// so does a direct message without a folder.
pub fn in_coding_section(is_channel: bool, name: Option<&str>, has_local_agent: bool, bound: bool) -> bool {
    bound || (is_channel && has_local_agent && name != Some("general"))
}

/// Checks a folder typed by the user. Agents may get write access inside it, so the whole
/// disk and the whole home folder (with its keys and settings) are refused.
pub fn check_path(raw: &str, home: &Path) -> Result<String, String> {
    let text = raw.trim().trim_matches('"');
    if text.is_empty() {
        return Err("請填寫資料夾的路徑".into());
    }
    let path = Path::new(text);
    if !path.is_absolute() {
        return Err("請填寫完整路徑（例如 C:\\Users\\me\\projects\\shop）".into());
    }
    if !path.is_dir() {
        return Err(format!("找不到這個資料夾：{text}"));
    }
    // `C:\` and `/` have no parent; `C:\x\..` is caught by comparing the real locations.
    let real = path.canonicalize().map_err(|e| format!("無法讀取這個資料夾：{e}"))?;
    if real.parent().is_none() {
        return Err("不能把整個磁碟當成專案資料夾，請選一個專案自己的資料夾".into());
    }
    if same_dir(path, home) {
        return Err("不能把整個使用者資料夾當成專案資料夾（裡面有金鑰和各種設定），請選一個專案自己的資料夾".into());
    }
    // Stored as typed, without a trailing separator: canonical Windows paths (`\\?\C:\…`)
    // confuse the tools that are started in this folder.
    let clean = text.trim_end_matches(['/', '\\']);
    Ok(if clean.is_empty() || clean.ends_with(':') { text.to_string() } else { clean.to_string() })
}

impl Projects {
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("local-projects.json");
        let saved: Saved =
            std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Projects { file, state: Mutex::new(saved) }
    }

    fn save(&self, state: &Saved) {
        let text = serde_json::to_string_pretty(state).unwrap_or_default();
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&self.file, text) {
            tracing::warn!(error = %e, file = %self.file.display(), "could not save local projects");
        }
    }

    pub fn get(&self, conversation_id: Uuid) -> Option<PathBuf> {
        self.state.lock().unwrap().projects.get(&conversation_id).map(PathBuf::from)
    }

    pub fn all(&self) -> HashMap<Uuid, String> {
        self.state.lock().unwrap().projects.clone()
    }

    /// The way a folder is already written by another conversation of the same project
    /// (`c:\Proj\` and `C:\proj` are one folder, and must be one group), else as given.
    pub fn spelling(&self, path: &str) -> String {
        let state = self.state.lock().unwrap();
        let known = state.projects.values().find(|p| p.as_str() == path || same_dir(Path::new(p), Path::new(path)));
        known.cloned().unwrap_or_else(|| path.to_string())
    }

    pub fn set(&self, conversation_id: Uuid, path: String) {
        let mut state = self.state.lock().unwrap();
        state.projects.insert(conversation_id, path);
        self.save(&state);
    }

    pub fn clear(&self, conversation_id: Uuid) -> bool {
        let mut state = self.state.lock().unwrap();
        let removed = state.projects.remove(&conversation_id).is_some();
        if removed {
            // A name belongs to a folder that is somebody's project; with the last
            // conversation gone there is no project left to call anything.
            let used: Vec<String> = state.projects.values().cloned().collect();
            state.names.retain(|path, _| used.contains(path));
            self.save(&state);
        }
        removed
    }

    /// What the project in this folder is called.
    pub fn name_of(&self, path: &str) -> String {
        let state = self.state.lock().unwrap();
        state.names.get(path).cloned().unwrap_or_else(|| default_name(path))
    }

    /// Names the project in a folder; an empty name goes back to the folder's own name.
    pub fn set_name(&self, path: &str, name: &str) {
        let mut state = self.state.lock().unwrap();
        let name = name.trim();
        if name.is_empty() || name == default_name(path) {
            state.names.remove(path);
        } else {
            state.names.insert(path.to_string(), name.chars().take(80).collect());
        }
        self.save(&state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_the_disk_root_the_home_folder_and_things_that_are_not_folders() {
        let home = std::env::temp_dir().join(format!("cc-projects-home-{}", std::process::id()));
        let project = home.join("code").join("shop");
        std::fs::create_dir_all(&project).unwrap();
        let p = project.to_string_lossy().to_string();
        assert_eq!(check_path(&format!("  {p}{}  ", std::path::MAIN_SEPARATOR), &home).unwrap(), p);
        assert!(check_path(&home.to_string_lossy(), &home).is_err());
        // The home folder written in a roundabout way is still the home folder.
        assert!(check_path(&project.join("..").join("..").to_string_lossy(), &home).is_err());
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        assert!(check_path(root, &home).is_err());
        assert!(check_path("relative/folder", &home).is_err());
        assert!(check_path(&home.join("nope").to_string_lossy(), &home).is_err());
        assert!(check_path("  ", &home).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn remembers_one_folder_per_conversation() {
        let dir = std::env::temp_dir().join(format!("cc-projects-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let p = Projects::load(&dir);
        p.set(a, "C:\\one".into());
        p.set(a, "C:\\two".into());
        assert_eq!(Projects::load(&dir).get(a), Some(PathBuf::from("C:\\two")));
        assert_eq!(p.get(b), None);
        assert!(p.clear(a) && !p.clear(a));
        assert!(Projects::load(&dir).all().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn conversations_bound_to_one_folder_are_one_project() {
        let root = std::env::temp_dir().join(format!("cc-projects-group-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join("Shop App");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.to_string_lossy().to_string();
        let p = Projects::load(&root);
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        p.set(a, path.clone());
        // The same folder typed differently is written the way the project already has it.
        let typed = format!("{path}{}", std::path::MAIN_SEPARATOR);
        assert_eq!(p.spelling(&typed), path);
        p.set(b, p.spelling(&typed));
        assert_eq!(p.all().values().collect::<std::collections::HashSet<_>>().len(), 1);
        assert_eq!(p.spelling("/somewhere/else"), "/somewhere/else");

        // Called after its folder until the user names it; the name is the project's, so
        // both conversations see it, and it survives a restart.
        assert_eq!(p.name_of(&path), "Shop App");
        p.set_name(&path, "  網路商店  ");
        assert_eq!(Projects::load(&root).name_of(&path), "網路商店");
        p.set_name(&path, "");
        assert_eq!(p.name_of(&path), "Shop App");
        // The name goes with the last conversation of the project.
        p.set_name(&path, "商店");
        p.clear(a);
        assert_eq!(p.name_of(&path), "商店");
        p.clear(b);
        assert_eq!(p.name_of(&path), "Shop App");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn default_names() {
        assert_eq!(default_name("C:\\Users\\me\\projects\\shop"), "shop");
        assert_eq!(default_name("/home/me/code/api/"), "api");
        assert_eq!(default_name("D:\\"), "D:");
        assert_eq!(channel_name("  Shop App #2 @home "), "Shop-App-2-home");
        assert_eq!(channel_name("網路商店"), "網路商店");
        assert_eq!(channel_name(" @# "), "project");
        let taken = vec!["shop".to_string(), "Shop-2".to_string()];
        assert_eq!(free_name("shop", &taken), "shop-3");
        assert_eq!(free_name("api", &taken), "api");
    }

    #[test]
    fn warns_about_catch_all_folders_and_folders_without_git() {
        let home = std::env::temp_dir().join(format!("cc-projects-warn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let downloads = home.join("Downloads");
        let project = downloads.join("shop");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        // Downloads itself: a catch-all, and not a repository.
        assert!(catch_all(&downloads, &home));
        let w = warnings(&downloads, &home);
        assert_eq!(w.len(), 2);
        assert!(w[0].contains("Downloads") && w[1].contains("不是 git 專案"));
        // A project inside Downloads is a project like any other.
        assert!(!catch_all(&project, &home));
        assert!(warnings(&project, &home).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn which_conversations_are_listed_under_coding_agent() {
        // Bound to a folder: always, also a direct message.
        assert!(in_coding_section(true, Some("shop"), true, true));
        assert!(in_coding_section(false, None, true, true));
        assert!(in_coding_section(true, Some("general"), false, true));
        // A channel with a coding agent of this computer and no folder: "no project yet".
        assert!(in_coding_section(true, Some("coding-agents"), true, false));
        // Not the everyone-channel, not a direct message, not a channel without such an agent.
        assert!(!in_coding_section(true, Some("general"), true, false));
        assert!(!in_coding_section(false, None, true, false));
        assert!(!in_coding_section(true, Some("random"), false, false));
    }
}
