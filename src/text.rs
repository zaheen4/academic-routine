//! HTML escaping for Telegram messages rendered with `parse_mode = HTML`.

/// Escapes the three characters Telegram's HTML parser treats as markup.
///
/// `&` is replaced first; doing it last would re-escape the ampersands that
/// `&lt;` and `&gt;` introduce.
#[must_use]
pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_html() {
        assert_eq!(escape_html("Hello & World"), "Hello &amp; World");
        assert_eq!(
            escape_html("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
        assert_eq!(escape_html("Room 612 (Lab)"), "Room 612 (Lab)");
        assert_eq!(escape_html("CSE-4101"), "CSE-4101");
        assert_eq!(escape_html("&lt;"), "&amp;lt;");
    }
}
