//! Isolated working copies for coding agents that may change files.
//!
//! An agent with write access does not work in the user's checkout: it gets a git worktree of
//! the project on a branch of its own. What it did is committed there and listed as "work" the
//! user can merge into the project or throw away. Every function here blocks (it runs git);
//! call them from `spawn_blocking`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::platforms;

/// Folder inside the project that holds the worktrees; kept out of `git status` through
/// `.git/info/exclude`, which unlike `.gitignore` is not part of the project's own files.
pub const FOLDER: &str = ".crewconvo";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkRecord {
    pub id: String,
    pub agent_id: Uuid,
    pub agent_handle: String,
    pub conversation_id: Uuid,
    pub conversation_name: String,
    pub thread_root_id: Option<Uuid>,
    pub project: String,
    pub worktree: String,
    pub branch: String,
    /// The project commit the worktree was created from.
    pub base_commit: String,
    /// The branch the project was on at that time; merging goes back into it.
    pub base_branch: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
    /// Commits on the branch that the base does not have, as of the last run.
    #[serde(default)]
    pub ahead: u32,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    work: Vec<WorkRecord>,
}

pub struct Works {
    file: PathBuf,
    records: Mutex<Vec<WorkRecord>>,
}

impl Works {
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("local-work.json");
        let saved: Saved =
            std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Works { file, records: Mutex::new(saved.work) }
    }

    fn save(&self, records: &[WorkRecord]) {
        let text = serde_json::to_string_pretty(&Saved { work: records.to_vec() }).unwrap_or_default();
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&self.file, text) {
            tracing::warn!(error = %e, file = %self.file.display(), "could not save local work");
        }
    }

    pub fn list(&self) -> Vec<WorkRecord> {
        let mut all = self.records.lock().unwrap().clone();
        all.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at));
        all
    }

    pub fn get(&self, id: &str) -> Option<WorkRecord> {
        self.records.lock().unwrap().iter().find(|r| r.id == id).cloned()
    }

    /// The working copy of one agent for one conversation (or one thread of it) in one project.
    pub fn find(&self, agent: Uuid, conversation: Uuid, thread: Option<Uuid>, project: &str) -> Option<WorkRecord> {
        let records = self.records.lock().unwrap();
        records
            .iter()
            .find(|r| {
                r.agent_id == agent
                    && r.conversation_id == conversation
                    && r.thread_root_id == thread
                    && r.project == project
            })
            .cloned()
    }

    pub fn add(&self, record: WorkRecord) {
        let mut records = self.records.lock().unwrap();
        records.retain(|r| r.id != record.id);
        records.push(record);
        self.save(&records);
    }

    pub fn used(&self, id: &str, ahead: u32, now: DateTime<Utc>) {
        let mut records = self.records.lock().unwrap();
        if let Some(r) = records.iter_mut().find(|r| r.id == id) {
            r.ahead = ahead;
            r.last_used_at = now;
            self.save(&records);
        }
    }

    pub fn remove(&self, id: &str) {
        let mut records = self.records.lock().unwrap();
        records.retain(|r| r.id != id);
        self.save(&records);
    }
}

/// Runs git with separate arguments (never through a shell) and no console window.
/// Ok: what it printed. Err: why it failed, short enough to show to a person.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let exe = platforms::which(&["git"]).ok_or_else(|| "這台電腦上找不到 git".to_string())?;
    let out = platforms::quiet_command(&exe)
        .current_dir(dir)
        .args(args)
        // Never wait for a password or an editor: nobody is there to answer.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("無法執行 git：{e}"))?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if out.status.success() {
        // Leading spaces are kept: they mean something in `status --porcelain` and `diff --stat`.
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        let mut why = text(&out.stderr);
        if why.is_empty() {
            why = text(&out.stdout);
        }
        Err(why.chars().take(400).collect())
    }
}

/// A git project we can make worktrees of: the folder is the top of a repository that has at
/// least one commit. A subfolder of a repository, or a repository without commits, is not.
pub fn is_repo(project: &Path) -> bool {
    project.join(".git").exists() && git(project, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok()
}

fn slug(handle: &str) -> String {
    let s: String = handle.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "agent".into() } else { s }
}

/// (folder name under `.crewconvo/worktrees`, branch) for a new working copy.
pub fn names(handle: &str, id: &str) -> (String, String) {
    let short: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect();
    let name = format!("{}-{short}", slug(handle));
    (name.clone(), format!("crewconvo/{name}"))
}

