//! Updating the desktop app from the project's GitHub releases.
//!
//! The app asks GitHub which release is the latest and, when that is newer than the one it
//! was built from, offers it. Nothing is downloaded until the user says so. The download is
//! checked against the size and SHA-256 GitHub publishes for it, then the running program
//! file is renamed aside and the new one takes its place and name — wherever the user keeps
//! it and whatever they called it. The request carries nothing about the user or the computer.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// The release asset that is this program.
pub const ASSET: &str = "CrewConvo-windows-x86_64.exe";
const FEED: &str = "https://api.github.com/repos/SamChengYo/CrewConvo/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/SamChengYo/CrewConvo/releases/latest";
const USER_AGENT: &str = "crewconvo-updater";

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// The tag, as published (`v0.7.0`).
    pub version: String,
    pub notes_url: String,
    pub asset_url: String,
    pub size: u64,
    /// Lower-case hex; None when GitHub did not publish one for the asset.
    pub sha256: Option<String>,
}

/// The release this program was built from; None for a development build, which never
/// offers updates (it could not tell whether anything is newer).
pub fn current() -> Option<&'static str> {
    ac_server::version()
}

/// `UPDATE_CHECK=false` turns the check off.
pub fn check_enabled() -> bool {
    let off = std::env::var("UPDATE_CHECK")
        .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"));
    !off && current().is_some()
}

/// Where to ask, and whether that is the development override (`UPDATE_FEED_URL`), for which
/// the rule "GitHub over https only" does not apply.
fn feed() -> (String, bool) {
    match std::env::var("UPDATE_FEED_URL").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
        Some(url) => (url, true),
        None => (FEED.to_string(), false),
    }
}

/// `v0.6.0`, `0.6`, `v1.2.3-rc1` → the numbers. None when it does not start with one.
pub fn parse_version(tag: &str) -> Option<Vec<u64>> {
    let core = tag.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next().unwrap_or_default();
    let parts: Option<Vec<u64>> = core.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

/// Whether `candidate` is a later release than `current`. Anything unreadable is not newer:
/// an update is only offered when we are sure. There are no downgrades.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(mut a), Some(mut b)) => {
            let n = a.len().max(b.len());
            a.resize(n, 0);
            b.resize(n, 0);
            a > b
        }
        _ => false,
    }
}

/// Downloads come from GitHub over https and nowhere else, redirects included.
pub fn url_allowed(url: &str, development_feed: bool) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else { return false };
    if development_feed {
        return matches!(u.scheme(), "http" | "https");
    }
    let host = u.host_str().unwrap_or_default().to_ascii_lowercase();
    u.scheme() == "https" && (host == "github.com" || host == "api.github.com" || host.ends_with(".githubusercontent.com"))
}

/// Reads GitHub's "latest release" answer.
pub fn parse_release(v: &Value) -> Result<Release, String> {
    let version = v["tag_name"].as_str().filter(|t| !t.is_empty()).ok_or("沒有版本號")?.to_string();
    let assets = v["assets"].as_array().ok_or("沒有可下載的檔案")?;
    let asset = assets.iter().find(|a| a["name"] == ASSET).ok_or_else(|| format!("這個版本沒有 {ASSET}"))?;
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"))
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    Ok(Release {
        version,
        notes_url: v["html_url"].as_str().unwrap_or(RELEASES_PAGE).to_string(),
        asset_url: asset["browser_download_url"].as_str().ok_or("沒有下載網址")?.to_string(),
        size: asset["size"].as_u64().filter(|s| *s > 0).ok_or("沒有檔案大小")?,
        sha256,
    })
}

fn client(development_feed: bool) -> Result<reqwest::Client, String> {
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() > 5 || !url_allowed(attempt.url().as_str(), development_feed) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        // Says what this is, and nothing about who is asking.
        .user_agent(USER_AGENT)
        .redirect(policy)
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}

/// The latest release if it is newer than this program; None when this is the latest.
pub async fn check() -> Result<Option<Release>, String> {
    let Some(current) = current() else { return Ok(None) };
    let (url, dev) = feed();
    if !url_allowed(&url, dev) {
        return Err("更新來源的網址不被允許".into());
    }
    let resp = client(dev)?
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let release = parse_release(&resp.json::<Value>().await.map_err(|e| e.to_string())?)?;
    if !url_allowed(&release.asset_url, dev) {
        return Err("下載網址不在允許的來源內".into());
    }
    Ok(is_newer(&release.version, current).then_some(release))
}

fn with_suffix(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    exe.with_file_name(name)
}

/// Where the previous program file waits until the new one has started.
pub fn old_path(exe: &Path) -> PathBuf {
    with_suffix(exe, ".old")
}

