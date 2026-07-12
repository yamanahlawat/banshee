use regex::Regex;
use std::sync::LazyLock;

// Spoken when an all-code reply leaves nothing else to say
const EMPTY_FALLBACK: &str = "I updated the code.";

static CODE_BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```.*?```").unwrap());
static INLINE_CODE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]*)`").unwrap());
static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://([A-Za-z0-9.-]+)[^\s)\]}]*").unwrap());
static MD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[*~]+").unwrap());
static SPACE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

pub fn sanitize(input: &str) -> String {
    let mut text = CODE_BLOCK_RE.replace_all(input, "").to_string();

    // Unwrap identifier-shaped inline code so the pronunciation pass can verbalize
    // it; drop anything else (commands, JSON) that would be read aloud as noise
    text = INLINE_CODE_RE
        .replace_all(&text, |caps: &regex::Captures| {
            let code = &caps[1];
            if !code.is_empty()
                && code
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
                && code.chars().any(char::is_alphanumeric)
            {
                code.to_string()
            } else {
                String::new()
            }
        })
        .to_string();

    // Speak the host as "example dot com"; raw dots would split the utterance
    // into a gap, and the path adds nothing spoken
    text = URL_RE
        .replace_all(&text, |caps: &regex::Captures| {
            let host = caps[1].trim_end_matches(['.', ',', ';', ':', '!', '?']);
            let trailing = &caps[1][host.len()..];
            format!("{}{trailing}", host.replace('.', " dot "))
        })
        .to_string();

    text = MD_RE.replace_all(&text, "").to_string();
    text = SPACE_RE.replace_all(&text, " ").to_string();

    let final_text = text.trim();
    if final_text.is_empty() {
        return EMPTY_FALLBACK.to_string();
    }

    final_text.to_string()
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn strips_non_speakable_markdown() {
        assert_eq!(
            sanitize("The result is **ready**. See `get_models_path` at https://example.com."),
            "The result is ready. See get_models_path at example dot com."
        );
    }

    #[test]
    fn uses_a_safe_fallback_when_only_code_was_supplied() {
        assert_eq!(
            sanitize("```rust\nprintln!(\"hello\");\n```"),
            "I updated the code."
        );
    }

    #[test]
    fn drops_non_identifier_inline_code() {
        // Identifiers survive; commands and punctuation salad are dropped
        assert_eq!(sanitize("Run `rm -rf /` now"), "Run now");
        assert_eq!(sanitize("Send `{\"a\":1}` please"), "Send please");
        assert_eq!(sanitize("Call `get_models_path` here"), "Call get_models_path here");
    }

    #[test]
    fn url_host_does_not_swallow_adjacent_words() {
        assert_eq!(sanitize("See (https://example.com) here"), "See (example dot com) here");
        assert_eq!(sanitize("Go to https://example.com/a/b then"), "Go to example dot com then");
    }
}
