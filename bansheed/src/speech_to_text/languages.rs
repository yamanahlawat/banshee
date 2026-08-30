use banshee_common::Language;

/// Every language this Whisper build holds, in its own order, which puts
/// English first and the rest by how much training data each had. A window
/// that carried its own copy would drift from the engine that has to accept it.
pub fn all() -> Vec<Language> {
    (0..=whisper_rs::get_lang_max_id())
        .filter_map(|id| {
            Some(Language {
                code: whisper_rs::get_lang_str(id)?.to_string(),
                name: titled(whisper_rs::get_lang_str_full(id)?),
            })
        })
        .collect()
}

/// Whisper spells its names in lower case, and the window sets them beside
/// device and voice names that are not.
fn titled(name: &str) -> String {
    name.split(' ')
        .map(|word| {
            let mut letters = word.chars();
            match letters.next() {
                Some(first) => first.to_uppercase().collect::<String>() + letters.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{all, titled};

    /// The engine accepts what it lists, so every code here must parse back.
    #[test]
    fn every_language_listed_is_one_whisper_accepts() {
        let languages = all();
        assert!(languages.len() > 50, "got {}", languages.len());
        for language in &languages {
            assert!(
                whisper_rs::get_lang_id(&language.code).is_some(),
                "{} is not a code whisper knows",
                language.code
            );
        }
    }

    /// The config's default has to be in the list a person picks from.
    #[test]
    fn english_is_listed_and_leads() {
        let languages = all();
        assert_eq!(languages[0].code, "en");
        assert_eq!(languages[0].name, "English");
    }

    #[test]
    fn a_two_word_name_capitalises_both() {
        assert_eq!(titled("haitian creole"), "Haitian Creole");
    }
}
