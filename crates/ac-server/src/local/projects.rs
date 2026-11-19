//! Which folder on this computer a conversation's coding agents work in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    projects: HashMap<Uuid, String>,
}

pub struct Projects {
    file: PathBuf,
    map: Mutex<HashMap<Uuid, String>>,
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
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
        Projects { file, map: Mutex::new(saved.projects) }
    }

    fn save(&self, map: &HashMap<Uuid, String>) {
        let text = serde_json::to_string_pretty(&Saved { projects: map.clone() }).unwrap_or_default();
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&self.file, text) {
            tracing::warn!(error = %e, file = %self.file.display(), "could not save local projects");
        }
    }

    pub fn get(&self, conversation_id: Uuid) -> Option<PathBuf> {
        self.map.lock().unwrap().get(&conversation_id).map(PathBuf::from)
    }

    pub fn all(&self) -> HashMap<Uuid, String> {
        self.map.lock().unwrap().clone()
    }

    pub fn set(&self, conversation_id: Uuid, path: String) {
        let mut map = self.map.lock().unwrap();
        map.insert(conversation_id, path);
        self.save(&map);
    }

    pub fn clear(&self, conversation_id: Uuid) -> bool {
        let mut map = self.map.lock().unwrap();
        let removed = map.remove(&conversation_id).is_some();
        if removed {
            self.save(&map);
        }
        removed
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
}
