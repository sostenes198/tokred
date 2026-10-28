//! Pure text rules for memory files: token estimate, comment stripping and `@path` imports.

use std::path::{Component, Path, PathBuf};

/// Estimated tokens of `text`: characters / 4, rounded up. Always an estimate.
pub(crate) fn estimate_tokens(text: &str) -> u64 {
    u64::try_from(text.chars().count().div_ceil(4)).unwrap_or(u64::MAX)
}

/// Opening code fence of a line: the fence character and its length.
fn opening_fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let ch = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = trimmed.chars().take_while(|c| *c == ch).count();
    (len >= 3).then_some((ch, len))
}

/// Whether `line` closes a fence opened with `ch` repeated `len` times.
fn closes_fence(line: &str, ch: char, len: usize) -> bool {
    let trimmed = line.trim();
    trimmed.chars().count() >= len && trimmed.chars().all(|c| c == ch)
}

/// Removes block-level HTML comments (`<!-- ... -->` starting a line), which Claude Code strips
/// before injecting a memory file. Comments inside fenced code blocks are kept.
pub(crate) fn strip_block_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<(char, usize)> = None;
    let mut in_comment = false;
    for line in text.lines() {
        if in_comment {
            in_comment = !line.contains("-->");
            continue;
        }
        if let Some((ch, len)) = fence {
            if closes_fence(line, ch, len) {
                fence = None;
            }
        } else if let Some(open) = opening_fence(line) {
            fence = Some(open);
        } else if line.trim_start().starts_with("<!--") {
            in_comment = !line.contains("-->");
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !text.ends_with('\n') {
        out.pop();
    }
    out
}

/// Replaces inline code spans with spaces so their contents are never read as imports.
fn blank_code_spans(line: &str) -> Vec<char> {
    let mut chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            i += 1;
            continue;
        }
        let run = chars[i..].iter().take_while(|c| **c == '`').count();
        let mut j = i + run;
        let mut close = None;
        while j < chars.len() {
            if chars[j] == '`' {
                let other = chars[j..].iter().take_while(|c| **c == '`').count();
                if other == run {
                    close = Some(j + other);
                    break;
                }
                j += other;
            } else {
                j += 1;
            }
        }
        match close {
            Some(end) => {
                chars[i..end].fill(' ');
                i = end;
            }
            None => i += run,
        }
    }
    chars
}

/// Imports of one line: `@path` at the start of a token, backslash-escaped spaces allowed.
fn line_imports(line: &str, out: &mut Vec<String>) {
    let chars = blank_code_spans(line);
    let mut i = 0;
    while i < chars.len() {
        let at_token_start = i == 0 || chars[i - 1].is_whitespace();
        if chars[i] != '@' || !at_token_start {
            i += 1;
            continue;
        }
        i += 1;
        let mut spec = String::new();
        while i < chars.len() && !chars[i].is_whitespace() {
            if chars[i] == '\\' && chars.get(i + 1) == Some(&' ') {
                spec.push(' ');
                i += 2;
            } else {
                spec.push(chars[i]);
                i += 1;
            }
        }
        let quoted = spec.starts_with(['"', '\'']);
        if !spec.is_empty() && !quoted {
            out.push(spec);
        }
    }
}

/// Finds the `@path` imports of a memory file, skipping fenced code blocks and code spans.
pub(crate) fn find_imports(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in text.lines() {
        if let Some((ch, len)) = fence {
            if closes_fence(line, ch, len) {
                fence = None;
            }
        } else if let Some(open) = opening_fence(line) {
            fence = Some(open);
        } else {
            line_imports(line, &mut out);
        }
    }
    out
}

/// Whether an import looks like a file path rather than a prose mention such as `@param`.
pub(crate) fn looks_like_path(spec: &str) -> bool {
    spec.starts_with('~') || spec.contains(['/', '\\', '.'])
}

/// The spellings tried for an import: as written, then without trailing sentence punctuation.
pub(crate) fn candidate_specs(spec: &str) -> Vec<String> {
    let mut specs = vec![spec.to_owned()];
    let trimmed = spec.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}']);
    if !trimmed.is_empty() && trimmed != spec {
        specs.push(trimmed.to_owned());
    }
    specs
}

