//! Full-text search (spec 8): Tantivy with Chinese-aware analysis, permission filtering
//! inside the query, asynchronous indexing from the event bus, rebuild from the database.

pub mod analyzer;
pub mod index;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ac_core::bus::{Bus, RealtimeEvent};
use ac_core::model::{ConversationKind, Member, MemberKind, MessageView};
use ac_core::{AppError, AppResult};
use ac_store::{MessageScope, Store};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

pub use index::{IndexDoc, IndexQuery, SearchIndex};

pub const MAX_LIMIT: usize = 50;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    #[default]
    Keyword,
    /// Reserved for vector search.
    Semantic,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchParams {
    pub query: String,
    #[serde(default)]
    pub conversation_id: Option<Uuid>,
    /// Author handle (with or without `@`).
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub after: Option<DateTime<Utc>>,
    #[serde(default)]
    pub before: Option<DateTime<Utc>>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub mode: SearchMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct HitConversation {
    pub id: Uuid,
    pub kind: ConversationKind,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub message: MessageView,
    pub conversation: HitConversation,
    /// One message before and after (same conversation / thread) for context.
    pub before: Option<MessageView>,
    pub after: Option<MessageView>,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub total: usize,
    pub offset: usize,
    pub hits: Vec<SearchHit>,
}

#[derive(Clone)]
pub struct SearchService {
    store: Store,
    index: Arc<SearchIndex>,
}

fn to_doc(m: &MessageView) -> IndexDoc {
    IndexDoc {
        message_id: m.id,
        conversation_id: m.conversation_id,
        author_id: m.author_id,
        seq: m.seq,
        created_at: m.created_at,
        body: m.body.clone(),
    }
}

impl SearchService {
    /// `dir = None` → in-memory index (tests).
    pub fn open(store: Store, dir: Option<PathBuf>) -> AppResult<Self> {
        let index = SearchIndex::open(dir.as_deref()).map_err(AppError::Internal)?;
        Ok(SearchService { store, index: Arc::new(index) })
    }

    pub fn index(&self) -> &SearchIndex {
        &self.index
    }

    /// Follows the event bus: new messages are indexed after their transaction committed,
    /// deletions are removed. Commits are batched. If events are lost, the index is rebuilt.
    pub fn start_indexer(&self, bus: &Bus) {
        let mut rx = bus.subscribe();
        let svc = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! {
                    ev = rx.recv() => match ev {
                        Ok(RealtimeEvent::MessageCreated { message }) => {
                            if message.author_kind != MemberKind::System
                                && let Err(err) = svc.index.upsert(&to_doc(&message)) {
                                    tracing::error!(error = %err, "search index upsert failed");
                                }
                        }
                        Ok(RealtimeEvent::MessageDeleted { message_id, .. }) => svc.index.delete(message_id),
                        Ok(_) => {}
                        Err(RecvError::Lagged(n)) => {
                            tracing::warn!(missed = n, "search indexer lagged; rebuilding index");
                            let s = svc.clone();
                            tokio::spawn(async move {
                                if let Err(err) = s.reindex_all().await {
                                    tracing::error!(error = %err, "search reindex failed");
                                }
                            });
                        }
                        Err(RecvError::Closed) => return,
                    },
                    _ = tick.tick() => {
                        if let Err(err) = svc.index.commit() {
                            tracing::error!(error = %err, "search index commit failed");
                        }
                    }
                }
            }
        });
    }

    /// Makes everything indexed so far searchable (used by tests and after bulk loads).
    pub fn flush(&self) -> AppResult<()> {
        self.index.commit().map_err(AppError::Internal)
    }

    /// Rebuilds the whole index from the database.
    pub async fn reindex_all(&self) -> AppResult<u64> {
        self.index.clear().map_err(AppError::Internal)?;
        let n = self.index_from_db(None).await?;
        self.flush()?;
        Ok(n)
    }

    /// Re-indexes one conversation from the database.
    pub async fn reindex_conversation(&self, conversation_id: Uuid) -> AppResult<u64> {
        let n = self.index_from_db(Some(conversation_id)).await?;
        self.flush()?;
        Ok(n)
    }

    async fn index_from_db(&self, conversation_id: Option<Uuid>) -> AppResult<u64> {
        let mut cursor: Option<Uuid> = None;
        let mut n = 0u64;
        loop {
            let batch = self.store.messages_for_index(conversation_id, cursor, 1000).await?;
            if batch.is_empty() {
                break;
            }
            for m in &batch {
                self.index.upsert(&to_doc(m)).map_err(AppError::Internal)?;
            }
            n += batch.len() as u64;
            cursor = batch.last().map(|m| m.id);
        }
        Ok(n)
    }

    /// Rebuilds in the background when the index is empty but the database is not
    /// (first start, or index directory deleted).
    pub fn ensure_populated(&self) {
        let svc = self.clone();
        tokio::spawn(async move {
            if svc.index.num_docs() > 0 {
                return;
            }
            match svc.store.messages_for_index(None, None, 1).await {
                Ok(v) if !v.is_empty() => match svc.reindex_all().await {
                    Ok(n) => tracing::info!(documents = n, "search index rebuilt from database"),
                    Err(err) => tracing::error!(error = %err, "search index rebuild failed"),
                },
                _ => {}
            }
        });
    }

    /// `search_messages` for any member (human or agent): only conversations the caller belongs to.
    pub async fn search(&self, actor: &Member, p: &SearchParams) -> AppResult<SearchResults> {
        if p.mode == SearchMode::Semantic {
            return Err(AppError::bad_request("semantic 搜尋尚未提供，請使用 keyword"));
        }
        // Operators inside the query: from:@handle  in:#channel  after:YYYY-MM-DD  before:YYYY-MM-DD
        let ops = parse_operators(&p.query);
        let text = ops.text.trim();
        if text.is_empty() {
            return Err(AppError::bad_request("query 不能是空的（運算子之外還需要關鍵字）"));
        }
        let limit = p.limit.unwrap_or(20).clamp(1, MAX_LIMIT);
        let offset = p.offset.unwrap_or(0).min(10_000);

        let mut conversations = self.store.member_conversation_ids(actor.id).await?;
        if let Some(c) = p.conversation_id {
            if !conversations.contains(&c) {
                return Err(AppError::forbidden("你不是此對話的成員"));
            }
            conversations = vec![c];
        }
        if let Some(name) = &ops.in_channel {
            let mine = self.store.list_conversations_for(actor.id).await?;
            let ids: Vec<Uuid> = mine
                .iter()
                .filter(|c| c.conversation.name.as_deref().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false))
                .map(|c| c.conversation.id)
                .collect();
            conversations.retain(|c| ids.contains(c));
        }
        let after = p.after.or(ops.after);
        let before = p.before.or(ops.before);
        let author = p.author.clone().or(ops.from.clone());
        let author_id = match author.as_deref().map(|a| a.trim().trim_start_matches('@')).filter(|a| !a.is_empty()) {
            Some(h) => match self.store.member_by_handle(actor.workspace_id, h).await? {
                Some(m) => Some(m.id),
                None => return Ok(SearchResults { total: 0, offset, hits: vec![] }),
            },
            None => None,
        };

        let index = self.index.clone();
        let q = IndexQuery { text: text.to_string(), conversations, author_id, after, before, limit, offset };
        let (raw, total) = tokio::task::spawn_blocking(move || index.search(&q))
            .await
            .map_err(AppError::internal)?
            .map_err(AppError::Internal)?;

        let mut hits = Vec::with_capacity(raw.len());
        for h in raw {
            // The index may lag behind deletions; the database decides.
            let Some(message) = self.store.get_message_view(h.message_id).await? else { continue };
            let Some(conv) = self.store.get_conversation(message.conversation_id).await? else { continue };
            let scope = MessageScope::from_thread(message.thread_root_id);
            let (before, after) = self.store.message_neighbors(conv.id, scope, message.seq).await?;
            hits.push(SearchHit {
                conversation: HitConversation { id: conv.id, kind: conv.kind, name: conv.name.clone() },
                message,
                before,
                after,
                score: h.score,
            });
        }
        Ok(SearchResults { total, offset, hits })
    }
}

