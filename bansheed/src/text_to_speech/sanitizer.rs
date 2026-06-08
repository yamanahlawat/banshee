use regex::Regex;
use std::sync::OnceLock;

// Global static locks for regex
static CODE_BLOCK_RE: OnceLock<Option<Regex>> = OnceLock::new();
static INLINE_CODE_RE: OnceLock<Option<Regex>> = OnceLock::new();
static URL_RE: OnceLock<Option<Regex>> = OnceLock::new();
static MD_RE: OnceLock<Option<Regex>> = OnceLock::new();
static SPACE_RE: OnceLock<Option<Regex>> = OnceLock::new();

pub fn sanitize(input: &str) -> String {
    let mut text = input.to_string();

    // Initialize the regex, if it fails, we don't strip it

    if let Some(re) = CODE_BLOCK_RE
        .get_or_init(|| Regex::new(r"(?s)```.*?```").ok())
        .as_ref()
    {
        text = re.replace_all(&text, "").to_string();
    }

    if let Some(re) = INLINE_CODE_RE
        .get_or_init(|| Regex::new(r"`.*?`").ok())
        .as_ref()
    {
        text = re.replace_all(&text, "").to_string();
    }

    if let Some(re) = URL_RE
        .get_or_init(|| Regex::new(r"https?://[^\s]+").ok())
        .as_ref()
    {
        text = re.replace_all(&text, "").to_string();
    }

    if let Some(re) = MD_RE.get_or_init(|| Regex::new(r"[*_~]+").ok()).as_ref() {
        text = re.replace_all(&text, "").to_string();
    }

    if let Some(re) = SPACE_RE.get_or_init(|| Regex::new(r"\s+").ok()).as_ref() {
        text = re.replace_all(&text, " ").to_string();
    }

    let final_text = text.trim();

    // Fallback if the AI returns the code and it all gets stripped out
    if final_text.is_empty() {
        return "I updated the code.".to_string();
    }

    final_text.to_string()
}