/// Lexically normalizes a path (`.` and `..` components); no file system access.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(component);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Resolves an import against the importing file's directory (`base`) or the home directory.
pub(crate) fn resolve_import(spec: &str, base: &Path, home: Option<&Path>) -> Option<PathBuf> {
    if spec == "~" {
        return home.map(Path::to_path_buf);
    }
    if let Some(rest) = spec.strip_prefix("~/") {
        return home.map(|h| h.join(rest));
    }
    let path = Path::new(spec);
    Some(if path.has_root() {
        path.to_path_buf()
    } else {
        base.join(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_characters_over_four_rounded_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("ééééé"), 2);
    }

    #[test]
    fn finds_imports_at_token_starts_only() {
        let text =
            "see @docs/a.md and @./b.md.\nmail me at a@b.com\n@c.md\n- x @Design\\ Docs/d.md";
        assert_eq!(
            find_imports(text),
            ["docs/a.md", "./b.md.", "c.md", "Design Docs/d.md"]
        );
    }

    #[test]
    fn ignores_quoted_and_empty_imports() {
        assert!(find_imports("@\"a b.md\" @'c.md' @ alone").is_empty());
    }

    #[test]
    fn skips_code_fences_and_code_spans() {
        let text = "```\n@in-fence.md\n```\n~~~sh\n@tilde.md\n~~~\n`@span.md` and ``@double ` span.md`` @real.md\n@after.md";
        assert_eq!(find_imports(text), ["real.md", "after.md"]);
    }

    #[test]
    fn longer_fence_is_not_closed_by_a_shorter_one() {
        let text = "````\n```\n@inside.md\n````\n@outside.md";
        assert_eq!(find_imports(text), ["outside.md"]);
    }

    #[test]
    fn unclosed_code_span_stays_literal() {
        assert_eq!(find_imports("`unclosed @a.md"), ["a.md"]);
        assert_eq!(find_imports("`` x ` @a.md"), ["a.md"]);
    }

    #[test]
    fn strips_block_comments_but_keeps_code() {
        let text =
            "keep\n<!-- one line -->\n<!-- multi\nline\n-->\nafter\n```\n<!-- in code -->\n```\n";
        assert_eq!(
            strip_block_comments(text),
            "keep\nafter\n```\n<!-- in code -->\n```\n"
        );
    }

    #[test]
    fn imports_inside_comments_are_not_followed() {
        let stripped = strip_block_comments("<!-- @hidden.md -->\n@shown.md\n");
        assert_eq!(find_imports(&stripped), ["shown.md"]);
    }

    #[test]
    fn path_heuristic_and_candidates() {
        assert!(looks_like_path("docs/a"));
        assert!(looks_like_path("a.md"));
        assert!(looks_like_path("~"));
        assert!(!looks_like_path("param"));
        assert_eq!(candidate_specs("a.md"), ["a.md"]);
        assert_eq!(candidate_specs("a.md)."), ["a.md).", "a.md"]);
        assert_eq!(candidate_specs("..."), ["..."]);
    }

    #[test]
    fn normalizes_dots_lexically() {
        assert_eq!(normalize(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normalize(Path::new("../x")), PathBuf::from("../x"));
        assert_eq!(normalize(Path::new("./a")), PathBuf::from("a"));
    }

    #[test]
    fn resolves_relative_home_and_absolute_imports() {
        let base = Path::new("/p/sub");
        let home = Path::new("/home/u");
        assert_eq!(
            resolve_import("a.md", base, Some(home)),
            Some(PathBuf::from("/p/sub/a.md"))
        );
        assert_eq!(
            resolve_import("~/x/a.md", base, Some(home)),
            Some(PathBuf::from("/home/u/x/a.md"))
        );
        assert_eq!(resolve_import("~", base, Some(home)), Some(home.into()));
        assert_eq!(resolve_import("~", base, None), None);
        assert_eq!(resolve_import("~/a", base, None), None);
        assert_eq!(
            resolve_import("/abs/a.md", base, None),
            Some(PathBuf::from("/abs/a.md"))
        );
    }
}
