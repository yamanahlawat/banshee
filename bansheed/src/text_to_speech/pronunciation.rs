use std::borrow::Cow;
use std::sync::LazyLock;

use misaki_rs::G2P;
use misaki_rs::lexicon::PhonemeEntry;
use num2words::Num2Words;
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
    ("w/o", "without"),
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

// Single words misaki spells out or mispronounces, in misaki's phoneme alphabet
// (from its lexicon entry for the respelling in the trailing comment). Grows
// from oov-words.log. Every entry needs a primary stress mark: misaki discards
// an unstressed entry for the all-caps form and spells the word out instead.
static DICTIONARY: &[(&str, &str)] = &[
    ("affordance", "əfˈɔɹdᵊns"),   // from accordance
    ("figma", "fˈɪɡmə"),           // from figment
    ("gradle", "ɡɹˈAdᵊl"),         // from cradle
    ("kubernetes", "kˌubənˈɛtɪs"), // koo-ber-NET-iss
    ("webhook", "wˈɛb hˈʊk"),      // web hook
    ("websocket", "wˈɛb sˈɑkət"),  // web socket
    ("earcon", "ˈɪɹ kˈɑn"),        // ear con
    ("symlink", "sˈɪm lˈɪŋk"),     // sim link
    ("yaml", "jˈæmᵊl"),            // jam-uhl
    ("toml", "tˈɑmᵊl"),            // tom-uhl
];

pub fn install_dictionary(g2p: &mut G2P) {
    for (word, phonemes) in DICTIONARY {
        let entry = PhonemeEntry::Simple((*phonemes).to_string());
        g2p.lexicon.golds.insert((*word).to_string(), entry);
    }
}

static LOWER_TO_UPPER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([a-z0-9])([A-Z])").unwrap());
// An acronym running into a new word: `XMLId` -> "XML Id". A plural acronym
// ends in that same shape, so the split itself skips the lone trailing `s`.
static ACRONYM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([A-Z])([A-Z][a-z]+)").unwrap());
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

static UNIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"((?:[0-9]{1,3}(?:,[0-9]{3})+|[0-9]+)(?:\.[0-9]+)?)( ?)(MB|GB|KB|kHz|ms|%|s)(/s)?")
        .unwrap()
});
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"v?[0-9]+(?:\.[0-9]+)+").unwrap());
static NUMBERED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\w) #([0-9])").unwrap());
static PATH_LIKE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\S*[./]\S*").unwrap());
static ABBREVIATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\W*(?:[A-Za-z]\.)+[A-Za-z]?\W*$").unwrap());
static HYPHENATED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[a-z0-9]+(?:-[a-z0-9]+)+\b").unwrap());

// Written unit, its word, and whether the word takes a plural "s"
const UNIT_WORDS: [(&str, &str, bool); 7] = [
    ("MB", "megabyte", true),
    ("GB", "gigabyte", true),
    ("KB", "kilobyte", true),
    ("kHz", "kilohertz", false),
    ("ms", "millisecond", true),
    ("s", "second", true),
    ("%", "percent", false),
];

pub(crate) const DIGITS: [&str; 10] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
];

/// Builds every rule so the first reply does not pay for compiling them.
pub fn warm() {
    LazyLock::force(&FIXUPS);
    for rule in [
        &LOWER_TO_UPPER,
        &ACRONYM,
        &WHITESPACE,
        &UNIT,
        &NUMBER,
        &NUMBERED,
        &PATH_LIKE,
        &ABBREVIATION,
        &HYPHENATED,
    ] {
        LazyLock::force(rule);
    }
}

/// get_models_path -> "get models path", parseJSONResponse -> "parse Jason Response",
/// src/main.rs -> "src slash main dot rs", 0.17.0 -> "zero point seventeen point zero".
pub fn normalize(input: &str) -> String {
    // Whole terms such as `macOS` first, before camel-case splitting breaks them
    let mut text = apply_fixups(input.to_string());
    // Before the camel-case split, which would cut `kHz` into "k Hz"
    text = spoken_forms(&text);
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

fn spoken_forms(text: &str) -> String {
    let text = units(text);
    let text = numbers(&text);
    let text = NUMBERED.replace_all(&text, "$1 number $2");
    let text = paths(&text);
    hyphenated_names(&text).into_owned()
}

fn units(text: &str) -> Cow<'_, str> {
    UNIT.replace_all(text, |caps: &Captures| {
        let whole = caps.get(0).unwrap();
        let (number, gap, unit) = (&caps[1], &caps[2], &caps[3]);
        // A glued `s` after a whole number is a plural: the 1990s
        let plural = unit == "s" && gap.is_empty() && !number.contains('.');
        if plural || glued(text, whole.start(), whole.end(), &['.', ','], &['_']) {
            return whole.as_str().to_string();
        }
        let (_, word, pluralises) = UNIT_WORDS
            .iter()
            .find(|(written, ..)| *written == unit)
            .unwrap();
        let rate = if caps.get(4).is_some() {
            " per second"
        } else {
            ""
        };
        let plural_s = if *pluralises && number != "1" {
            "s"
        } else {
            ""
        };
        format!("{number} {word}{plural_s}{rate}")
    })
}

