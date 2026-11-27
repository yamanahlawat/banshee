//! Fixes the words to know in finished text. The rule follows Handy's word
//! correction (cjpais/Handy, MIT).

use strsim::levenshtein;

/// A score below this is a match. The edit distance over the longer key gives
/// the score, and a matching Soundex code multiplies it by `SOUNDEX_WEIGHT`.
pub const THRESHOLD: f64 = 0.10;

const SOUNDEX_WEIGHT: f64 = 0.5;
const LONGEST_RUN: usize = 3;
const LONGEST_KEY: usize = 50;

struct Target {
    word: String,
    key: String,
    soundex: Option<[u8; 4]>,
}

/// The words to know, prepared for matching. Only ASCII words in the list
/// take part.
pub struct Targets(Vec<Target>);

impl Targets {
    pub fn new(words: &[String]) -> Self {
        Self(
            words
                .iter()
                .filter_map(|word| {
                    let key = match_key(word).filter(|key| !key.is_empty())?;
                    Some(Target {
                        word: word.clone(),
                        soundex: soundex(&key),
                        key,
                    })
                })
                .collect(),
        )
    }

    /// Replaces words the model misheard with the closest listed word. A run
    /// of up to three words can become one, so "charge be" matches "ChargeBee".
    pub fn correct(&self, text: &str) -> String {
        if self.0.is_empty() {
            return text.to_string();
        }
        let tokens: Vec<&str> = text.split_whitespace().collect();
        let mut out = Vec::with_capacity(tokens.len());
        let mut at = 0;
        while at < tokens.len() {
            match best_run(&tokens[at..], &self.0) {
                Some((len, target)) => {
                    let (prefix, _) = edges(tokens[at]);
                    let (_, suffix) = edges(tokens[at + len - 1]);
                    out.push(format!(
                        "{prefix}{}{suffix}",
                        match_case(tokens[at], &target.word)
                    ));
                    at += len;
                }
                None => {
                    out.push(tokens[at].to_string());
                    at += 1;
                }
            }
        }
        out.join(" ")
    }
}

/// The run at the front of `tokens` with the lowest score. A punctuation mark
/// inside a run ends it, so "charge, be quick" never reaches past the comma.
fn best_run<'a>(tokens: &[&str], targets: &'a [Target]) -> Option<(usize, &'a Target)> {
    let mut best: Option<(usize, &Target, f64)> = None;
    for len in (1..=LONGEST_RUN.min(tokens.len())).rev() {
        let run = &tokens[..len];
        if run[..len - 1]
            .iter()
            .any(|token| !edges(token).1.is_empty())
        {
            continue;
        }
        let Some(key) = run
            .iter()
            .map(|token| match_key(token))
            .collect::<Option<String>>()
        else {
            continue;
        };
        if let Some((target, score)) = closest(&key, targets)
            && best.is_none_or(|(_, _, lowest)| score < lowest)
        {
            best = Some((len, target, score));
        }
    }
    best.map(|(len, target, _)| (len, target))
}

