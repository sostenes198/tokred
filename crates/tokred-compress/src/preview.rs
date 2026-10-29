//! The deterministic replacement of a large text tool result: the first and last lines
//! of the original plus a one-line marker carrying the id that `tokred_retrieve` reads back.
//!
//! Everything here is pure: the same text and rules always give the same bytes (no clock, no
//! randomness), which is what keeps the prompt cache and preserved thinking consistent when the
//! replacement is applied again on later turns.

use tokred_core::config::CompressionConfig;

/// Version of the replacement rules, part of [`PreviewRules::rules_version`].
pub const RULES_VERSION: &str = "r1";

/// Hex characters of the content hash used as the id given to the model (64 bits).
pub const ID_LEN: usize = 16;

/// Hex blake3 of a text.
#[must_use]
pub fn content_hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

/// The id given to the model for an original whose hash is `hash`: its first [`ID_LEN`] hex
/// characters. The same original therefore always gets the same id, in every session.
#[must_use]
pub fn retrieve_id(hash: &str) -> String {
    hash.chars().take(ID_LEN).collect()
}

/// Estimated tokens of a text: bytes / 4 (the estimate used everywhere).
#[must_use]
pub fn estimated_tokens(bytes: usize) -> u64 {
    u64::try_from(bytes / 4).unwrap_or(u64::MAX)
}

/// The lines of a text, each keeping its terminator (`\n` or `\r\n`), so concatenating them gives
/// the text back byte for byte. A final line without a terminator is still a line; a trailing
/// terminator does not start an extra empty line.
#[must_use]
pub fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// The one-line marker that follows the preview.
#[must_use]
pub fn marker(omitted: usize, id: &str) -> String {
    format!(
        "[tokred: {omitted} lines omitted (id={id}). Call tokred_retrieve with this id and an \
         optional offset/limit to read them.]"
    )
}

/// What a replacement keeps and when it applies (from `[compression]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewRules {
    /// Only texts above this many estimated tokens are replaced.
    pub min_tokens: u64,
    /// Lines kept from the start.
    pub head_lines: usize,
    /// Lines kept from the end.
    pub tail_lines: usize,
}

impl PreviewRules {
    /// Reads the rules from `[compression]`.
    #[must_use]
    pub fn from_config(config: &CompressionConfig) -> Self {
        Self {
            min_tokens: config.min_tokens,
            head_lines: usize::try_from(config.preview_head_lines).unwrap_or(usize::MAX),
            tail_lines: usize::try_from(config.preview_tail_lines).unwrap_or(usize::MAX),
        }
    }

    /// The version recorded with every replacement and shadow record, e.g. `r1-h40-t20`: the
    /// replacement bytes depend on the preview sizes, so they are part of it.
    #[must_use]
    pub fn rules_version(&self) -> String {
        format!("{RULES_VERSION}-h{}-t{}", self.head_lines, self.tail_lines)
    }
}

impl Default for PreviewRules {
    fn default() -> Self {
        Self::from_config(&CompressionConfig::default())
    }
}

/// A computed replacement for one original text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    /// blake3 (hex) of the original.
    pub hash: String,
    /// The id in the marker (see [`retrieve_id`]).
    pub id: String,
    /// The text sent instead of the original.
    pub text: String,
    /// Lines in the original.
    pub lines: usize,
    /// Lines left out of the preview.
    pub omitted: usize,
    /// Bytes of the original.
    pub original_bytes: usize,
}

impl Replacement {
    /// Bytes not sent because of the replacement.
    #[must_use]
    pub fn bytes_saved(&self) -> usize {
        self.original_bytes.saturating_sub(self.text.len())
    }
}

/// The replacement of `text`, or `None` when it must be sent as it is: not above
/// `min_tokens`, no line would be left out (for example one long line of minified JSON), or the
/// preview would not be shorter than the original.
///
/// The replacement is the first `head_lines` lines, the [`marker`], a line break and the last
/// `tail_lines` lines, with the original bytes of those lines untouched.
#[must_use]
pub fn replacement(text: &str, rules: &PreviewRules) -> Option<Replacement> {
    if estimated_tokens(text.len()) <= rules.min_tokens {
        return None;
    }
    let all = lines(text);
    let kept = rules.head_lines.saturating_add(rules.tail_lines);
    if all.len() <= kept {
        return None;
    }
    let omitted = all.len() - kept;
    let hash = content_hash(text);
    let id = retrieve_id(&hash);
    let mut out = all[..rules.head_lines].concat();
    out.push_str(&marker(omitted, &id));
    if rules.tail_lines > 0 {
        out.push('\n');
        out.push_str(&all[all.len() - rules.tail_lines..].concat());
    }
    (out.len() < text.len()).then_some(Replacement {
        hash,
        id,
        text: out,
        lines: all.len(),
        omitted,
        original_bytes: text.len(),
    })
}

/// A page of a stored original, as returned by `tokred_retrieve`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// 0-based index of the first line returned.
    pub first: usize,
    /// Lines returned.
    pub count: usize,
    /// Lines in the original.
    pub total: usize,
    /// The returned lines, with their original terminators.
    pub text: String,
}

impl Page {
    /// Lines after this page.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.total - self.first - self.count
    }
}

