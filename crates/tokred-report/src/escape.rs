//! Escaping of untrusted text for HTML bodies, attributes and the embedded JSON block.

/// Escapes text for use in HTML element content or in a double- or single-quoted attribute.
#[must_use]
pub fn html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Makes a JSON document safe to embed in a `<script>` element.
///
/// `<`, `>` and `&` become `<`, `>` and `&` (so neither `</script>`
/// nor `<!--` can appear), and the line separators U+2028/U+2029 are escaped too.
/// The result is still the same JSON value.
#[must_use]
pub fn json_for_script(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for ch in json.chars() {
        match ch {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escapes_every_special_character() {
        assert_eq!(
            html(r#"<script>alert("x")&'y'</script>"#),
            "&lt;script&gt;alert(&quot;x&quot;)&amp;&#39;y&#39;&lt;/script&gt;"
        );
    }

    #[test]
    fn html_leaves_plain_text_alone() {
        assert_eq!(html("claude-opus-4 ünï"), "claude-opus-4 ünï");
    }

    #[test]
    fn json_for_script_blocks_script_terminators() {
        let escaped = json_for_script(r#"{"a":"</script><!-- &  "}"#);
        assert!(!escaped.contains('<'));
        assert!(!escaped.contains('>'));
        assert!(!escaped.contains('&'));
        assert!(!escaped.contains("</"));
    }

    #[test]
    fn json_for_script_round_trips_to_the_same_value() {
        let original = serde_json::json!({
            "name": "</script><!-- a & b > c \u{2028}\u{2029}",
        });
        let escaped = json_for_script(&original.to_string());
        let parsed: serde_json::Value = serde_json::from_str(&escaped).unwrap();
        assert_eq!(parsed, original);
    }
}