fn new_path(exe: &Path) -> PathBuf {
    with_suffix(exe, ".new")
}

/// Whether the program's folder can be written to (it cannot when the app was put somewhere
/// like Program Files). The update needs to place a file beside the program.
pub fn can_replace(exe: &Path) -> bool {
    let probe = with_suffix(exe, ".probe");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Removes what an earlier update left beside the program.
pub fn clean_up(exe: &Path) {
    for leftover in [old_path(exe), new_path(exe)] {
        if leftover.exists()
            && let Err(e) = std::fs::remove_file(&leftover)
        {
            tracing::warn!(error = %e, file = %leftover.display(), "could not remove a leftover update file");
        }
    }
}

/// Checks a downloaded file against what GitHub says it should be.
pub fn verify(file: &Path, release: &Release) -> Result<(), String> {
    let size = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
    if size != release.size {
        return Err(format!("下載的檔案大小不符（應為 {} 位元組，實際 {size}）", release.size));
    }
    if let Some(expected) = &release.sha256 {
        use std::io::Read;
        let mut hasher = Sha256::new();
        let mut f = std::fs::File::open(file).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if &actual != expected {
            return Err("下載的檔案驗證失敗（SHA-256 不符），沒有安裝".into());
        }
    }
    Ok(())
}

async fn download(release: &Release, to: &Path) -> Result<(), String> {
    use std::io::Write;
    let (_, dev) = feed();
    if !url_allowed(&release.asset_url, dev) {
        return Err("下載網址不在允許的來源內".into());
    }
    let mut resp = client(dev)?
        .get(&release.asset_url)
        .timeout(Duration::from_secs(30 * 60))
        .send()
        .await
        .map_err(|e| format!("下載失敗：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下載失敗：HTTP {}", resp.status().as_u16()));
    }
    let mut file = std::fs::File::create(to).map_err(|e| format!("無法寫入 {}：{e}", to.display()))?;
    let mut got: u64 = 0;
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("下載中斷：{e}"))? {
        got += chunk.len() as u64;
        // Never fill the disk because a server sends more than it announced.
        if got > release.size {
            return Err("下載的檔案比公告的大，已中止".into());
        }
        file.write_all(&chunk).map_err(|e| format!("無法寫入 {}：{e}", to.display()))?;
    }
    file.sync_all().map_err(|e| e.to_string())
}

/// Puts `new` where `exe` is. The running program file cannot be overwritten, but it can be
/// renamed: it goes aside as `<name>.old`, and comes back if the second step fails.
pub fn swap(exe: &Path, new: &Path) -> Result<(), String> {
    let old = old_path(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| format!("無法移開目前的程式檔：{e}"))?;
    if let Err(e) = std::fs::rename(new, exe) {
        let back = std::fs::rename(&old, exe);
        return Err(match back {
            Ok(()) => format!("無法放上新的程式檔：{e}。已還原成原本的版本"),
            Err(b) => format!("無法放上新的程式檔：{e}；還原也失敗：{b}。原本的程式檔在 {}", old.display()),
        });
    }
    Ok(())
}

/// Undoes [`swap`]: the previous program file back under its name.
pub fn restore(exe: &Path) -> Result<(), String> {
    let old = old_path(exe);
    if !old.exists() {
        return Err("找不到原本的程式檔".into());
    }
    let _ = std::fs::remove_file(exe);
    std::fs::rename(&old, exe).map_err(|e| e.to_string())
}

