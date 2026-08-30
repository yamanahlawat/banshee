use banshee_common::Voice;

/// Display name and one distinguishing word per Kokoro voice. The id is the
/// value `config.toml` carries.
const NAMED: &[(&str, &str, &str)] = &[
    ("af_sky", "Sky", "American, clear"),
    ("af_bella", "Bella", "American, warm"),
    ("af_heart", "Heart", "American, soft"),
    ("af_nicole", "Nicole", "American, hushed"),
    ("af_sarah", "Sarah", "American, even"),
    ("am_adam", "Adam", "American, low"),
    ("am_michael", "Michael", "American, steady"),
    ("am_santa", "Santa", "American, deep"),
    ("bf_emma", "Emma", "British, bright"),
    ("bf_isabella", "Isabella", "British, warm"),
    ("bm_george", "George", "British, steady"),
    ("bm_lewis", "Lewis", "British, low"),
];

/// Every voice this build can name, on this machine or not. A client that can
/// fetch one offers the whole list; setup itself fetches only the voice set.
pub fn catalogue() -> impl Iterator<Item = &'static str> {
    NAMED.iter().map(|(id, _, _)| *id)
}

pub fn describe(id: &str, downloaded: bool) -> Voice {
    if let Some((_, name, description)) = NAMED.iter().find(|(known, _, _)| *known == id) {
        return Voice {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            downloaded,
        };
    }
    let accent = match id.chars().next() {
        Some('a') => "American",
        Some('b') => "British",
        _ => "Unknown accent",
    };
    let gender = match id.chars().nth(1) {
        Some('f') => "female",
        Some('m') => "male",
        _ => "voice",
    };
    Voice {
        id: id.to_string(),
        name: id.to_string(),
        description: format!("{accent}, {gender}"),
        downloaded,
    }
}

#[cfg(test)]
mod tests {
    use super::describe;

    #[test]
    fn a_known_voice_gets_a_name_and_one_word() {
        let voice = describe("af_sky", true);
        assert_eq!(voice.name, "Sky");
        assert_eq!(voice.description, "American, clear");
        assert_eq!(voice.id, "af_sky");
    }

    #[test]
    fn an_unknown_voice_falls_back_to_its_id_and_its_accent() {
        let voice = describe("bf_lily", false);
        assert_eq!(voice.name, "bf_lily");
        assert_eq!(voice.description, "British, female");
    }

    #[test]
    fn every_installed_kokoro_voice_has_a_description() {
        for id in [
            "af_sky",
            "af_heart",
            "am_adam",
            "am_santa",
            "bf_emma",
            "bm_george",
        ] {
            assert_ne!(describe(id, true).name, id, "{id} needs a display name");
        }
    }
}
