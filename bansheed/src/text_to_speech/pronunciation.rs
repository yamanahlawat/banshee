use std::sync::LazyLock;

use regex::{NoExpand, Regex};

// Terms the neural voice mispronounces (spells "JSON", says "makos"). All are
// alphanumeric-bounded, so a plain \bWORD\b match is enough.
static FIXUPS: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    [
        ("macOS", "Mac O S"),
        ("iOS", "I O S"),
        ("JSON-RPC", "Jason RPC"),
        ("JSON", "Jason"),
    ]
    .into_iter()
    .map(|(written, spoken)| {
        (
            Regex::new(&format!(r"(?i)\b{}\b", regex::escape(written))).unwrap(),
            spoken,
        )
    })
    .collect()
});

static LOWER_TO_UPPER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([a-z0-9])([A-Z])").unwrap());
static ACRONYM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([A-Z])([A-Z][a-z])").unwrap());
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// Turns identifier punctuation and casing into speakable words and fixes terms
/// the voice mispronounces: `get_models_path` -> "get models path",
/// `parseJSONResponse` -> "parse Jason Response".
pub fn normalize(input: &str) -> String {
    // Whole terms such as `macOS` first, before camel-case splitting breaks them
    let mut text = apply_fixups(input.to_string());
    text = text.replace('_', " ");
    text = LOWER_TO_UPPER.replace_all(&text, "$1 $2").into_owned();
    text = ACRONYM.replace_all(&text, "$1 $2").into_owned();
    // Second pass catches terms exposed by the split, e.g. `parseJSONResponse`
    text = apply_fixups(text);
    WHITESPACE.replace_all(&text, " ").trim().to_string()
}

fn apply_fixups(mut text: String) -> String {
    for fixup in FIXUPS.iter() {
        text = fixup.0.replace_all(&text, NoExpand(fixup.1)).into_owned();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn splits_identifiers_into_words() {
        assert_eq!(
            normalize("Call get_models_path here"),
            "Call get models path here"
        );
    }

    #[test]
    fn fixes_mispronounced_terms_including_embedded() {
        assert_eq!(
            normalize("macOS sends JSON via parseJSONResponse"),
            "Mac O S sends Jason via parse Jason Response"
        );
    }
}
