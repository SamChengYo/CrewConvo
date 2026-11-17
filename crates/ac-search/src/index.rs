//! Tantivy index of messages. The database is the source of truth; this index can be
//! dropped and rebuilt at any time.

use std::ops::Bound;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime as ChronoDateTime, Utc};
use jieba_rs::Jieba;
use tantivy::collector::{Count, TopDocs};
use tantivy::directory::MmapDirectory;
use tantivy::query::{BooleanQuery, BoostQuery, Occur, PhraseQuery, Query, RangeQuery, TermQuery, TermSetQuery};
use tantivy::schema::{
    DateOptions, DateTimePrecision, Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
    Value,
};
use tantivy::{DateTime, Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};
use uuid::Uuid;

use crate::analyzer::{JIEBA, JiebaTokenizer, NGRAM, NgramTokenizer, jieba_tokens, ngram_tokens};

pub type IndexResult<T> = Result<T, String>;

fn e(err: impl std::fmt::Display) -> String {
    err.to_string()
}

#[derive(Clone, Copy)]
struct Fields {
    message_id: Field,
    conversation_id: Field,
    author_id: Field,
    seq: Field,
    created_at: Field,
    body: Field,
    body_words: Field,
}

/// A message as indexed.
#[derive(Debug, Clone)]
pub struct IndexDoc {
    pub message_id: Uuid,
    pub conversation_id: Uuid,
    pub author_id: Uuid,
    pub seq: i64,
    pub created_at: ChronoDateTime<Utc>,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct IndexQuery {
    pub text: String,
    pub conversations: Vec<Uuid>,
    pub author_id: Option<Uuid>,
    pub after: Option<ChronoDateTime<Utc>>,
    pub before: Option<ChronoDateTime<Utc>>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub struct RawHit {
    pub message_id: Uuid,
    pub score: f32,
}

pub struct SearchIndex {
    #[allow(dead_code)] // keeps the index handle alongside its reader / writer
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    fields: Fields,
    jieba: Arc<Jieba>,
    dirty: AtomicBool,
}

fn schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let message_id = b.add_text_field("message_id", STRING | STORED);
    let conversation_id = b.add_text_field("conversation_id", STRING | STORED);
    let author_id = b.add_text_field("author_id", STRING | STORED);
    let seq = b.add_i64_field("seq", STORED);
    let created_at = b.add_date_field(
        "created_at",
        DateOptions::default().set_indexed().set_fast().set_stored().set_precision(DateTimePrecision::Seconds),
    );
    let body = b.add_text_field(
        "body",
        TextOptions::default().set_stored().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(NGRAM)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        ),
    );
    let body_words = b.add_text_field(
        "body_words",
        TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(JIEBA)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        ),
    );
    (b.build(), Fields { message_id, conversation_id, author_id, seq, created_at, body, body_words })
}

fn to_date(dt: ChronoDateTime<Utc>) -> DateTime {
    DateTime::from_timestamp_micros(dt.timestamp_micros())
}

/// Splits a query into quoted phrases ("…", “…”, 「…」) and the remaining bare text.
pub fn split_query(q: &str) -> (Vec<String>, String) {
    let mut phrases = Vec::new();
    let mut bare = String::new();
    let mut cur = String::new();
    let mut close: Option<char> = None;
    for c in q.chars() {
        match close {
            Some(cl) if c == cl => {
                if !cur.trim().is_empty() {
                    phrases.push(cur.trim().to_string());
                }
                cur.clear();
                close = None;
            }
            Some(_) => cur.push(c),
            None => match c {
                '"' => close = Some('"'),
                '“' => close = Some('”'),
                '「' => close = Some('」'),
                _ => bare.push(c),
            },
        }
    }
    if close.is_some() {
        bare.push(' ');
        bare.push_str(&cur);
    }
    (phrases, bare.trim().to_string())
}