/// Lines `offset..offset + limit` of `content`, stopping early so the text stays within
/// `max_bytes` (but always at least one line, so paging always moves forward). `None` when
/// `offset` is at or past the end.
#[must_use]
pub fn page(content: &str, offset: usize, limit: usize, max_bytes: usize) -> Option<Page> {
    let all = lines(content);
    if offset >= all.len() {
        return None;
    }
    let mut text = String::new();
    let mut count = 0;
    for line in all.iter().skip(offset).take(limit.max(1)) {
        if count > 0 && text.len() + line.len() > max_bytes {
            break;
        }
        text.push_str(line);
        count += 1;
    }
    Some(Page {
        first: offset,
        count,
        total: all.len(),
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(lines: usize, width: usize) -> String {
        (1..=lines)
            .map(|n| format!("{n:0width$}\n"))
            .collect::<Vec<_>>()
            .concat()
    }

    fn rules(min_tokens: u64, head: usize, tail: usize) -> PreviewRules {
        PreviewRules {
            min_tokens,
            head_lines: head,
            tail_lines: tail,
        }
    }

    #[test]
    fn hashes_and_ids_are_stable() {
        let hash = content_hash("abc");
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, content_hash("abc"));
        assert_eq!(retrieve_id(&hash), hash[..ID_LEN]);
        assert_eq!(estimated_tokens(4001), 1000);
    }

    #[test]
    fn lines_keep_their_terminators() {
        assert_eq!(lines("a\r\nb\nc"), ["a\r\n", "b\n", "c"]);
        assert_eq!(lines("a\n"), ["a\n"]);
        assert!(lines("").is_empty());
    }

    #[test]
    fn the_default_rules_come_from_the_config() {
        let rules = PreviewRules::default();
        assert_eq!(rules, self::rules(1500, 40, 20));
        assert_eq!(rules.rules_version(), "r1-h40-t20");
    }

    #[test]
    fn a_large_text_becomes_head_marker_and_tail() {
        let text = numbered(100, 99);
        let r = replacement(&text, &rules(10, 2, 3)).unwrap();
        let id = retrieve_id(&content_hash(&text));
        let line = |n: usize| format!("{n:099}\n");
        let expected = [
            line(1),
            line(2),
            marker(95, &id),
            "\n".to_owned(),
            line(98),
            line(99),
            line(100),
        ]
        .concat();
        assert_eq!(r.text, expected);
        assert_eq!((r.lines, r.omitted, r.original_bytes), (100, 95, 10_000));
        assert_eq!(r.id, id);
        assert_eq!(r.bytes_saved(), 10_000 - expected.len());
        assert!(r.text.contains(&format!(
            "[tokred: 95 lines omitted (id={id}). Call tokred_retrieve with this id and an \
             optional offset/limit to read them.]"
        )));
    }

    #[test]
    fn the_same_input_gives_the_same_bytes() {
        let text = numbered(500, 20);
        let first = replacement(&text, &PreviewRules::default());
        assert!(first.is_some());
        for _ in 0..3 {
            assert_eq!(replacement(&text, &PreviewRules::default()), first);
        }
    }

    #[test]
    fn edges_of_the_preview() {
        // No tail: the marker ends the text.
        let text = numbered(50, 99);
        let r = replacement(&text, &rules(1, 1, 0)).unwrap();
        assert!(r.text.ends_with("read them.]"));
        // No head: the marker starts it, and an unterminated last line stays unterminated.
        let text = format!("{}tail without newline", numbered(50, 99));
        let r = replacement(&text, &rules(1, 0, 1)).unwrap();
        assert!(r.text.starts_with("[tokred: 50 lines omitted"));
        assert!(r.text.ends_with("]\ntail without newline"));
    }

    #[test]
    fn texts_that_stay_as_they_are() {
        // At the threshold (bytes / 4 == min_tokens) it is not above it.
        assert_eq!(replacement(&"x\n".repeat(2000), &rules(1000, 1, 1)), None);
        // One long line (minified JSON) has nothing to omit.
        assert_eq!(replacement(&"{}".repeat(10_000), &rules(10, 40, 20)), None);
        // As many lines as the preview keeps.
        assert_eq!(replacement(&numbered(60, 999), &rules(10, 40, 20)), None);
        // Short lines and a long marker: the preview would not be shorter.
        assert_eq!(replacement(&"x\n".repeat(64), &rules(1, 30, 30)), None);
    }

    #[test]
    fn pages_follow_offset_limit_and_the_byte_budget() {
        let text = numbered(10, 3); // ten lines of four bytes
        let p = page(&text, 2, 3, 1000).unwrap();
        assert_eq!(p.text, "003\n004\n005\n");
        assert_eq!((p.first, p.count, p.total, p.remaining()), (2, 3, 10, 5));

        let p = page(&text, 8, 100, 1000).unwrap();
        assert_eq!((p.count, p.remaining()), (2, 0));

        // The budget stops early, at a line boundary.
        let p = page(&text, 0, 100, 10).unwrap();
        assert_eq!((p.count, p.text.as_str()), (2, "001\n002\n"));
        // But one line always comes back, even over the budget, and a zero limit means one.
        let p = page(&text, 0, 0, 1).unwrap();
        assert_eq!(p.count, 1);

        assert_eq!(page(&text, 10, 1, 1000), None);
        assert_eq!(page("", 0, 1, 1000), None);
    }
}
