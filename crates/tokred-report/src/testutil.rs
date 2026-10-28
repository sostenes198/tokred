//! Test-only helpers shared by the unit tests.

/// HTML void elements: they have no closing tag.
const VOID: [&str; 6] = ["meta", "br", "link", "hr", "img", "input"];

/// Checks that tags are balanced: every start tag is closed in order, `/>` and void
/// elements need no closing tag, and `<script>`/`<style>` contents are skipped.
///
/// A minimal structural check, not a full HTML parser.
pub(crate) fn assert_well_formed(html: &str) -> Result<(), String> {
    let mut stack: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let end = after.find('>').ok_or("unterminated tag")?;
        let tag = &after[..end];
        rest = &after[end + 1..];
        if tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let open = stack.pop().ok_or_else(|| format!("unexpected </{name}>"))?;
            if open != name.trim() {
                return Err(format!("expected </{open}>, found </{name}>"));
            }
            continue;
        }
        let name: String = tag
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '/')
            .collect();
        if name.is_empty() {
            return Err("empty tag name".to_owned());
        }
        if tag.ends_with('/') || VOID.contains(&name.as_str()) {
            continue;
        }
        if name == "script" || name == "style" {
            let closing = format!("</{name}>");
            let at = rest.find(&closing).ok_or("unterminated raw text element")?;
            rest = &rest[at + closing.len()..];
            continue;
        }
        stack.push(name);
    }
    if stack.is_empty() {
        Ok(())
    } else {
        Err(format!("unclosed tags: {stack:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_balanced_markup() {
        let html = "<!doctype html><html><head><meta charset=\"utf-8\"><style>a>b{}</style>\
                    <script>if (1 < 2) {}</script></head><body><br><p>x</p><svg><g/></svg></body></html>";
        assert_eq!(assert_well_formed(html), Ok(()));
    }

    #[test]
    fn rejects_broken_markup() {
        assert!(assert_well_formed("<p>").unwrap_err().contains("unclosed"));
        assert!(
            assert_well_formed("</p>")
                .unwrap_err()
                .contains("unexpected")
        );
        assert!(
            assert_well_formed("<a><b></a>")
                .unwrap_err()
                .contains("expected")
        );
        assert!(
            assert_well_formed("<a")
                .unwrap_err()
                .contains("unterminated tag")
        );
        assert!(assert_well_formed("<>").unwrap_err().contains("empty"));
        assert!(
            assert_well_formed("<script>")
                .unwrap_err()
                .contains("raw text")
        );
    }
}