fn numbers(text: &str) -> Cow<'_, str> {
    NUMBER.replace_all(text, |caps: &Captures| {
        let found = caps.get(0).unwrap();
        if glued(
            text,
            found.start(),
            found.end(),
            &['.', ',', '_', '$'],
            &['_'],
        ) {
            return found.as_str().to_string();
        }
        let (prefix, number) = match found.as_str().strip_prefix('v') {
            Some(rest) => ("v ", rest),
            None => ("", found.as_str()),
        };
        let parts: Vec<&str> = number.split('.').collect();
        let spoken = match parts[..] {
            [whole, fraction] => format!("{} point {}", cardinal(whole), digits(fraction)),
            _ => parts
                .iter()
                .map(|part| cardinal(part))
                .collect::<Vec<_>>()
                .join(" point "),
        };
        format!("{prefix}{spoken}")
    })
}

/// Whether `text[start..end]` touches a letter, a digit, or one of the given
/// characters on that side.
fn glued(text: &str, start: usize, end: usize, before: &[char], after: &[char]) -> bool {
    let touches = |c: char, others: &[char]| c.is_alphanumeric() || others.contains(&c);
    text[..start]
        .chars()
        .next_back()
        .is_some_and(|c| touches(c, before))
        || text[end..]
            .chars()
            .next()
            .is_some_and(|c| touches(c, after))
}

fn cardinal(number: &str) -> String {
    if number.len() > 1 && number.starts_with('0') {
        return digits(number);
    }
    number
        .parse::<i64>()
        .ok()
        .and_then(|n| Num2Words::new(n).to_words().ok())
        .map(|words| {
            words
                .split(|c: char| !c.is_ascii_alphabetic())
                .filter(|word| !word.is_empty() && *word != "and")
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| digits(number))
}

fn digits(number: &str) -> String {
    number
        .bytes()
        .map(|digit| DIGITS[usize::from(digit - b'0')])
        .collect::<Vec<_>>()
        .join(" ")
}

fn paths(text: &str) -> Cow<'_, str> {
    PATH_LIKE.replace_all(text, |caps: &Captures| {
        let found = caps.get(0).unwrap();
        let token = &text[found.range()];
        let dot_word = token.char_indices().any(|(at, c)| {
            c == '.'
                && token[at + 1..]
                    .chars()
                    .next()
                    .is_some_and(char::is_lowercase)
        });
        if (!token.contains('/') && !dot_word) || ABBREVIATION.is_match(token) {
            return Cow::Borrowed(token);
        }
        Cow::Owned(spell_separators(token))
    })
}

fn spell_separators(token: &str) -> String {
    // Digits and slashes alone are a fraction or a date
    let path = token.chars().any(char::is_alphabetic);
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let chars: Vec<char> = token.chars().collect();
    let mut out = String::with_capacity(token.len() + 8);
    for (at, c) in chars.iter().enumerate() {
        let before = at.checked_sub(1).and_then(|at| chars.get(at));
        let after = chars.get(at + 1);
        match c {
            '/' if path && word(before) && word(after) => out.push_str(" slash "),
            '.' if word(before) && after.is_some_and(|c| c.is_lowercase()) => out.push_str(" dot "),
            _ => out.push(*c),
        }
    }
    out
}

fn hyphenated_names(text: &str) -> Cow<'_, str> {
    HYPHENATED.replace_all(text, |caps: &Captures| {
        let name = &caps[0];
        if name.chars().any(|c| c.is_ascii_lowercase()) {
            name.replace('-', " ")
        } else {
            name.to_string()
        }
    })
}

