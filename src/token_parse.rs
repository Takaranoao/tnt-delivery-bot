use regex::Regex;
use std::sync::LazyLock;

static RE_QUERY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[?&]token=([A-Za-z0-9]+)").unwrap());
static RE_HASH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#/([A-Za-z0-9]+)").unwrap());
static RE_BARE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9]{4,32}$").unwrap());

/// Extract a T&T token from a user message. Order: `?token=` / `&token=`,
/// then `#/<token>`, then a whole trimmed bare token.
pub fn parse_token(text: &str) -> Option<String> {
    if let Some(c) = RE_QUERY.captures(text) {
        return Some(c[1].to_string());
    }
    if let Some(c) = RE_HASH.captures(text) {
        return Some(c[1].to_string());
    }
    let t = text.trim();
    if RE_BARE.is_match(t) {
        return Some(t.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_token() {
        assert_eq!(parse_token("3abc128856").as_deref(), Some("3abc128856"));
        assert_eq!(parse_token("  3abc128856 \n").as_deref(), Some("3abc128856"));
    }

    #[test]
    fn api_url() {
        assert_eq!(
            parse_token("https://tmsapi.tntsupermarket.us/track/customer?token=3abc128856")
                .as_deref(),
            Some("3abc128856")
        );
    }

    #[test]
    fn tracking_hash_url() {
        assert_eq!(
            parse_token("https://tmstracking.tntsupermarket.us/#/3abc128856").as_deref(),
            Some("3abc128856")
        );
    }

    #[test]
    fn full_sentence_with_url() {
        let msg = "Your T&T order 000039752 is on the way. https://tmstracking.tntsupermarket.us/#/3abc128856 [Do Not Reply]";
        assert_eq!(parse_token(msg).as_deref(), Some("3abc128856"));
    }

    #[test]
    fn sentence_without_url_is_not_token() {
        assert_eq!(parse_token("Your T&T order 000039752 is on the way."), None);
    }

    #[test]
    fn junk_is_none() {
        assert_eq!(parse_token("hello there!"), None);
        assert_eq!(parse_token(""), None);
    }
}