fn closest<'a>(key: &str, targets: &'a [Target]) -> Option<(&'a Target, f64)> {
    if key.is_empty() || key.len() > LONGEST_KEY {
        return None;
    }
    let sound = soundex(key);
    targets
        .iter()
        .filter_map(|target| {
            let longer = key.len().max(target.key.len()) as f64;
            if key.len().abs_diff(target.key.len()) as f64 > (longer * 0.25).max(2.0) {
                return None;
            }
            let mut score = levenshtein(key, &target.key) as f64 / longer;
            if sound.is_some() && sound == target.soundex {
                score *= SOUNDEX_WEIGHT;
            }
            (score < THRESHOLD).then_some((target, score))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The lowercase letters and digits of a word, or `None` when any of them is
/// not ASCII: Soundex and the length rule are English-only.
fn match_key(word: &str) -> Option<String> {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.is_ascii().then_some(c.to_ascii_lowercase()))
        .collect()
}

/// Gives the replacement the capitalisation of the word it replaces.
fn match_case(original: &str, replacement: &str) -> String {
    if original.chars().all(char::is_uppercase) {
        replacement.to_uppercase()
    } else if original.chars().next().is_some_and(char::is_uppercase) {
        crate::audio::cues::capitalised(replacement)
    } else {
        replacement.to_string()
    }
}

/// The punctuation before the first letter or digit and after the last one.
fn edges(token: &str) -> (&str, &str) {
    let start = token
        .find(|c: char| c.is_alphanumeric())
        .unwrap_or(token.len());
    let end = token
        .rfind(|c: char| c.is_alphanumeric())
        .map_or(token.len(), |at| {
            at + token[at..].chars().next().map_or(0, char::len_utf8)
        });
    (&token[..start], &token[end.max(start)..])
}

/// The Soundex code of an all-letter key. A key with a digit has none, so
/// numbers are compared by edit distance alone.
fn soundex(key: &str) -> Option<[u8; 4]> {
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    let digit = |b: u8| match b {
        b'b' | b'f' | b'p' | b'v' => b'1',
        b'c' | b'g' | b'j' | b'k' | b'q' | b's' | b'x' | b'z' => b'2',
        b'd' | b't' => b'3',
        b'l' => b'4',
        b'm' | b'n' => b'5',
        b'r' => b'6',
        _ => b'0',
    };
    let bytes = key.as_bytes();
    let mut code = [b'0'; 4];
    code[0] = bytes[0].to_ascii_uppercase();
    let mut filled = 1;
    let mut last = digit(bytes[0]);
    for &b in &bytes[1..] {
        if filled == 4 {
            break;
        }
        let d = digit(b);
        if d != b'0' && d != last {
            code[filled] = d;
            filled += 1;
        }
        // h and w do not separate two letters of one code
        if b != b'h' && b != b'w' {
            last = d;
        }
    }
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(words: &[&str]) -> Vec<String> {
        words.iter().map(ToString::to_string).collect()
    }

    fn correct(text: &str, words: &[String]) -> String {
        Targets::new(words).correct(text)
    }

    #[test]
    fn a_misheard_name_is_corrected() {
        assert_eq!(
            correct("deploy it on kubernetis today", &list(&["Kubernetes"])),
            "deploy it on Kubernetes today"
        );
    }

    #[test]
    fn ordinary_words_near_a_listed_word_stay() {
        assert_eq!(
            correct("a bunch of files on the bench", &list(&["banshee"])),
            "a bunch of files on the bench"
        );
        assert_eq!(
            correct("cloud computing", &list(&["Claude"])),
            "cloud computing"
        );
    }

    /// Parakeet's own transcript of Pocket reading "The page is written in Svelte."
    #[test]
    fn a_word_parakeet_misheard_is_corrected() {
        assert_eq!(
            correct("The page is written in Svelt,", &list(&["Svelte"])),
            "The page is written in Svelte,"
        );
    }

    #[test]
    fn a_split_word_is_joined() {
        assert_eq!(
            correct("charge be is down", &list(&["ChargeBee"])),
            "ChargeBee is down"
        );
    }

    #[test]
    fn a_common_phrase_stays() {
        assert_eq!(
            correct("but she said no", &list(&["banshee"])),
            "but she said no"
        );
    }

    #[test]
    fn punctuation_around_a_match_stays() {
        assert_eq!(
            correct("parakeat, listen.", &list(&["Parakeet"])),
            "Parakeet, listen."
        );
    }

    #[test]
    fn a_comma_ends_a_run() {
        assert_eq!(
            correct("charge, be quick", &list(&["ChargeBee"])),
            "charge, be quick"
        );
    }

    #[test]
    fn a_non_ascii_token_is_never_corrected() {
        assert_eq!(
            correct("der über wagen", &list(&["Uber"])),
            "der über wagen"
        );
        assert_eq!(correct("a café", &list(&["caf"])), "a café");
    }

    #[test]
    fn a_non_ascii_listed_word_is_skipped() {
        assert_eq!(correct("a caf here", &list(&["café"])), "a caf here");
    }

    #[test]
    fn the_replacement_takes_the_case_of_the_original() {
        let lower = list(&["parakeet"]);
        assert_eq!(correct("Parakeat is here", &lower), "Parakeet is here");
        assert_eq!(correct("PARAKEAT is here", &lower), "PARAKEET is here");
        assert_eq!(correct("parakeat is here", &lower), "parakeet is here");
        assert_eq!(
            correct("parakeat is here", &list(&["Parakeet"])),
            "Parakeet is here"
        );
    }

    #[test]
    fn no_words_returns_the_text() {
        assert_eq!(correct("ask bancy", &[]), "ask bancy");
    }

    #[test]
    fn non_ascii_words_are_skipped() {
        assert_eq!(correct("ask bancy", &list(&["班シー"])), "ask bancy");
    }

    #[test]
    fn soundex_codes_follow_the_standard() {
        assert_eq!(soundex("robert"), soundex("rupert"));
        assert_eq!(soundex("robert"), Some(*b"R163"));
        assert_eq!(soundex("ashcraft"), Some(*b"A261"));
        assert_eq!(soundex("tymczak"), Some(*b"T522"));
        assert_eq!(soundex("pfister"), Some(*b"P236"));
        assert_eq!(soundex("a1"), None);
    }
}