fn apply_fixups(mut text: String) -> String {
    for fixup in FIXUPS.iter() {
        text = fixup.0.replace_all(&text, NoExpand(fixup.1)).into_owned();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_builds_every_rule() {
        warm();
        assert!(LazyLock::get(&FIXUPS).is_some());
        for rule in [
            &LOWER_TO_UPPER,
            &ACRONYM,
            &WHITESPACE,
            &UNIT,
            &NUMBER,
            &NUMBERED,
            &PATH_LIKE,
            &ABBREVIATION,
            &HYPHENATED,
        ] {
            assert!(LazyLock::get(rule).is_some());
        }
    }

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
    fn plain_words_stay() {
        assert_eq!(normalize("plain words stay"), "plain words stay");
    }

    #[test]
    fn a_version_reads_each_part_with_point() {
        assert_eq!(
            normalize("Upgrade to 0.17.0 today"),
            "Upgrade to zero point seventeen point zero today"
        );
        assert_eq!(
            normalize("Released v0.17.0 to the tap"),
            "Released v zero point seventeen point zero to the tap"
        );
        assert_eq!(
            normalize("1.123.0"),
            "one point one hundred twenty three point zero"
        );
    }

    #[test]
    fn a_decimal_reads_its_fraction_digit_by_digit() {
        assert_eq!(
            normalize("1.2 and 1.2.3"),
            "one point two and one point two point three"
        );
        assert_eq!(normalize("pi is 3.14"), "pi is three point one four");
    }

    #[test]
    fn a_number_glued_to_a_word_or_a_thousands_comma_stays() {
        for text in ["python3.11", "1,234.5", "about .5"] {
            assert_eq!(normalize(text), text);
        }
    }

    #[test]
    fn digits_of_another_script_stay() {
        let text = "\u{663}.\u{664}.\u{665} and \u{663} MB";
        assert_eq!(normalize(text), text);
    }

    #[test]
    fn a_full_stop_after_a_version_or_a_file_name_still_ends_the_sentence() {
        assert_eq!(
            normalize("Upgraded to 0.17.0."),
            "Upgraded to zero point seventeen point zero."
        );
        assert_eq!(
            normalize("I changed Cargo.toml."),
            "I changed Cargo dot toml."
        );
    }

    #[test]
    fn a_path_reads_slash_and_dot() {
        assert_eq!(normalize("open src/main.rs"), "open src slash main dot rs");
        assert_eq!(
            normalize("see bansheed/src/text_to_speech/pronunciation.rs now"),
            "see bansheed slash src slash text to speech slash pronunciation dot rs now"
        );
    }

    #[test]
    fn without_reads_as_a_word() {
        assert_eq!(
            normalize("shipped w/o a doc, W/o tests."),
            "shipped without a doc, without tests."
        );
    }

    #[test]
    fn and_or_read_write_and_tcp_ip_keep_slash() {
        assert_eq!(
            normalize("tests and/or docs, read/write over TCP/IP"),
            "tests and slash or docs, read slash write over TCP slash IP"
        );
    }

    #[test]
    fn a_fraction_a_date_and_an_abbreviation_stay() {
        for text in [
            "half is 1/2",
            "due 10/31/2026",
            "e.g. this",
            "the U.S. office",
        ] {
            assert_eq!(normalize(text), text);
        }
    }

    #[test]
    fn a_url_is_left_to_the_sanitizer() {
        let clean = crate::text_to_speech::sanitizer::sanitize(
            "see https://example.com/v1.2.3/a-b/c.rs now",
        );
        assert_eq!(normalize(&clean), "see example dot com now");
    }

    #[test]
    fn a_numbered_reference_reads_number() {
        assert_eq!(normalize("see PR #218"), "see PR number 218");
    }

    #[test]
    fn a_unit_after_a_number_reads_as_a_word() {
        assert_eq!(
            normalize("731 MB in 4.5 s"),
            "731 megabytes in four point five seconds"
        );
        assert_eq!(
            normalize("1 GB, 2 KB, 900ms, 24 kHz, 50% and 1 s"),
            "1 gigabyte, 2 kilobytes, 900 milliseconds, 24 kilohertz, 50 percent and 1 second"
        );
    }

    #[test]
    fn a_unit_per_second_reads_per_second() {
        assert_eq!(
            normalize("2 MB/s, 1.5 GB/s and 300KB/s"),
            "2 megabytes per second, one point five gigabytes per second and 300 kilobytes per second"
        );
    }

    #[test]
    fn a_price_is_left_to_the_voice() {
        assert_eq!(normalize("it costs $1.50"), "it costs $1.50");
    }

    #[test]
    fn an_s_glued_to_a_whole_number_stays_a_plural() {
        for text in ["the 90s", "since the 1990s"] {
            assert_eq!(normalize(text), text);
        }
        assert_eq!(normalize("took 4.5s"), "took four point five seconds");
    }

    #[test]
    fn a_hyphenated_lowercase_name_reads_as_words() {
        assert_eq!(
            normalize("calls banshee-mcp-shim on feat/pocket-voice"),
            "calls banshee mcp shim on feat slash pocket voice"
        );
    }

    #[test]
    fn a_hyphenated_date_or_capitalised_name_stays() {
        for text in ["shipped 2026-10-30", "on Wi-Fi"] {
            assert_eq!(normalize(text), text);
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

    // Only the lowercase form is installed; the lexicon answers for the capitalized one.
    #[test]
    fn dictionary_survives_a_sentence_start() {
        use misaki_rs::{G2P, Language};
        let mut g2p = G2P::new(Language::EnglishUS);
        super::install_dictionary(&mut g2p);
        for (word, expected) in super::DICTIONARY {
            let sentence = format!("{} is fine.", crate::audio::cues::capitalised(word));
            let (phonemes, _) = g2p.g2p(&sentence).unwrap();
            assert!(
                phonemes.contains(expected),
                "{word} lost at sentence start in {sentence:?}: got {phonemes}"
            );
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