impl SearchIndex {
    /// Opens (or creates) an on-disk index, or an in-memory one when `dir` is None.
    pub fn open(dir: Option<&Path>) -> IndexResult<Self> {
        let (schema, fields) = schema();
        let index = match dir {
            Some(d) => {
                std::fs::create_dir_all(d).map_err(e)?;
                let mmap = MmapDirectory::open(d).map_err(e)?;
                Index::open_or_create(mmap, schema.clone()).map_err(e)?
            }
            None => Index::create_in_ram(schema),
        };
        let jieba = Arc::new(Jieba::new());
        index.tokenizers().register(NGRAM, NgramTokenizer);
        index.tokenizers().register(JIEBA, JiebaTokenizer(jieba.clone()));
        let writer: IndexWriter = index.writer_with_num_threads(1, 50_000_000).map_err(e)?;
        let reader = index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into().map_err(e)?;
        Ok(SearchIndex { index, reader, writer: Mutex::new(writer), fields, jieba, dirty: AtomicBool::new(false) })
    }

    fn id_term(&self, id: Uuid) -> Term {
        Term::from_field_text(self.fields.message_id, &id.to_string())
    }

    /// Adds or replaces a message.
    pub fn upsert(&self, d: &IndexDoc) -> IndexResult<()> {
        let f = self.fields;
        let mut doc = TantivyDocument::default();
        doc.add_text(f.message_id, d.message_id.to_string());
        doc.add_text(f.conversation_id, d.conversation_id.to_string());
        doc.add_text(f.author_id, d.author_id.to_string());
        doc.add_i64(f.seq, d.seq);
        doc.add_date(f.created_at, to_date(d.created_at));
        doc.add_text(f.body, &d.body);
        doc.add_text(f.body_words, &d.body);
        let w = self.writer.lock().unwrap();
        w.delete_term(self.id_term(d.message_id));
        w.add_document(doc).map_err(e)?;
        self.dirty.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn delete(&self, message_id: Uuid) {
        self.writer.lock().unwrap().delete_term(self.id_term(message_id));
        self.dirty.store(true, Ordering::SeqCst);
    }

    pub fn clear(&self) -> IndexResult<()> {
        self.writer.lock().unwrap().delete_all_documents().map_err(e)?;
        self.dirty.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Commits pending changes (if any) and makes them visible to searches.
    pub fn commit(&self) -> IndexResult<()> {
        if self.dirty.swap(false, Ordering::SeqCst) {
            self.writer.lock().unwrap().commit().map_err(e)?;
            self.reader.reload().map_err(e)?;
        }
        Ok(())
    }

    pub fn num_docs(&self) -> u64 {
        self.reader.searcher().num_docs()
    }

    fn text_query(&self, text: &str) -> Option<Box<dyn Query>> {
        let f = self.fields;
        let (phrases, bare) = split_query(text);
        let mut must: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        for p in phrases {
            let terms: Vec<Term> =
                ngram_tokens(&p).into_iter().map(|t| Term::from_field_text(f.body, &t.text)).collect();
            match terms.len() {
                0 => {}
                1 => must.push((Occur::Must, Box::new(TermQuery::new(terms[0].clone(), IndexRecordOption::WithFreqs)))),
                _ => must.push((Occur::Must, Box::new(PhraseQuery::new(terms)))),
            }
        }

        if !bare.is_empty() {
            let mut ngram: Vec<String> = ngram_tokens(&bare).into_iter().map(|t| t.text).collect();
            ngram.sort();
            ngram.dedup();
            let mut words: Vec<String> = jieba_tokens(&self.jieba, &bare).into_iter().map(|t| t.text).collect();
            words.sort();
            words.dedup();
            let all_of = |field: Field, toks: &[String]| -> Box<dyn Query> {
                Box::new(BooleanQuery::new(
                    toks.iter()
                        .map(|t| {
                            (
                                Occur::Must,
                                Box::new(TermQuery::new(Term::from_field_text(field, t), IndexRecordOption::WithFreqs))
                                    as Box<dyn Query>,
                            )
                        })
                        .collect(),
                ))
            };
            let mut alternatives: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            if !ngram.is_empty() {
                // Every bigram present: substring-like match, ranked higher.
                alternatives.push((Occur::Should, Box::new(BoostQuery::new(all_of(f.body, &ngram), 2.0))));
            }
            if !words.is_empty() {
                // Every word present, in any order / distance.
                alternatives.push((Occur::Should, all_of(f.body_words, &words)));
            }
            if !alternatives.is_empty() {
                must.push((Occur::Must, Box::new(BooleanQuery::new(alternatives))));
            }
        }
        if must.is_empty() { None } else { Some(Box::new(BooleanQuery::new(must))) }
    }

    /// Searches with the permission filter applied inside the query (before pagination).
    pub fn search(&self, q: &IndexQuery) -> IndexResult<(Vec<RawHit>, usize)> {
        let f = self.fields;
        if q.conversations.is_empty() {
            return Ok((vec![], 0));
        }
        let Some(text) = self.text_query(&q.text) else {
            return Ok((vec![], 0));
        };
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![
            (Occur::Must, text),
            (
                Occur::Must,
                Box::new(TermSetQuery::new(
                    q.conversations.iter().map(|c| Term::from_field_text(f.conversation_id, &c.to_string())),
                )),
            ),
        ];
        if let Some(a) = q.author_id {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(Term::from_field_text(f.author_id, &a.to_string()), IndexRecordOption::Basic)),
            ));
        }
        if q.after.is_some() || q.before.is_some() {
            let lower = q
                .after
                .map(|d| Bound::Included(Term::from_field_date(f.created_at, to_date(d))))
                .unwrap_or(Bound::Unbounded);
            let upper = q
                .before
                .map(|d| Bound::Excluded(Term::from_field_date(f.created_at, to_date(d))))
                .unwrap_or(Bound::Unbounded);
            clauses.push((Occur::Must, Box::new(RangeQuery::new(lower, upper))));
        }
        let query = BooleanQuery::new(clauses);
        let searcher = self.reader.searcher();
        let (top, total) = searcher
            .search(&query, &(TopDocs::with_limit(q.limit.max(1)).and_offset(q.offset).order_by_score(), Count))
            .map_err(e)?;
        let mut hits = Vec::with_capacity(top.len());
        for (score, addr) in top {
            let doc: TantivyDocument = searcher.doc(addr).map_err(e)?;
            let id = doc.get_first(f.message_id).and_then(|v| v.as_str()).and_then(|s| s.parse().ok());
            if let Some(message_id) = id {
                hits.push(RawHit { message_id, score });
            }
        }
        Ok((hits, total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(conv: Uuid, seq: i64, body: &str) -> IndexDoc {
        IndexDoc {
            message_id: Uuid::now_v7(),
            conversation_id: conv,
            author_id: Uuid::nil(),
            seq,
            created_at: Utc::now(),
            body: body.into(),
        }
    }

    fn q(text: &str, convs: &[Uuid]) -> IndexQuery {
        IndexQuery { text: text.into(), conversations: convs.to_vec(), limit: 10, ..Default::default() }
    }

    #[test]
    fn chinese_words_phrases_and_permissions() {
        let idx = SearchIndex::open(None).unwrap();
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let d1 = doc(a, 1, "我們明天要部署新版本到正式環境");
        let d2 = doc(a, 2, "資料庫的慢查詢已經修好了");
        let d3 = doc(b, 1, "私密頻道：部署金鑰在保險箱");
        for d in [&d1, &d2, &d3] {
            idx.upsert(d).unwrap();
        }
        idx.commit().unwrap();

        let (h, n) = idx.search(&q("部署", &[a])).unwrap();
        assert_eq!(n, 1);
        assert_eq!(h[0].message_id, d1.message_id);

        // Non-adjacent words (jieba path).
        let (h, _) = idx.search(&q("部署 版本", &[a])).unwrap();
        assert_eq!(h[0].message_id, d1.message_id);
        let (h, _) = idx.search(&q("部署版本", &[a])).unwrap();
        assert_eq!(h.len(), 1, "words found even if not adjacent");

        // Phrase must be contiguous.
        assert_eq!(idx.search(&q("\"部署新版本\"", &[a])).unwrap().1, 1);
        assert_eq!(idx.search(&q("「新版本部署」", &[a])).unwrap().1, 0);

        // Permission filter.
        assert_eq!(idx.search(&q("部署", &[a, b])).unwrap().1, 2);
        assert_eq!(idx.search(&q("金鑰", &[a])).unwrap().1, 0);

        // Delete.
        idx.delete(d2.message_id);
        idx.commit().unwrap();
        assert_eq!(idx.search(&q("慢查詢", &[a])).unwrap().1, 0);
    }

    #[test]
    fn split_quotes() {
        let (p, b) = split_query("foo \"部署 新版\" bar 「慢查詢」");
        assert_eq!(p, vec!["部署 新版", "慢查詢"]);
        assert_eq!(b, "foo  bar");
    }
}