/// Downloads the release, checks it, and puts it in place of the running program. On any
/// failure the program file is what it was before.
pub async fn install(release: &Release, exe: &Path) -> Result<(), String> {
    let new = new_path(exe);
    let result = async {
        download(release, &new).await?;
        verify(&new, release)?;
        swap(exe, &new)
    }
    .await;
    if result.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_release_numbers() {
        assert!(is_newer("v0.7.0", "v0.6.0"));
        assert!(is_newer("v0.10.0", "v0.9.9"));
        assert!(is_newer("1.0", "v0.99.99"));
        assert!(is_newer("v0.6.1", "0.6"));
        assert!(!is_newer("v0.6.0", "v0.6.0"));
        assert!(!is_newer("v0.5.9", "v0.6.0"));
        // Not sure → not newer.
        assert!(!is_newer("nightly", "v0.6.0"));
        assert!(!is_newer("v0.7.0", "dev"));
        assert_eq!(parse_version("v1.2.3-rc1"), Some(vec![1, 2, 3]));
        assert_eq!(parse_version(""), None);
    }

    fn github(digest: Value) -> Value {
        json!({
            "tag_name": "v0.7.0",
            "html_url": "https://github.com/SamChengYo/CrewConvo/releases/tag/v0.7.0",
            "assets": [
                { "name": "crewconvo-windows-x86_64.zip", "size": 5, "browser_download_url": "https://github.com/x.zip" },
                { "name": ASSET, "size": 43971072u64, "digest": digest,
                  "browser_download_url": "https://github.com/SamChengYo/CrewConvo/releases/download/v0.7.0/CrewConvo-windows-x86_64.exe" }
            ]
        })
    }

    #[test]
    fn reads_the_latest_release_as_github_answers() {
        let hash = "8e8e63c07673ebef3b60bb9c95c11f6504266db94a304a474763f1f0cabf5831";
        let r = parse_release(&github(json!(format!("sha256:{hash}")))).unwrap();
        assert_eq!(r.version, "v0.7.0");
        assert_eq!(r.size, 43971072);
        assert_eq!(r.sha256.as_deref(), Some(hash));
        assert!(r.asset_url.ends_with("/CrewConvo-windows-x86_64.exe"));
        // No digest published, or one we cannot read: only the size is checked.
        assert_eq!(parse_release(&github(Value::Null)).unwrap().sha256, None);
        assert_eq!(parse_release(&github(json!("md5:abc"))).unwrap().sha256, None);
        assert!(parse_release(&json!({ "tag_name": "v1", "assets": [] })).is_err());
        assert!(parse_release(&json!({ "message": "Not Found" })).is_err());
    }

    #[test]
    fn only_github_over_https_unless_the_feed_is_overridden() {
        assert!(url_allowed("https://api.github.com/repos/SamChengYo/CrewConvo/releases/latest", false));
        assert!(url_allowed("https://github.com/SamChengYo/CrewConvo/releases/download/v1/x.exe", false));
        assert!(url_allowed("https://release-assets.githubusercontent.com/github-production-release-asset/1/2", false));
        assert!(url_allowed("https://objects.githubusercontent.com/x", false));
        assert!(!url_allowed("http://github.com/x", false));
        assert!(!url_allowed("https://github.com.evil.example/x", false));
        assert!(!url_allowed("https://evilgithubusercontent.com/x", false));
        assert!(!url_allowed("https://example.com/CrewConvo.exe", false));
        assert!(!url_allowed("file:///C:/x.exe", false));
        // Development feed: a local test server is fine, other schemes still are not.
        assert!(url_allowed("http://127.0.0.1:8199/latest", true));
        assert!(!url_allowed("file:///C:/x.exe", true));
    }

    fn folder(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cc-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn release_for(bytes: &[u8]) -> Release {
        let hash: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
        Release {
            version: "v9.9.9".into(),
            notes_url: String::new(),
            asset_url: String::new(),
            size: bytes.len() as u64,
            sha256: Some(hash),
        }
    }

    #[test]
    fn a_download_that_is_not_what_github_published_is_rejected() {
        let d = folder("verify");
        let file = d.join("x.new");
        std::fs::write(&file, b"new program").unwrap();
        assert!(verify(&file, &release_for(b"new program")).is_ok());
        // Same size, different content.
        assert!(verify(&file, &release_for(b"NEW program")).unwrap_err().contains("SHA-256"));
        assert!(verify(&file, &release_for(b"short")).unwrap_err().contains("大小不符"));
        // No digest published: the size alone decides.
        let mut no_digest = release_for(b"new program");
        no_digest.sha256 = None;
        assert!(verify(&file, &no_digest).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_program_file_keeps_its_name_and_comes_back_when_the_swap_fails() {
        let d = folder("swap");
        // The name the user happens to have.
        let exe = d.join("CrewConvo-windows-x86_64 (1).exe");
        std::fs::write(&exe, b"old").unwrap();
        let new = new_path(&exe);
        std::fs::write(&new, b"new").unwrap();
        swap(&exe, &new).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(old_path(&exe)).unwrap(), b"old");
        assert!(!new.exists());

        // The new file is missing: the old one is back under its name.
        restore(&exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        let err = swap(&exe, &d.join("nope.new")).unwrap_err();
        assert!(err.contains("已還原"));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        assert!(!old_path(&exe).exists());

        // Leftovers of an earlier update are removed at the next start.
        std::fs::write(old_path(&exe), b"older").unwrap();
        std::fs::write(new_path(&exe), b"partial").unwrap();
        clean_up(&exe);
        assert!(!old_path(&exe).exists() && !new_path(&exe).exists());
        assert!(can_replace(&exe));
        assert!(!can_replace(&d.join("no-such-folder").join("x.exe")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
