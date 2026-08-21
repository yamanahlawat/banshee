use std::sync::LazyLock;

use misaki_rs::G2P;
use misaki_rs::lexicon::PhonemeEntry;
use regex::{Captures, NoExpand, Regex};

// Terms the voice mispronounces, respelled into words it says right. Regex-based
// and case-insensitive, so it also catches embedded forms (parseJSONResponse).
const FIXUP_PAIRS: &[(&str, &str)] = &[
    ("macOS", "Mac O S"),
    ("iOS", "I O S"),
    ("JSON-RPC", "Jason RPC"),
    ("JSON", "Jason"),
    ("fastapi", "fast API"),
    ("lockfile", "lock file"),
    ("frontend", "front end"),
    ("github", "git hub"),
    ("gitlab", "git lab"),
    ("bitbucket", "bit bucket"),
    ("nginx", "engine x"),
    ("kubectl", "cube control"),
    ("postgres", "post gres"),
    ("mutex", "mew tex"),
    ("config", "con fig"),
    ("OAuth", "oh auth"),
    ("javascript", "java script"),
    ("nodejs", "node J S"),
    ("localhost", "local host"),
    ("hostname", "host name"),
    ("namespace", "name space"),
    ("changelog", "change log"),
    ("dotfiles", "dot files"),
    ("fullstack", "full stack"),
    ("devops", "dev ops"),
    ("tarball", "tar ball"),
    ("webpack", "web pack"),
    ("neovim", "neo vim"),
    ("protobuf", "proto buff"),
    ("mongodb", "mongo D B"),
    ("dockerfile", "docker file"),
    ("gitignore", "git ignore"),
    ("golang", "go lang"),
    ("stdin", "standard in"),
    ("stdout", "standard out"),
    ("stderr", "standard err"),
    ("enum", "ee num"),
    ("axum", "axe um"),
    ("sqlite", "sequel light"),
    ("mysql", "my sequel"),
    ("graphql", "graph Q L"),
    ("redis", "red dis"),
    ("eslint", "E S lint"),
    ("sudo", "sue dough"),
    ("tokio", "tokyo"),
    ("serde", "sir day"),
    ("clippy", "clip pea"),
    ("deno", "dee know"),
    ("onnx", "onyx"),
    ("tensorflow", "tensor flow"),
    ("pytorch", "pi torch"),
    ("opencv", "open C V"),
];

static FIXUPS: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    FIXUP_PAIRS
        .iter()
        .map(|(written, spoken)| {
            (
                Regex::new(&format!(r"(?i)\b{}\b", regex::escape(written))).unwrap(),
                *spoken,
            )
        })
        .collect()
});

// Single words misaki spells out or mispronounces, given fixed IPA (from
// misaki's own g2p of the respelling in the trailing comment). Grows from
// oov-words.log. Every entry needs a primary stress mark: misaki discards an
// unstressed entry for the all-caps form and spells the word out instead.
static DICTIONARY: &[(&str, &str)] = &[
    ("affordance", "ɐfˈɔːɹdəns"),   // from accordance
    ("figma", "fˈɪɡmə"),            // from figment
    ("gradle", "ɡɹˈeɪdəl"),         // from cradle
    ("kubernetes", "kˌuːbəˈnɛtɪs"), // koo-ber-NET-iss
    ("webhook", "wˈɛb hˈʊk"),       // web hook
    ("websocket", "wˈɛb sˈɑːkɪt"),  // web socket
    ("earcon", "ˈɪɹ kˈɑːn"),        // ear con
    ("symlink", "sˈɪm lˈɪŋk"),      // sim link
    ("yaml", "jˈæməl"),             // jam-uhl
    ("toml", "tˈɑːməl"),            // tom-uhl
];

// Merge our dictionary into misaki's gold lexicon, before any synthesis.
pub fn install_dictionary(g2p: &mut G2P) {
    for (word, phonemes) in DICTIONARY {
        let entry = PhonemeEntry::Simple((*phonemes).to_string());
        g2p.lexicon.golds.insert((*word).to_string(), entry.clone());
        // Capitalized form too, so lookup hits at sentence start.
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            let capitalized = first.to_uppercase().collect::<String>() + chars.as_str();
            g2p.lexicon.golds.insert(capitalized, entry);
        }
    }
}

