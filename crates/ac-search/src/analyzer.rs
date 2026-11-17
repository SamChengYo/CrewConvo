//! Text analysis for mixed Chinese / Latin chat messages.
//!
//! Two analyzers feed two index fields:
//! - `cc_ngram`: CJK runs → overlapping bigrams (a single CJK char → unigram);
//!   Latin/digit runs → one lower-cased word. Positions are sequential, so phrase queries work,
//!   and any CJK substring of length ≥ 2 is findable regardless of how a segmenter would cut it.
//! - `cc_jieba`: jieba search-mode segmentation, used to rank results by real words and to match
//!   multi-word queries whose words are not adjacent in the message.

use std::sync::Arc;

use jieba_rs::Jieba;
use tantivy::tokenizer::{Token, TokenStream, Tokenizer};

pub const NGRAM: &str = "cc_ngram";
pub const JIEBA: &str = "cc_jieba";

pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // Hiragana, Katakana
        | 0x3400..=0x4DBF // CJK Ext A
        | 0x4E00..=0x9FFF // CJK Unified
        | 0xAC00..=0xD7AF // Hangul
        | 0xF900..=0xFAFF // CJK Compatibility
        | 0x20000..=0x2FFFF)
}

fn is_word(c: char) -> bool {
    !is_cjk(c) && (c.is_alphanumeric() || c == '_')
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tok {
    pub text: String,
    pub from: usize,
    pub to: usize,
}

/// Bigram / word analysis (see module docs).
pub fn ngram_tokens(text: &str) -> Vec<Tok> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let end_of = |i: usize| if i + 1 < chars.len() { chars[i + 1].0 } else { text.len() };
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].1;
        if is_cjk(c) {
            let start = i;
            while i < chars.len() && is_cjk(chars[i].1) {
                i += 1;
            }
            let run = &chars[start..i];
            if run.len() == 1 {
                out.push(Tok { text: run[0].1.to_string(), from: run[0].0, to: end_of(start) });
            } else {
                for k in 0..run.len() - 1 {
                    let s: String = [run[k].1, run[k + 1].1].iter().collect();
                    out.push(Tok { text: s, from: run[k].0, to: end_of(start + k + 1) });
                }
            }
        } else if is_word(c) {
            let start = i;
            while i < chars.len() && is_word(chars[i].1) {
                i += 1;
            }
            let from = chars[start].0;
            let to = end_of(i - 1);
            out.push(Tok { text: text[from..to].to_lowercase(), from, to });
        } else {
            i += 1;
        }
    }
    out
}

/// jieba search-mode words (lower-cased, punctuation/whitespace dropped).
pub fn jieba_tokens(jieba: &Jieba, text: &str) -> Vec<Tok> {
    jieba
        .cut_for_search(text, true)
        .into_iter()
        .filter(|t| t.word.chars().any(|c| is_cjk(c) || c.is_alphanumeric()))
        .map(|t| Tok { text: t.word.to_lowercase(), from: t.byte_start, to: t.byte_start + t.word.len() })
        .collect()
}

pub struct VecTokenStream {
    tokens: Vec<Token>,
    idx: Option<usize>,
}

impl VecTokenStream {
    fn new(toks: Vec<Tok>) -> Self {
        let tokens = toks
            .into_iter()
            .enumerate()
            .map(|(i, t)| Token { offset_from: t.from, offset_to: t.to, position: i, text: t.text, position_length: 1 })
            .collect();
        VecTokenStream { tokens, idx: None }
    }
}

impl TokenStream for VecTokenStream {
    fn advance(&mut self) -> bool {
        let next = self.idx.map(|i| i + 1).unwrap_or(0);
        self.idx = Some(next);
        next < self.tokens.len()
    }
    fn token(&self) -> &Token {
        &self.tokens[self.idx.unwrap_or(0)]
    }
    fn token_mut(&mut self) -> &mut Token {
        let i = self.idx.unwrap_or(0);
        &mut self.tokens[i]
    }
}

#[derive(Clone)]
pub struct NgramTokenizer;

impl Tokenizer for NgramTokenizer {
    type TokenStream<'a> = VecTokenStream;
    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        VecTokenStream::new(ngram_tokens(text))
    }
}

#[derive(Clone)]
pub struct JiebaTokenizer(pub Arc<Jieba>);

impl Tokenizer for JiebaTokenizer {
    type TokenStream<'a> = VecTokenStream;
    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        VecTokenStream::new(jieba_tokens(&self.0, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(v: Vec<Tok>) -> Vec<String> {
        v.into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn bigrams_and_words() {
        assert_eq!(texts(ngram_tokens("部署 API v2")), vec!["部署", "api", "v2"]);
        assert_eq!(texts(ngram_tokens("明天部署")), vec!["明天", "天部", "部署"]);
        assert_eq!(texts(ngram_tokens("好")), vec!["好"]);
        assert_eq!(texts(ngram_tokens("用Rust寫")), vec!["用", "rust", "寫"]);
    }

    #[test]
    fn offsets_are_byte_ranges() {
        let t = ngram_tokens("a部署b");
        assert_eq!(t[1].text, "部署");
        assert_eq!(&"a部署b"[t[1].from..t[1].to], "部署");
    }

    #[test]
    fn jieba_words() {
        let j = Jieba::new();
        let words = texts(jieba_tokens(&j, "我們明天要部署新版本"));
        assert!(words.contains(&"部署".to_string()), "{words:?}");
        assert!(words.contains(&"版本".to_string()), "{words:?}");
    }
}