/// Keeps `.crewconvo/` out of the project's `git status` without touching its `.gitignore`.
fn exclude(project: &Path) -> Result<(), String> {
    let common = git(project, &["rev-parse", "--git-common-dir"])?;
    let file = project.join(common).join("info").join("exclude");
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    let line = format!("{FOLDER}/");
    if current.lines().any(|l| l.trim() == line) {
        return Ok(());
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let sep = if current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
    std::fs::write(&file, format!("{current}{sep}{line}\n")).map_err(|e| e.to_string())
}

pub struct Created {
    pub worktree: PathBuf,
    pub branch: String,
    pub base_commit: String,
    pub base_branch: Option<String>,
}

/// Makes a worktree of the project's current commit on a new branch.
pub fn create(project: &Path, handle: &str, id: &str) -> Result<Created, String> {
    let (name, branch) = names(handle, id);
    let worktree = project.join(FOLDER).join("worktrees").join(&name);
    exclude(project)?;
    let base_commit = git(project, &["rev-parse", "HEAD"])?;
    let base_branch = git(project, &["symbolic-ref", "--short", "-q", "HEAD"]).ok().filter(|b| !b.is_empty());
    let path = worktree.to_string_lossy().to_string();
    git(project, &["worktree", "add", "-b", &branch, &path, &base_commit])?;
    Ok(Created { worktree, branch, base_commit, base_branch })
}

pub fn head(dir: &Path) -> Option<String> {
    git(dir, &["rev-parse", "HEAD"]).ok()
}

/// Commits everything changed in the worktree. Ok(false) when there was nothing to commit.
/// Uses the user's git identity; `fallback` (name, e-mail) only when git has none configured.
pub fn commit_all(worktree: &Path, message: &str, fallback: (&str, &str)) -> Result<bool, String> {
    if git(worktree, &["status", "--porcelain"])?.is_empty() {
        return Ok(false);
    }
    git(worktree, &["add", "-A"])?;
    let mut args: Vec<String> = identity_args(worktree, fallback);
    args.extend(["commit", "-q", "-m", message].map(String::from));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git(worktree, &args)?;
    Ok(true)
}

fn identity_args(dir: &Path, fallback: (&str, &str)) -> Vec<String> {
    let has = |key: &str| git(dir, &["config", key]).is_ok_and(|v| !v.is_empty());
    if has("user.name") && has("user.email") {
        vec![]
    } else {
        vec!["-c".into(), format!("user.name={}", fallback.0), "-c".into(), format!("user.email={}", fallback.1)]
    }
}

pub fn ahead(project: &Path, base: &str, branch: &str) -> u32 {
    git(project, &["rev-list", "--count", &format!("{base}..{branch}")]).ok().and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// `git diff --stat` of everything the branch changed since its base, at most `max` lines
/// (the summary line is always kept).
pub fn diffstat(project: &Path, base: &str, branch: &str, max: usize) -> String {
    let out = git(project, &["diff", "--stat=100", base, branch]).unwrap_or_default();
    cap_lines(&out, max)
}

fn cap_lines(text: &str, max: usize) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
    if lines.len() <= max || max < 2 {
        return lines.join("\n");
    }
    let hidden = lines.len() - max;
    let mut kept: Vec<String> = lines[..max - 1].iter().map(|l| l.to_string()).collect();
    kept.push(format!(" …（另外 {hidden} 個檔案）"));
    kept.push(lines[lines.len() - 1].to_string());
    kept.join("\n")
}

/// Removes the worktree and its branch. Used after a merge and for discarding.
pub fn remove(rec: &WorkRecord) -> Result<(), String> {
    let project = Path::new(&rec.project);
    if Path::new(&rec.worktree).exists() {
        git(project, &["worktree", "remove", "--force", &rec.worktree])?;
    }
    let _ = git(project, &["worktree", "prune"]);
    // Already gone (deleted by hand) is fine.
    if git(project, &["rev-parse", "--verify", "-q", &format!("refs/heads/{}", rec.branch)]).is_ok() {
        git(project, &["branch", "-D", &rec.branch])?;
    }
    Ok(())
}

/// Merges the work into the project checkout. Refuses rather than touching anything the user
/// has in progress: never switches branches, never stashes; a conflicting merge is undone.
pub fn merge(rec: &WorkRecord, fallback: (&str, &str)) -> Result<(), String> {
    let project = Path::new(&rec.project);
    if !project.is_dir() {
        return Err(format!("專案資料夾已經不存在：{}", rec.project));
    }
    let Some(base_branch) = &rec.base_branch else {
        return Err(format!(
            "建立這份工作時專案不在任何分支上，無法自動合併。請自己在專案裡執行 git merge {}",
            rec.branch
        ));
    };
    let current = git(project, &["symbolic-ref", "--short", "-q", "HEAD"]).unwrap_or_default();
    if &current != base_branch {
        let now = if current.is_empty() { "不在任何分支上".to_string() } else { format!("在 {current}") };
        return Err(format!(
            "這份工作是從分支 {base_branch} 開始的，但專案現在{now}。請先在專案裡切回 {base_branch} 再合併"
        ));
    }
    let dirty = git(project, &["status", "--porcelain", "--untracked-files=no"])?;
    if !dirty.is_empty() {
        let files: Vec<&str> = dirty.lines().take(5).map(|l| l.get(3..).unwrap_or(l)).collect();
        return Err(format!("專案裡有還沒提交的修改（{}），請先提交或還原再合併", files.join("、")));
    }
    let mut args = identity_args(project, fallback);
    let message = format!("Merge {} (CrewConvo @{})", rec.branch, rec.agent_handle);
    args.extend(["merge", "--no-ff", "--no-edit", "-m", &message, &rec.branch].map(String::from));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Err(e) = git(project, &args) {
        let conflicts = git(project, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
        let _ = git(project, &["merge", "--abort"]);
        if conflicts.is_empty() {
            return Err(format!("合併失敗，專案沒有被更動：{e}"));
        }
        let files: Vec<&str> = conflicts.lines().take(8).collect();
        return Err(format!(
            "合併時發生衝突（{}），已經取消合併，專案和這份工作都維持原狀。請自己在專案裡執行 git merge {} 並解決衝突，或捨棄這份工作",
            files.join("、"),
            rec.branch
        ));
    }
    remove(rec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_conversations_of_one_project_have_their_own_working_copies() {
        let dir = std::env::temp_dir().join(format!("cc-work-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let works = Works::load(&dir);
        let (agent, first, second, thread) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        let rec = |id: &str, conversation: Uuid, thread: Option<Uuid>| WorkRecord {
            id: id.into(),
            agent_id: agent,
            agent_handle: "codex".into(),
            conversation_id: conversation,
            conversation_name: "#shop".into(),
            thread_root_id: thread,
            project: "/work/shop".into(),
            worktree: format!("/work/shop/.crewconvo/worktrees/codex-{id}"),
            branch: format!("crewconvo/codex-{id}"),
            base_commit: "abc".into(),
            base_branch: Some("main".into()),
            created_at: Utc::now(),
            last_used_at: Utc::now(),
            ahead: 0,
        };
        works.add(rec("aaaa1111", first, None));
        works.add(rec("bbbb2222", second, None));
        works.add(rec("cccc3333", first, Some(thread)));
        // Same agent, same folder: each conversation (and each thread) finds only its own.
        let found = |c: Uuid, t: Option<Uuid>| works.find(agent, c, t, "/work/shop").map(|r| r.id);
        assert_eq!(found(first, None).as_deref(), Some("aaaa1111"));
        assert_eq!(found(second, None).as_deref(), Some("bbbb2222"));
        assert_eq!(found(first, Some(thread)).as_deref(), Some("cccc3333"));
        assert_eq!(found(second, Some(thread)), None);
        // The same conversation bound to another folder has none there.
        assert!(works.find(agent, first, None, "/work/other").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn worktree_and_branch_names() {
        assert_eq!(names("codex", "3f9a1c2e-77aa-4b11"), ("codex-3f9a1c2e".into(), "crewconvo/codex-3f9a1c2e".into()));
        // Handles are free text; what goes into a path and a branch name is not.
        assert_eq!(names("小明 bot/..", "abcd1234ffff").0, "bot-abcd1234");
        assert_eq!(names("///", "abcd1234").1, "crewconvo/agent-abcd1234");
        assert_eq!(names("claude-2", "ab").0, "claude-2-ab");
    }

    #[test]
    fn long_diffstats_are_cut_but_keep_the_summary() {
        let stat: String = (1..=30).map(|i| format!(" f{i}.rs | 2 +-\n")).collect::<String>() + " 30 files changed\n";
        let cut = cap_lines(&stat, 15);
        let lines: Vec<&str> = cut.lines().collect();
        assert_eq!(lines.len(), 16);
        assert_eq!(lines[0], " f1.rs | 2 +-");
        assert!(lines[14].contains("另外 16 個檔案"));
        assert_eq!(lines[15], " 30 files changed");
        assert_eq!(cap_lines(" a | 1 +\n 1 file changed\n", 15), " a | 1 +\n 1 file changed");
    }

    fn sh(dir: &Path, args: &[&str]) {
        git(dir, args).unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    }

    /// A throwaway repository with one commit; None when git is not installed.
    fn repo(name: &str) -> Option<PathBuf> {
        platforms::which(&["git"])?;
        let dir = std::env::temp_dir().join(format!("cc-work-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sh(&dir, &["init", "-q", "-b", "main"]);
        sh(&dir, &["config", "user.name", "T"]);
        sh(&dir, &["config", "user.email", "t@example.com"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        sh(&dir, &["add", "-A"]);
        sh(&dir, &["commit", "-q", "-m", "first"]);
        Some(dir)
    }

    fn record(project: &Path, c: &Created) -> WorkRecord {
        WorkRecord {
            id: "abcd1234".into(),
            agent_id: Uuid::from_u128(1),
            agent_handle: "codex".into(),
            conversation_id: Uuid::from_u128(2),
            conversation_name: "#dev".into(),
            thread_root_id: None,
            project: project.to_string_lossy().into(),
            worktree: c.worktree.to_string_lossy().into(),
            branch: c.branch.clone(),
            base_commit: c.base_commit.clone(),
            base_branch: c.base_branch.clone(),
            created_at: Utc::now(),
            last_used_at: Utc::now(),
            ahead: 0,
        }
    }

    #[test]
    fn work_is_committed_on_its_own_branch_and_merged_or_refused() {
        let Some(project) = repo("merge") else { return };
        assert!(is_repo(&project));
        let c = create(&project, "codex", "abcd1234").unwrap();
        assert!(c.worktree.ends_with(Path::new(".crewconvo/worktrees/codex-abcd1234")));
        assert_eq!(c.base_branch.as_deref(), Some("main"));
        // The worktree folder does not show up in the project.
        assert_eq!(git(&project, &["status", "--porcelain"]).unwrap(), "");
        let rec = record(&project, &c);
        let who = ("Codex", "codex@crewconvo.local");
        assert!(!commit_all(&c.worktree, "nothing", who).unwrap());
        std::fs::write(c.worktree.join("b.txt"), "new\n").unwrap();
        std::fs::write(c.worktree.join("a.txt"), "one\ntwo\n").unwrap();
        assert!(commit_all(&c.worktree, "CrewConvo: @codex — add b", who).unwrap());
        assert_eq!(ahead(&project, &rec.base_commit, &rec.branch), 1);
        assert!(diffstat(&project, &rec.base_commit, &rec.branch, 15).contains("2 files changed"));
        assert!(!project.join("b.txt").exists());

        // Uncommitted changes to tracked files in the checkout: refused, nothing touched.
        std::fs::write(project.join("a.txt"), "mine\n").unwrap();
        assert!(merge(&rec, who).unwrap_err().contains("還沒提交的修改"));
        // Committed, and now both sides changed the same line: conflict, undone.
        sh(&project, &["commit", "-q", "-am", "mine"]);
        let before = head(&project);
        assert!(merge(&rec, who).unwrap_err().contains("衝突"));
        assert_eq!(head(&project), before);
        assert_eq!(git(&project, &["status", "--porcelain"]).unwrap(), "");
        assert!(c.worktree.exists());
        // On another branch than the one the work started from: refused.
        sh(&project, &["checkout", "-q", "-b", "other"]);
        assert!(merge(&rec, who).unwrap_err().contains("切回 main"));
        // Discard removes the worktree and the branch.
        remove(&rec).unwrap();
        assert!(!c.worktree.exists());
        assert!(git(&project, &["rev-parse", "--verify", "-q", "refs/heads/crewconvo/codex-abcd1234"]).is_err());
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn a_clean_merge_lands_in_the_project_and_cleans_up() {
        let Some(project) = repo("clean") else { return };
        let c = create(&project, "claude", "ffff0000").unwrap();
        let rec = record(&project, &c);
        std::fs::write(c.worktree.join("b.txt"), "new\n").unwrap();
        assert!(commit_all(&c.worktree, "add b", ("Claude", "claude@crewconvo.local")).unwrap());
        // Untracked files of the user do not block a merge.
        std::fs::write(project.join("notes.txt"), "mine\n").unwrap();
        merge(&rec, ("Claude", "claude@crewconvo.local")).unwrap();
        assert_eq!(std::fs::read_to_string(project.join("b.txt")).unwrap().trim(), "new");
        assert!(project.join("notes.txt").exists());
        assert!(!c.worktree.exists());
        assert!(git(&project, &["rev-parse", "--verify", "-q", &format!("refs/heads/{}", rec.branch)]).is_err());
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn a_folder_without_commits_is_not_a_git_project() {
        let dir = std::env::temp_dir().join(format!("cc-work-plain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_repo(&dir));
        if platforms::which(&["git"]).is_some() {
            sh(&dir, &["init", "-q"]);
            assert!(!is_repo(&dir));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
