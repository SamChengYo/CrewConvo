//! `@handle` parsing. The server is the only source of truth for mentions.

/// Valid handle characters: letters and digits of any script (so a name such as `客服助理` can be
/// a handle), `_`, `-`, `.`.
pub fn is_handle_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

/// `@all` notifies everyone in the conversation, so these names cannot be handles.
pub const RESERVED_HANDLES: &[&str] = &["all", "everyone", "here", "channel"];

/// The handle that mentions every member of the conversation.
pub const MENTION_ALL: &str = "all";

/// Validates a handle for registration: 2-32 chars, starts with a letter or digit.
pub fn validate_handle(handle: &str) -> Result<(), String> {
    let len = handle.chars().count();
    if !(2..=32).contains(&len) {
        return Err("handle 長度需為 2–32 個字元".into());
    }
    if !handle.chars().all(is_handle_char) {
        return Err("handle 只能包含文字、數字、底線、連字號與句點（不能有空白）".into());
    }
    let first = handle.chars().next().unwrap();
    if !first.is_alphanumeric() {
        return Err("handle 需以文字或數字開頭".into());
    }
    if handle.ends_with('.') {
        return Err("handle 不能以句點結尾".into());
    }
    if RESERVED_HANDLES.contains(&handle.to_ascii_lowercase().as_str()) {
        return Err(format!("「{handle}」是保留字（@{handle} 用來通知所有人），請換一個 handle"));
    }
    Ok(())
}

/// Extracts mention tokens (`@` plus the run of handle characters after it), lower-cased,
/// de-duplicated, in order of first appearance. An `@` preceded by an ASCII handle character
/// (e.g. an e-mail address) is not a mention. A trailing `.` is treated as punctuation.
/// In text without spaces the token can run into the words after the name (`@helper幫忙`);
/// [`resolve_mention`] finds the handle inside it.
pub fn parse_mentions(body: &str) -> Vec<String> {
    let chars: Vec<char> = body.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (i == 0 || !(chars[i - 1].is_ascii() && is_handle_char(chars[i - 1]))) {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && is_handle_char(chars[end]) {
                end += 1;
            }
            let mut h: String = chars[start..end].iter().collect();
            while h.ends_with('.') {
                h.pop();
            }
            if !h.is_empty() {
                let h = h.to_lowercase();
                if !out.contains(&h) {
                    out.push(h);
                }
            }
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mentions() {
        assert_eq!(parse_mentions("@alice 你好 @Bob."), vec!["alice", "bob"]);
        assert_eq!(parse_mentions("寄信到 a@b.com"), Vec::<String>::new());
        assert_eq!(parse_mentions("@a-1,@a-1 @x_y"), vec!["a-1", "x_y"]);
        assert_eq!(parse_mentions("請@helper幫忙"), vec!["helper幫忙"]);
        assert_eq!(resolve_mention("helper幫忙", ["helper"]), Some("helper"));
        assert_eq!(parse_mentions("@小明@小華 好"), vec!["小明", "小華"]);
        assert_eq!(parse_mentions("@"), Vec::<String>::new());
    }

    #[test]
    fn validates_handles() {
        assert!(validate_handle("ab").is_ok());
        assert!(validate_handle("a").is_err());
        assert!(validate_handle("-ab").is_err());
        assert!(validate_handle("ab.").is_err());
        assert!(validate_handle("中文").is_ok());
        assert!(validate_handle("中 文").is_err());
    }
}

/// Resolves a parsed mention token against the known handles (all lower-cased).
///
/// Scripts without spaces glue the handle to the following text (`@客服助理請幫我`, `@helper幫忙`),
/// so besides an exact match the longest handle that is a prefix of the token also matches — but
/// only when the text right after it is not ASCII (`@bobby` never matches `bob`).
pub fn resolve_mention<'a>(token: &str, handles: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<&'a str> = None;
    for h in handles {
        if h.is_empty() {
            continue;
        }
        if h == token {
            return Some(h);
        }
        if let Some(rest) = token.strip_prefix(h)
            && rest.chars().next().is_some_and(|c| !c.is_ascii())
            && best.is_none_or(|b| h.len() > b.len())
        {
            best = Some(h);
        }
    }
    best
}

/// Turns a display name into a valid handle: spaces become `-`, other invalid characters are
/// dropped, at most 32 characters. Returns None if nothing usable is left.
pub fn handle_from_name(name: &str) -> Option<String> {
    let mut out = String::new();
    for c in name.trim().chars() {
        if is_handle_char(c) {
            out.push(c);
        } else if c.is_whitespace() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches(|c| c == '-' || c == '.' || c == '_').chars().take(32).collect();
    let out = out.trim_end_matches(|c| c == '-' || c == '.').to_string();
    validate_handle(&out).ok().map(|_| out)
}

#[cfg(test)]
mod unicode_tests {
    use super::*;

    #[test]
    fn unicode_handles_and_prefix_resolution() {
        assert!(validate_handle("客服助理").is_ok());
        assert!(validate_handle("客服 助理").is_err());
        assert_eq!(parse_mentions("@客服助理請幫我查"), vec!["客服助理請幫我查"]);
        let hs = ["客服", "客服助理", "bob", "helper"];
        assert_eq!(resolve_mention("客服助理請幫我查", hs), Some("客服助理"));
        assert_eq!(resolve_mention("客服助理", hs), Some("客服助理"));
        assert_eq!(resolve_mention("helper幫忙", hs), Some("helper"));
        assert_eq!(resolve_mention("bobby", hs), None);
        assert_eq!(resolve_mention("小明", hs), None);
        assert_eq!(handle_from_name("  Sales Bot (TW) "), Some("Sales-Bot-TW".into()));
        assert_eq!(handle_from_name("客服 助理"), Some("客服-助理".into()));
        assert_eq!(handle_from_name("!"), None);
    }
}
