//! Minimal YAML frontmatter reader for skills, agents and rules.
//!
//! Only top-level `key: value` pairs are read (plus indented continuation lines and block
//! scalars), which is all the audit needs: `name`, `description`, `when_to_use` and `paths`.

use std::collections::BTreeMap;

/// Whether a file has frontmatter and whether it is well formed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// The first line is not `---`: Claude Code reads no frontmatter.
    Absent,
    /// Opening and closing `---` found.
    Valid,
    /// Opening `---` without a closing one.
    Unterminated,
}

/// Parsed frontmatter.
#[derive(Debug)]
pub(crate) struct Frontmatter {
    pub state: State,
    pub fields: BTreeMap<String, String>,
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

fn scalar(value: &str) -> &str {
    match value.trim() {
        ">" | "|" | ">-" | "|-" | ">+" | "|+" => "",
        other => unquote(other),
    }
}

/// Parses the frontmatter of `text`.
pub(crate) fn parse(text: &str) -> Frontmatter {
    let mut lines = text.lines();
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    if lines.next().map(str::trim_end) != Some("---") {
        return Frontmatter {
            state: State::Absent,
            fields,
        };
    }
    let mut current: Option<String> = None;
    for line in lines {
        if line.trim_end() == "---" {
            return Frontmatter {
                state: State::Valid,
                fields,
            };
        }
        if line.starts_with(char::is_whitespace) {
            if let Some(value) = current.as_ref().and_then(|key| fields.get_mut(key)) {
                if !value.is_empty() {
                    value.push(' ');
                }
                value.push_str(line.trim());
            }
        } else if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_owned();
            fields
                .entry(key.clone())
                .or_insert_with(|| scalar(value).to_owned());
            current = Some(key);
        }
    }
    Frontmatter {
        state: State::Unterminated,
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter_when_first_line_is_not_a_fence() {
        let fm = parse("# title\n---\nname: x\n---\n");
        assert_eq!(fm.state, State::Absent);
        assert!(fm.fields.is_empty());
        assert_eq!(parse("").state, State::Absent);
    }

    #[test]
    fn reads_keys_quotes_and_crlf() {
        let fm = parse("---\r\nname: \"my-skill\"\r\ndescription: 'Does things'\r\n---\r\nbody");
        assert_eq!(fm.state, State::Valid);
        assert_eq!(fm.fields["name"], "my-skill");
        assert_eq!(fm.fields["description"], "Does things");
    }

    #[test]
    fn folds_block_scalars_and_continuations() {
        let fm =
            parse("---\ndescription: >\n  first line\n  second line\nwhen_to_use: a\n  b\n---\n");
        assert_eq!(fm.fields["description"], "first line second line");
        assert_eq!(fm.fields["when_to_use"], "a b");
    }

    #[test]
    fn first_duplicate_key_wins_and_stray_lines_are_ignored() {
        let fm = parse("---\nname: a\nname: b\nnot a pair\n---\n");
        assert_eq!(fm.fields["name"], "a");
    }

    #[test]
    fn continuation_before_any_key_is_ignored() {
        let fm = parse("---\n  orphan\nname: a\n---\n");
        assert_eq!(fm.fields.len(), 1);
    }

    #[test]
    fn unterminated_frontmatter_is_flagged() {
        let fm = parse("---\nname: a\nbody without closing fence");
        assert_eq!(fm.state, State::Unterminated);
        assert_eq!(fm.fields["name"], "a");
    }
}
