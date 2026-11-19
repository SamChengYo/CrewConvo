//! The coding-agent sessions CrewConvo has started on this computer, and what each one is for.
//!
//! Every wake used to be a brand-new session of the command-line tool, so the coding agent
//! forgot what it had just done. Here each session is written down; a later wake that belongs
//! to the same work continues it. Nothing here talks to the tools themselves.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Not continued after this long without use: the work it was for is probably over.
const IDLE_DAYS: i64 = 7;
/// Not continued after this many wakes: a very long session gets slow and expensive.
const MAX_USES: u32 = 30;
/// Records untouched for this long are dropped.
const KEEP_DAYS: i64 = 30;
const TRIGGER_CHARS: usize = 80;
const RECENT_CHARS: usize = 160;
const RECENT_MARK: &str = "｜最近：";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// The session id of the command-line tool.
    pub id: String,
    pub platform: String,
    pub agent_id: Uuid,
    pub conversation_id: Uuid,
    pub conversation_name: String,
    pub thread_root_id: Option<Uuid>,
    /// What the session is for, and what it did most recently.
    pub purpose: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
    pub uses: u32,
    /// The folder the session was created in; None is the home folder (also for records
    /// written before folders existed). A session only continues in the folder it knows:
    /// Claude Code cannot find it from anywhere else, and its notes are about those files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    sessions: Vec<SessionRecord>,
}

#[derive(Default)]
struct State {
    records: Vec<SessionRecord>,
    /// Sessions a running wake is using right now.
    busy: HashSet<String>,
}

pub struct Sessions {
    file: PathBuf,
    state: Mutex<State>,
}

fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(max).collect();
    if flat.chars().count() > max {
        out.push('…');
    }
    out
}

/// "#頻道：觸發訊息的開頭" — written when the session is created.
pub fn initial_purpose(conversation_name: &str, trigger: &str) -> String {
    format!("{conversation_name}：{}", clip(trigger, TRIGGER_CHARS))
}

/// Keeps what the session was created for and replaces what it did most recently.
pub fn with_recent(purpose: &str, log: &str) -> String {
    let base = purpose.split(RECENT_MARK).next().unwrap_or_default();
    let recent = clip(log, RECENT_CHARS);
    if recent.is_empty() { base.to_string() } else { format!("{base}{RECENT_MARK}{recent}") }
}

fn worn_out(r: &SessionRecord, now: DateTime<Utc>) -> bool {
    r.uses >= MAX_USES || now - r.last_used_at > Duration::days(IDLE_DAYS)
}

/// The session of the same thread, if the wake is in a thread.
pub fn by_thread(candidates: &[SessionRecord], thread_root_id: Option<Uuid>) -> Option<&SessionRecord> {
    let thread = thread_root_id?;
    candidates.iter().filter(|r| r.thread_root_id == Some(thread)).max_by_key(|r| r.last_used_at)
}

/// The rule used when the decision model cannot be asked: the most recently used session of the
/// same conversation that does not belong to a thread. A wake in a thread that has no session
/// yet gets none — a thread is its own piece of work.
pub fn by_conversation(
    candidates: &[SessionRecord],
    conversation_id: Uuid,
    thread_root_id: Option<Uuid>,
) -> Option<&SessionRecord> {
    if thread_root_id.is_some() {
        return None;
    }
    candidates
        .iter()
        .filter(|r| r.conversation_id == conversation_id && r.thread_root_id.is_none())
        .max_by_key(|r| r.last_used_at)
}