static LOWER_TO_UPPER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([a-z0-9])([A-Z])").unwrap());
// An acronym running into a new word: `XMLId` -> "XML Id". A plural acronym
// ends in that same shape, so the split itself skips the lone trailing `s`.
static ACRONYM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([A-Z])([A-Z][a-z]+)").unwrap());
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// Turns identifier punctuation and casing into speakable words and fixes terms
/// the voice mispronounces: `get_models_path` -> "get models path",
/// `parseJSONResponse` -> "parse Jason Response".
pub fn normalize(input: &str) -> String {
    // Whole terms such as `macOS` first, before camel-case splitting breaks them
    let mut text = apply_fixups(input.to_string());
    text = text.replace('_', " ");
    text = LOWER_TO_UPPER.replace_all(&text, "$1 $2").into_owned();
    text = ACRONYM
        .replace_all(&text, |caps: &Captures| {
            // `APIs` is one word, `XMLId` is two, and both look alike here
            if &caps[2][1..] == "s" {
                caps[0].to_string()
            } else {
                format!("{} {}", &caps[1], &caps[2])
            }
        })
        .into_owned();
    // Second pass catches terms exposed by the split
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

    // Split fragments get letter-spelled by misaki.
    #[test]
    fn keeps_plural_acronyms_intact() {
        for word in ["IDs", "APIs", "URLs", "PRs", "IDEs", "SDKs", "CLIs"] {
            assert_eq!(normalize(word), word);
        }
        assert_eq!(
            normalize("the request IDs are stale"),
            "the request IDs are stale"
        );
    }

    #[test]
    fn splits_an_acronym_from_a_short_following_word() {
        assert_eq!(normalize("convertJSONToXML"), "convert Jason To XML");
        assert_eq!(normalize("parseXMLId"), "parse XML Id");
        assert_eq!(normalize("loadCSVIn"), "load CSV In");
    }

    #[test]
    #[ignore = "audit helper: cargo test audit_fixup_phonemes -- --ignored --nocapture"]
    fn audit_fixup_phonemes() {
        use misaki_rs::{G2P, Language};
        let g2p = G2P::new(Language::EnglishUS);
        for (word, spoken) in super::FIXUP_PAIRS {
            let via = g2p.g2p(spoken).map(|(p, _)| p).unwrap_or_default();
            println!("{word:<12} \"{spoken}\" -> [{via}]");
        }
    }

    #[test]
    #[ignore = "helper: edit the pair, then cargo test emit_dictionary_entry -- --ignored --nocapture"]
    fn emit_dictionary_entry() {
        use misaki_rs::{G2P, Language};
        // Prints a ready-to-paste DICTIONARY row from misaki's g2p of a respelling.
        // Edit this pair for the word you're adding:
        let (word, respelling) = ("webhook", "web hook");
        let g2p = G2P::new(Language::EnglishUS);
        let p = g2p.g2p(respelling).map(|(p, _)| p).unwrap_or_default();
        println!("    (\"{word}\", \"{}\"), // {respelling}", p.trim());
    }

    #[test]
    #[ignore = "audit helper: cargo test audit_all_caps_phonemes -- --ignored --nocapture"]
    fn audit_all_caps_phonemes() {
        use misaki_rs::{G2P, Language};
        // Every term as a user would write it shouting, through the real pipeline.
        // Uppercase A I O Q S T W Y in the output are kokoro diphthongs, not
        // unmapped letters.
        let mut g2p = G2P::new(Language::EnglishUS);
        super::install_dictionary(&mut g2p);
        let words = super::FIXUP_PAIRS
            .iter()
            .map(|(w, _)| *w)
            .chain(super::DICTIONARY.iter().map(|(w, _)| *w));
        for word in words {
            let normalized = normalize(&format!("the {} file", word.to_uppercase()));
            let (phonemes, _) = g2p.g2p(&normalized).unwrap();
            println!("{word:<12} -> {normalized:<28} [{phonemes}]");
        }
    }

    #[test]
    fn dictionary_overrides_are_installed() {
        use misaki_rs::{G2P, Language};
        let mut g2p = G2P::new(Language::EnglishUS);
        super::install_dictionary(&mut g2p);
        // Live iff g2p echoes our exact string; also catches an out-of-vocab symbol.
        for (word, expected) in super::DICTIONARY {
            let (phonemes, _) = g2p.g2p(word).unwrap();
            assert_eq!(phonemes.trim(), *expected, "{word} override not installed");
        }
    }

    // The all-caps form is tagged NNP, and misaki drops an unstressed entry for
    // it in favour of spelling the word out.
    #[test]
    fn dictionary_survives_the_all_caps_form() {
        use misaki_rs::{G2P, Language};
        let mut g2p = G2P::new(Language::EnglishUS);
        super::install_dictionary(&mut g2p);
        for (word, expected) in super::DICTIONARY {
            assert!(expected.contains('ˈ'), "{word} needs a primary stress mark");
            let sentence = format!("the {} file", word.to_uppercase());
            let (phonemes, _) = g2p.g2p(&sentence).unwrap();
            assert!(
                phonemes.contains(expected),
                "{word} spelled out in {sentence:?}: got {phonemes}"
            );
        }
    }
}