/// Search operators parsed out of a free-text query.
#[derive(Debug, Default, PartialEq)]
pub struct QueryOps {
    pub text: String,
    pub from: Option<String>,
    pub in_channel: Option<String>,
    pub after: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
}

fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)).map(|d| d.and_utc())
}

/// `from:@nova in:#ops after:2026-11-01 before:2026-11-30 部署` → operators + remaining text.
/// Unknown `word:value` tokens stay in the text.
pub fn parse_operators(q: &str) -> QueryOps {
    let mut ops = QueryOps::default();
    let mut rest = Vec::new();
    for tok in q.split_whitespace() {
        let (k, v) = match tok.split_once(':') {
            Some((k, v)) if !v.is_empty() => (k.to_ascii_lowercase(), v),
            _ => {
                rest.push(tok);
                continue;
            }
        };
        match k.as_str() {
            "from" => ops.from = Some(v.trim_start_matches('@').to_string()),
            "in" => ops.in_channel = Some(v.trim_start_matches('#').to_string()),
            "after" | "since" => match parse_date(v) {
                Some(d) => ops.after = Some(d),
                None => rest.push(tok),
            },
            "before" | "until" => match parse_date(v) {
                Some(d) => ops.before = Some(d),
                None => rest.push(tok),
            },
            _ => rest.push(tok),
        }
    }
    ops.text = rest.join(" ");
    ops
}

#[cfg(test)]
mod op_tests {
    use super::*;

    #[test]
    fn parses_operators() {
        let o = parse_operators("from:@nova in:#ops after:2026-11-01 部署 失敗 https://x.y/z");
        assert_eq!(o.from.as_deref(), Some("nova"));
        assert_eq!(o.in_channel.as_deref(), Some("ops"));
        assert!(o.after.is_some());
        assert_eq!(o.text, "部署 失敗 https://x.y/z");
        assert_eq!(parse_operators("after:yesterday 部署").text, "after:yesterday 部署");
    }
}