impl Sessions {
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("local-sessions.json");
        let saved: Saved =
            std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Sessions { file, state: Mutex::new(State { records: saved.sessions, busy: HashSet::new() }) }
    }

    fn save(&self, state: &State) {
        let text = serde_json::to_string_pretty(&Saved { sessions: state.records.clone() }).unwrap_or_default();
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&self.file, text) {
            tracing::warn!(error = %e, file = %self.file.display(), "could not save local sessions");
        }
    }

    /// Every record, most recently used first.
    pub fn list(&self) -> Vec<SessionRecord> {
        let mut all = self.state.lock().unwrap().records.clone();
        all.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at));
        all
    }

    /// The sessions a new wake of this agent may continue: its own, on the same tool, not worn
    /// out and not in use. Drops records nobody has touched for a long time.
    pub fn candidates(
        &self,
        platform: &str,
        agent_id: Uuid,
        dir: Option<&str>,
        now: DateTime<Utc>,
    ) -> Vec<SessionRecord> {
        let mut state = self.state.lock().unwrap();
        let before = state.records.len();
        state.records.retain(|r| now - r.last_used_at <= Duration::days(KEEP_DAYS));
        if state.records.len() != before {
            self.save(&state);
        }
        state
            .records
            .iter()
            .filter(|r| r.platform == platform && r.agent_id == agent_id && r.dir.as_deref() == dir)
            .filter(|r| !worn_out(r, now) && !state.busy.contains(&r.id))
            .cloned()
            .collect()
    }

    /// Reserves a session for one wake. False when it is gone or another wake took it first —
    /// two runs must never write into the same session at once.
    pub fn claim(&self, id: &str) -> bool {
        let mut state = self.state.lock().unwrap();
        state.records.iter().any(|r| r.id == id) && state.busy.insert(id.to_string())
    }

    pub fn release(&self, id: &str) {
        self.state.lock().unwrap().busy.remove(id);
    }

    pub fn add(&self, record: SessionRecord) {
        let mut state = self.state.lock().unwrap();
        state.records.retain(|r| r.id != record.id);
        state.records.push(record);
        self.save(&state);
    }

    /// After a wake that continued the session: one more use, and what it did this time.
    pub fn used(&self, id: &str, log: &str, now: DateTime<Utc>) {
        let mut state = self.state.lock().unwrap();
        if let Some(r) = state.records.iter_mut().find(|r| r.id == id) {
            r.uses += 1;
            r.last_used_at = now;
            r.purpose = with_recent(&r.purpose, log);
            self.save(&state);
        }
    }

    /// Removes one record (the tool no longer has the session, or the user asked to forget it).
    pub fn forget(&self, id: &str) -> bool {
        let mut state = self.state.lock().unwrap();
        let before = state.records.len();
        state.records.retain(|r| r.id != id);
        let removed = state.records.len() != before;
        if removed {
            self.save(&state);
        }
        removed
    }

    /// Removes the records of the sessions created in a folder that is going away.
    pub fn forget_dir(&self, dir: &str) {
        let mut state = self.state.lock().unwrap();
        let before = state.records.len();
        state.records.retain(|r| r.dir.as_deref() != Some(dir));
        if state.records.len() != before {
            self.save(&state);
        }
    }

    /// Removes the records of an agent that was disconnected.
    pub fn forget_agent(&self, agent_id: Uuid) {
        let mut state = self.state.lock().unwrap();
        let before = state.records.len();
        state.records.retain(|r| r.agent_id != agent_id);
        if state.records.len() != before {
            self.save(&state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cc-sessions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn rec(id: &str, agent: Uuid, conv: Uuid, thread: Option<Uuid>, used_days_ago: i64, uses: u32) -> SessionRecord {
        let at = Utc::now() - Duration::days(used_days_ago);
        SessionRecord {
            id: id.into(),
            platform: "claude-code".into(),
            agent_id: agent,
            conversation_id: conv,
            conversation_name: "#dev".into(),
            thread_root_id: thread,
            purpose: initial_purpose("#dev", "修登入"),
            created_at: at,
            last_used_at: at,
            uses,
            dir: None,
        }
    }

    #[test]
    fn a_thread_continues_its_own_session() {
        let (agent, conv, thread) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let all = vec![rec("plain", agent, conv, None, 0, 1), rec("thread", agent, conv, Some(thread), 2, 1)];
        assert_eq!(by_thread(&all, Some(thread)).map(|r| r.id.as_str()), Some("thread"));
        assert!(by_thread(&all, Some(Uuid::from_u128(9))).is_none());
        assert!(by_thread(&all, None).is_none());
    }

    #[test]
    fn falls_back_to_the_latest_session_of_the_conversation() {
        let (agent, conv, other) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(4));
        let all = vec![
            rec("old", agent, conv, None, 3, 1),
            rec("new", agent, conv, None, 1, 1),
            rec("elsewhere", agent, other, None, 0, 1),
            rec("thread", agent, conv, Some(Uuid::from_u128(3)), 0, 1),
        ];
        assert_eq!(by_conversation(&all, conv, None).map(|r| r.id.as_str()), Some("new"));
        assert!(by_conversation(&all, Uuid::from_u128(8), None).is_none());
        // A thread without a session of its own starts a new one.
        assert!(by_conversation(&all, conv, Some(Uuid::from_u128(5))).is_none());
    }

    #[test]
    fn worn_out_and_busy_sessions_are_not_candidates() {
        let d = dir("worn");
        let s = Sessions::load(&d);
        let (agent, conv) = (Uuid::from_u128(1), Uuid::from_u128(2));
        s.add(rec("fresh", agent, conv, None, 1, 3));
        s.add(rec("idle", agent, conv, None, 8, 3));
        s.add(rec("long", agent, conv, None, 0, MAX_USES));
        s.add(rec("ancient", agent, conv, None, 31, 1));
        s.add(rec("other-agent", Uuid::from_u128(7), conv, None, 0, 1));
        let mut codex = rec("other-tool", agent, conv, None, 0, 1);
        codex.platform = "codex".into();
        s.add(codex);
        let ids = |v: Vec<SessionRecord>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids(s.candidates("claude-code", agent, None, Utc::now())), vec!["fresh"]);
        // Untouched for 30 days: the record itself is gone, also from the file.
        assert!(!s.list().iter().any(|r| r.id == "ancient"));
        assert!(!Sessions::load(&d).list().iter().any(|r| r.id == "ancient"));
        // In use by a running wake: a second wake cannot take it.
        assert!(s.claim("fresh"));
        assert!(!s.claim("fresh"));
        assert!(s.candidates("claude-code", agent, None, Utc::now()).is_empty());
        s.release("fresh");
        assert_eq!(ids(s.candidates("claude-code", agent, None, Utc::now())), vec!["fresh"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_stale_session_is_removed_and_cannot_be_claimed() {
        let d = dir("stale");
        let s = Sessions::load(&d);
        let (agent, conv) = (Uuid::from_u128(1), Uuid::from_u128(2));
        s.add(rec("gone", agent, conv, None, 0, 1));
        assert!(s.forget("gone"));
        assert!(!s.forget("gone"));
        assert!(!s.claim("gone"));
        assert!(s.candidates("claude-code", agent, None, Utc::now()).is_empty());
        assert!(Sessions::load(&d).list().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_session_only_continues_in_the_folder_it_was_created_in() {
        let d = dir("folders");
        let s = Sessions::load(&d);
        let (agent, conv) = (Uuid::from_u128(1), Uuid::from_u128(2));
        // Written before folders existed: counts as the home folder.
        s.add(rec("home", agent, conv, None, 0, 1));
        let mut project = rec("project", agent, conv, None, 0, 1);
        project.dir = Some("C:\\proj".into());
        s.add(project);
        let mut worktree = rec("worktree", agent, conv, None, 0, 1);
        worktree.dir = Some("C:\\proj\\.crewconvo\\worktrees\\claude-1".into());
        s.add(worktree);
        let ids = |dir: Option<&str>| {
            s.candidates("claude-code", agent, dir, Utc::now()).into_iter().map(|r| r.id).collect::<Vec<_>>()
        };
        assert_eq!(ids(None), vec!["home"]);
        assert_eq!(ids(Some("C:\\proj")), vec!["project"]);
        assert_eq!(ids(Some("C:\\proj\\.crewconvo\\worktrees\\claude-1")), vec!["worktree"]);
        assert!(ids(Some("C:\\elsewhere")).is_empty());
        // The old record is read back without a folder.
        assert_eq!(Sessions::load(&d).list().iter().find(|r| r.id == "home").unwrap().dir, None);
        s.forget_dir("C:\\proj\\.crewconvo\\worktrees\\claude-1");
        assert!(ids(Some("C:\\proj\\.crewconvo\\worktrees\\claude-1")).is_empty());
        assert_eq!(s.list().len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rebinding_a_conversation_to_another_folder_leaves_its_old_sessions_behind() {
        let d = dir("rebind");
        let s = Sessions::load(&d);
        let (agent, conv, thread) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        // Made while the conversation was bound to the old folder: one for the conversation,
        // one for a thread of it.
        let mut main = rec("old-main", agent, conv, None, 0, 1);
        main.dir = Some("/work/old".into());
        s.add(main);
        let mut in_thread = rec("old-thread", agent, conv, Some(thread), 0, 1);
        in_thread.dir = Some("/work/old".into());
        s.add(in_thread);

        // Same conversation, same thread, now bound to another folder: none of the rules
        // that choose a session has anything to choose from.
        let now = s.candidates("claude-code", agent, Some("/work/new"), Utc::now());
        assert!(now.is_empty());
        assert!(by_thread(&now, Some(thread)).is_none());
        assert!(by_conversation(&now, conv, None).is_none());

        // A session made in the new folder is the only one continued there…
        let mut fresh = rec("new-main", agent, conv, None, 0, 1);
        fresh.dir = Some("/work/new".into());
        s.add(fresh);
        let now = s.candidates("claude-code", agent, Some("/work/new"), Utc::now());
        assert_eq!(by_conversation(&now, conv, None).map(|r| r.id.as_str()), Some("new-main"));
        assert!(by_thread(&now, Some(thread)).is_none());

        // …and binding the old folder again finds the old ones, untouched.
        let back = s.candidates("claude-code", agent, Some("/work/old"), Utc::now());
        assert_eq!(by_thread(&back, Some(thread)).map(|r| r.id.as_str()), Some("old-thread"));
        assert_eq!(by_conversation(&back, conv, None).map(|r| r.id.as_str()), Some("old-main"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn purpose_says_what_it_is_for_and_what_it_did_last() {
        let long = "請幫我".repeat(40);
        let p = initial_purpose("#dev", &format!("  第一行\n第二行 {long}"));
        assert!(p.starts_with("#dev：第一行 第二行 請幫我"));
        assert_eq!(p.chars().count(), "#dev：".chars().count() + TRIGGER_CHARS + 1);
        let once = with_recent(&p, "改好了 login.rs");
        assert_eq!(once, format!("{p}｜最近：改好了 login.rs"));
        // The newest log replaces the previous one; the original purpose stays.
        let twice = with_recent(&once, &"測試".repeat(200));
        assert!(twice.starts_with(&format!("{p}｜最近：測試")));
        assert_eq!(twice.matches("最近：").count(), 1);
        assert_eq!(twice.chars().count(), p.chars().count() + RECENT_MARK.chars().count() + RECENT_CHARS + 1);
        assert_eq!(with_recent(&once, "  "), p);
    }

    #[test]
    fn use_is_counted_and_saved() {
        let d = dir("used");
        let s = Sessions::load(&d);
        let (agent, conv) = (Uuid::from_u128(1), Uuid::from_u128(2));
        s.add(rec("a", agent, conv, None, 2, 1));
        s.used("a", "做了一件事", Utc::now());
        let again = Sessions::load(&d).list();
        assert_eq!(again[0].uses, 2);
        assert!(again[0].purpose.ends_with("｜最近：做了一件事"));
        assert!(Utc::now() - again[0].last_used_at < Duration::minutes(1));
        let _ = std::fs::remove_dir_all(&d);
    }
}
